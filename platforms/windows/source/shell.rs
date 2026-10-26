use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::controls::{self, Controls, ShellEvent};
use anyhow::{Context, Result, bail};
use base64::Engine;
use p12_keystore::{Certificate, KeyStore, KeyStoreEntry, PrivateKey, PrivateKeyChain};
use rustls_pki_types::{CertificateDer, pem::PemObject};
use serde::Deserialize;
use tao::event::{Event as TaoEvent, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder, EventLoopProxy};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::platform::windows::IconExtWindows;
use tao::window::{Icon, Window, WindowBuilder};
use tokamak::{
    Certificates, Config, DevProxyConfig, DevelopmentConfig, Event, PackageLayout, Runtime,
    frontend_url,
};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_PERMISSION_KIND, COREWEBVIEW2_PERMISSION_KIND_CAMERA,
    COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION, COREWEBVIEW2_PERMISSION_KIND_MICROPHONE,
    COREWEBVIEW2_PERMISSION_KIND_UNKNOWN_PERMISSION, COREWEBVIEW2_PERMISSION_STATE_ALLOW,
    COREWEBVIEW2_PERMISSION_STATE_DENY, COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_ALWAYS_ALLOW,
    COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_CANCEL, ICoreWebView2, ICoreWebView2_5,
    ICoreWebView2_14, ICoreWebView2ClientCertificate,
    ICoreWebView2ClientCertificateRequestedEventArgs, ICoreWebView2Environment5,
    ICoreWebView2PermissionRequestedEventArgs,
    ICoreWebView2ServerCertificateErrorDetectedEventArgs,
};
use webview2_com::take_pwstr;
use webview2_com::{
    BrowserProcessExitedEventHandler, ClientCertificateRequestedEventHandler,
    PermissionRequestedEventHandler, ServerCertificateErrorDetectedEventHandler,
};
use windows::Win32::Security::Cryptography::{
    CERT_CONTEXT, CERT_STORE_ADD_REPLACE_EXISTING_INHERIT_PROPERTIES, CRYPT_INTEGER_BLOB,
    CertAddCertificateContextToStore, CertCloseStore, CertDeleteCertificateFromStore,
    CertEnumCertificatesInStore, CertOpenSystemStoreW, HCERTSTORE, PFXImportCertStore,
    PKCS12_PREFER_CNG_KSP,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    IDYES, MB_ICONQUESTION, MB_YESNO, MessageBoxW, SW_SHOWNORMAL,
};
use windows::core::{HSTRING, Interface, PCWSTR, PWSTR, w};
use wry::{
    NewWindowResponse, WebContext, WebView, WebViewBuilder, WebViewBuilderExtWindows,
    WebViewExtWindows,
};

/// How long the Worker has for `resume` and `suspend`.
const LIFECYCLE_DEADLINE: Duration = Duration::from_secs(10);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShellConfig {
    name: String,
    slug: String,
    host: String,
    #[serde(rename = "devEndpoint")]
    dev_endpoint: Option<String>,
    #[serde(rename = "devSessionToken")]
    dev_session_token: Option<String>,
}

pub(crate) fn run() -> Result<()> {
    let mut event_loop = EventLoopBuilder::<ShellEvent>::with_user_event().build();
    let events = event_loop.create_proxy();
    let root = executable_dir()?;
    let config = read_config(&root)?;
    let state = state_dir(&config.slug)?;
    let runtime = Arc::new(start_runtime(&config, &root, &state, events.clone())?);
    let identity = ClientIdentity::install(&runtime.certificates(), &config.host)?;
    let window = build_window(&event_loop, &root, &config.name)?;
    let mut app = AppWindow::new(&window, &config, &state, &runtime, identity, events)?;

    let (lifecycle, deliveries) = deliver_lifecycle_events(Arc::clone(&runtime))?;
    let set_foreground = |foreground| runtime.set_foreground(foreground);
    let window = &window;
    let controls_runtime = Arc::clone(&runtime);
    // The closure, which owns the page, drops as `run_return` returns.
    event_loop.run_return(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            TaoEvent::UserEvent(ShellEvent::Call {
                id,
                plugin,
                method,
                payload,
            }) => app.call(&controls_runtime, id, &plugin, &method, &payload),
            TaoEvent::UserEvent(ShellEvent::Unsupported(id)) => controls::respond(
                &controls_runtime,
                id,
                Err((
                    "NotSupportedError",
                    "Subscriptions are not supported".into(),
                )),
            ),
            TaoEvent::UserEvent(ShellEvent::Runtime(Event::CertificatesRenewed)) => {
                if let Err(error) = app.renew_identity() {
                    eprintln!("tokamak client certificate renewal failed: {error:#}");
                }
            }
            TaoEvent::UserEvent(ShellEvent::Runtime(Event::Listening { port })) => {
                if let Err(error) = app.gateway_moved(port) {
                    eprintln!("tokamak could not follow the gateway to port {port}: {error:#}");
                    *flow = ControlFlow::Exit;
                }
            }
            TaoEvent::UserEvent(ShellEvent::BrowserExited) => {
                if let Err(error) = app.browser_exited() {
                    eprintln!("tokamak could not reopen the app: {error:#}");
                    *flow = ControlFlow::Exit;
                }
            }
            // Windows resizes the window as it is minimised and restored.
            TaoEvent::WindowEvent {
                event: WindowEvent::Resized(_),
                ..
            } => {
                let foreground = !window.is_minimized();
                if set_foreground(foreground) {
                    let _ = lifecycle.send(foreground);
                }
            }
            TaoEvent::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *flow = ControlFlow::Exit,
            _ => {}
        }
    });
    let _ = deliveries.join();
    drop(runtime);
    Ok(())
}

/// The app's `WebView`, with the handlers installed on it.
struct Page {
    // The handlers are removed before the WebView closes.
    _handlers: Handlers,
    webview: WebView,
}

/// The app's window, whose page follows the gateway to each port it listens
/// on. A `WebView2` browser keeps the proxy it started with, so the page closes
/// and opens on the new port once its browser process has exited.
struct AppWindow<'a> {
    window: &'a Window,
    context: WebContext,
    config: &'a ShellConfig,
    certificates: Arc<Certificates>,
    /// The client certificate the page presents, which `identity` installs.
    client_certificate: Arc<RwLock<Vec<u8>>>,
    identity: ClientIdentity,
    events: EventLoopProxy<ShellEvent>,
    /// The port the gateway listens on.
    port: u16,
    page: Option<Page>,
    /// Waits for the browser process of the closed page to exit.
    exit: Option<BrowserExit>,
}

impl<'a> AppWindow<'a> {
    /// The app's `window`, with the page open on the gateway's port.
    fn new(
        window: &'a Window,
        config: &'a ShellConfig,
        state: &Path,
        runtime: &Runtime,
        identity: ClientIdentity,
        events: EventLoopProxy<ShellEvent>,
    ) -> Result<Self> {
        let mut app = Self {
            window,
            context: WebContext::new(Some(state.join("webview"))),
            config,
            certificates: runtime.certificates(),
            client_certificate: Arc::new(RwLock::new(identity.der.clone())),
            identity,
            events,
            port: runtime.port(),
            page: None,
            exit: None,
        };
        app.open()?;
        Ok(app)
    }

    /// Opens the page on the gateway's port.
    fn open(&mut self) -> Result<()> {
        let host = &self.config.host;
        let webview = build_webview(self.window, &mut self.context, host, self.port)?;
        let handlers = Handlers::install(
            &webview,
            Arc::clone(&self.certificates),
            host,
            &self.config.name,
            Arc::clone(&self.client_certificate),
        )?;
        webview
            .load_url(&frontend_url(host))
            .context("load app URL")?;
        self.page = Some(Page {
            _handlers: handlers,
            webview,
        });
        Ok(())
    }

    /// Performs the Worker's call `id` on the `WebView`, or refuses it while
    /// the page is closed.
    fn call(&self, runtime: &Arc<Runtime>, id: u64, plugin: &str, method: &str, payload: &[u8]) {
        match &self.page {
            Some(page) => controls::call(&page.webview, runtime, id, plugin, method, payload),
            None => controls::respond(
                runtime,
                id,
                Err(("InvalidStateError", "The WebView is restarting".into())),
            ),
        }
    }

    /// Records that the gateway listens on `port`, closing the page once the
    /// gateway has moved.
    fn gateway_moved(&mut self, port: u16) -> Result<()> {
        if port == self.port {
            return Ok(());
        }
        self.port = port;
        if let Some(page) = self.page.take() {
            self.exit = Some(BrowserExit::watch(&page.webview, self.events.clone())?);
        }
        Ok(())
    }

    /// Installs the renewed client certificate, which the page presents from
    /// then on.
    fn renew_identity(&mut self) -> Result<()> {
        self.identity.replace(
            &self.certificates,
            &self.config.host,
            &self.client_certificate,
        )
    }

    /// Opens the page on the gateway's port once the browser that served the
    /// closed page has exited.
    fn browser_exited(&mut self) -> Result<()> {
        if self.exit.take().is_none() {
            return Ok(());
        }
        self.open()
    }
}

/// Sends `ShellEvent::BrowserExited` once the browser process behind a
/// `WebView` exits, which it does after its last `WebView` closes.
struct BrowserExit {
    environment: ICoreWebView2Environment5,
    token: i64,
}

impl BrowserExit {
    fn watch(webview: &WebView, events: EventLoopProxy<ShellEvent>) -> Result<Self> {
        let environment: ICoreWebView2Environment5 = webview.environment().cast()?;
        let mut token = 0;
        unsafe {
            environment.add_BrowserProcessExited(
                &BrowserProcessExitedEventHandler::create(Box::new(move |_, _| {
                    let _ = events.send_event(ShellEvent::BrowserExited);
                    Ok(())
                })),
                &raw mut token,
            )?;
        }
        Ok(Self { environment, token })
    }
}

impl Drop for BrowserExit {
    fn drop(&mut self) {
        unsafe {
            let _ = self.environment.remove_BrowserProcessExited(self.token);
        }
    }
}

/// The `WebView` in `window`, which keeps the app's origin and opens other
/// URLs in the default browser.
fn build_webview(
    window: &Window,
    context: &mut WebContext,
    host: &str,
    port: u16,
) -> Result<WebView> {
    let navigation_host = host.to_owned();
    let new_window_host = host.to_owned();
    WebViewBuilder::new_with_web_context(context)
        .with_additional_browser_args(browser_arguments(host, port))
        .with_navigation_handler(move |url| {
            if is_app_origin(&url, &navigation_host) {
                true
            } else {
                open_external(&url);
                false
            }
        })
        .with_new_window_req_handler(move |url, features| {
            if is_app_origin(&url, &new_window_host) {
                let url = HSTRING::from(url);
                if let Err(error) = unsafe { features.opener.webview.Navigate(&url) } {
                    eprintln!("tokamak could not reuse its WebView for a new window: {error}");
                }
            } else {
                open_external(&url);
            }
            NewWindowResponse::Deny
        })
        .build(window)
        .context("create WebView2")
}

/// Delivers `resume` for each `true` the returned sender receives and
/// `suspend` for each `false`, one at a time, until the sender is dropped.
/// The event loop sends each change it records with `set_foreground`.
fn deliver_lifecycle_events(runtime: Arc<Runtime>) -> Result<(Sender<bool>, JoinHandle<()>)> {
    let (sender, receiver) = mpsc::channel::<bool>();
    let deliveries = thread::Builder::new()
        .name("tokamak-lifecycle".to_owned())
        .spawn(move || {
            for foreground in receiver {
                let name = if foreground { "resume" } else { "suspend" };
                if let Err(error) = runtime.emit(name, b"{}", LIFECYCLE_DEADLINE) {
                    eprintln!("tokamak {name} failed: {error}");
                }
            }
        })
        .context("start lifecycle events")?;
    Ok((sender, deliveries))
}

fn start_runtime(
    config: &ShellConfig,
    root: &Path,
    state: &Path,
    events: EventLoopProxy<ShellEvent>,
) -> Result<Runtime> {
    let plugins: Arc<dyn tokamak::PluginHandler> = Arc::new(Controls(events.clone()));
    let listener = move |event: Event| {
        report(&event);
        let _ = events.send_event(ShellEvent::Runtime(event));
    };
    match (
        config.dev_endpoint.as_deref(),
        config.dev_session_token.as_deref(),
    ) {
        (Some(endpoint), Some(session_token)) => Ok(Runtime::start_development(
            DevelopmentConfig {
                state_dir: state.join("runtime"),
                host: config.host.clone(),
                proxy: DevProxyConfig {
                    endpoint: endpoint.to_owned(),
                    session_token: session_token.to_owned(),
                },
                foreground: true,
                plugins: Some(plugins),
                server_identity: None,
            },
            listener,
        )?),
        _ => Ok(Runtime::start(
            Config {
                app: PackageLayout::new(root.join("app")),
                state_dir: state.join("runtime"),
                storage_dir: state.join("storage"),
                host: config.host.clone(),
                foreground: true,
                plugins: Some(plugins),
                server_identity: None,
            },
            listener,
        )?),
    }
}

fn executable_dir() -> Result<PathBuf> {
    env::current_exe()?
        .parent()
        .map(Path::to_path_buf)
        .context("app executable has no parent directory")
}

/// The app's window, titled `title`, with the app icon when the app has one.
fn build_window(event_loop: &EventLoop<ShellEvent>, root: &Path, title: &str) -> Result<Window> {
    WindowBuilder::new()
        .with_title(title)
        .with_window_icon(load_window_icon(root)?)
        .build(event_loop)
        .context("create app window")
}

fn load_window_icon(root: &Path) -> Result<Option<Icon>> {
    let path = root.join("AppIcon.ico");
    if !path.is_file() {
        return Ok(None);
    }
    Icon::from_path(path, None)
        .map(Some)
        .context("load Windows app icon")
}

fn read_config(root: &Path) -> Result<ShellConfig> {
    Ok(serde_json::from_slice(
        &fs::read(root.join("tokamak.json")).context("read tokamak.json")?,
    )?)
}

fn state_dir(slug: &str) -> Result<PathBuf> {
    let root = env::var_os(concat!("LOCALAPP", "DATA"))
        .context("Windows local application data directory is unavailable")?;
    let path = PathBuf::from(root).join(slug).join("tokamak");
    fs::create_dir_all(&path)?;
    Ok(path)
}

fn browser_arguments(host: &str, port: u16) -> String {
    let pac = format!(
        "function FindProxyForURL(url, host){{return host.toLowerCase()==='{host}'?'PROXY 127.0.0.1:{port}':'DIRECT';}}"
    );
    let pac = base64::engine::general_purpose::STANDARD.encode(pac);
    format!(
        "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --proxy-pac-url=data:application/x-ns-proxy-autoconfig;base64,{pac}"
    )
}

fn open_external(url: &str) {
    let url = HSTRING::from(url);
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &url,
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if result.0 as usize <= 32 {
        eprintln!("tokamak could not open external URL");
    }
}

fn report(event: &Event) {
    eprintln!("tokamak runtime: {event:?}");
}

struct ClientIdentity {
    store: HCERTSTORE,
    certificate: *const CERT_CONTEXT,
    der: Vec<u8>,
}

impl ClientIdentity {
    fn install(certificates: &Certificates, host: &str) -> Result<Self> {
        let tokamak::Decision::PresentIdentity {
            certificate,
            private_key,
        } = certificates.decide(&tokamak::Challenge::ClientCertificate {
            host,
            previous_failures: 0,
        })
        else {
            bail!("tokamak client certificate is unavailable")
        };
        let pfx = pfx(&certificate, &private_key)?;
        let imported = import_pfx(&pfx)?;
        let result = add_to_personal_store(imported, &certificate);
        unsafe {
            let _ = CertCloseStore(Some(imported), 0);
        }
        result
    }

    fn replace(
        &mut self,
        certificates: &Certificates,
        host: &str,
        current: &RwLock<Vec<u8>>,
    ) -> Result<()> {
        let replacement = Self::install(certificates, host)?;
        let mut held = current
            .write()
            .map_err(|_| anyhow::anyhow!("client certificate state is unavailable"))?;
        (*held).clone_from(&replacement.der);
        drop(held);
        drop(std::mem::replace(self, replacement));
        Ok(())
    }
}

impl Drop for ClientIdentity {
    fn drop(&mut self) {
        unsafe {
            let _ = CertDeleteCertificateFromStore(self.certificate);
            let _ = CertCloseStore(Some(self.store), 0);
        }
    }
}

fn pfx(certificate: &[u8], private_key: &[u8]) -> Result<Vec<u8>> {
    let certificate = Certificate::from_der(certificate)?;
    let private_key = PrivateKey::from_der(private_key)?;
    let chain = PrivateKeyChain::new("tokamak", private_key, [certificate]);
    let mut store = KeyStore::new();
    store.add_entry("tokamak", KeyStoreEntry::PrivateKeyChain(chain));
    Ok(store.writer("").write()?)
}

fn import_pfx(data: &[u8]) -> Result<HCERTSTORE> {
    let blob = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(data.len()).context("client identity is too large")?,
        pbData: data.as_ptr().cast_mut(),
    };
    unsafe {
        PFXImportCertStore(&raw const blob, PCWSTR::null(), PKCS12_PREFER_CNG_KSP)
            .context("import tokamak client identity")
    }
}

fn add_to_personal_store(imported: HCERTSTORE, der: &[u8]) -> Result<ClientIdentity> {
    let certificate = unsafe { CertEnumCertificatesInStore(imported, None) };
    if certificate.is_null() {
        bail!("imported tokamak client identity contains no certificate")
    }
    let store = unsafe { CertOpenSystemStoreW(None, w!("MY")) }
        .context("open personal certificate store")?;
    let mut added = std::ptr::null_mut();
    let result = unsafe {
        CertAddCertificateContextToStore(
            Some(store),
            certificate,
            CERT_STORE_ADD_REPLACE_EXISTING_INHERIT_PROPERTIES,
            Some(&raw mut added),
        )
    };
    if let Err(error) = result {
        unsafe {
            let _ = CertCloseStore(Some(store), 0);
        }
        return Err(error).context("install tokamak client identity");
    }
    if added.is_null() {
        unsafe {
            let _ = CertCloseStore(Some(store), 0);
        }
        bail!("Windows did not return the installed tokamak client identity")
    }
    Ok(ClientIdentity {
        store,
        certificate: added,
        der: der.to_vec(),
    })
}

struct Handlers {
    client: ICoreWebView2_5,
    client_token: i64,
    permissions: ICoreWebView2,
    permission_token: i64,
    server: ICoreWebView2_14,
    server_token: i64,
}

impl Handlers {
    fn install(
        webview: &WebView,
        certificates: std::sync::Arc<Certificates>,
        host: &str,
        name: &str,
        client: Arc<RwLock<Vec<u8>>>,
    ) -> Result<Self> {
        let raw = webview.webview();
        let client_view: ICoreWebView2_5 = raw.cast()?;
        let permissions = raw.clone();
        let server_view: ICoreWebView2_14 = raw.cast()?;
        let mut client_token = 0;
        let mut permission_token = 0;
        let mut server_token = 0;
        unsafe {
            client_view.add_ClientCertificateRequested(
                &ClientCertificateRequestedEventHandler::create(Box::new(client_challenge(
                    host.to_owned(),
                    client,
                ))),
                &raw mut client_token,
            )?;
            permissions.add_PermissionRequested(
                &PermissionRequestedEventHandler::create(Box::new(permission_challenge(
                    host.to_owned(),
                    name.to_owned(),
                ))),
                &raw mut permission_token,
            )?;
            server_view.add_ServerCertificateErrorDetected(
                &ServerCertificateErrorDetectedEventHandler::create(Box::new(server_challenge(
                    certificates,
                    host.to_owned(),
                ))),
                &raw mut server_token,
            )?;
        }
        Ok(Self {
            client: client_view,
            client_token,
            permissions,
            permission_token,
            server: server_view,
            server_token,
        })
    }
}

impl Drop for Handlers {
    fn drop(&mut self) {
        unsafe {
            let _ = self
                .client
                .remove_ClientCertificateRequested(self.client_token);
            let _ = self
                .permissions
                .remove_PermissionRequested(self.permission_token);
            let _ = self
                .server
                .remove_ServerCertificateErrorDetected(self.server_token);
        }
    }
}

fn client_challenge(
    host: String,
    client: Arc<RwLock<Vec<u8>>>,
) -> impl FnMut(
    Option<ICoreWebView2>,
    Option<ICoreWebView2ClientCertificateRequestedEventArgs>,
) -> windows::core::Result<()> {
    move |_, args| {
        let Some(args) = args else { return Ok(()) };
        if webview_string(|value| unsafe { args.Host(value) })? != host {
            return Ok(());
        }
        let client = client
            .read()
            .map_err(|_| windows::core::Error::from_win32())?;
        let certificates = unsafe { args.MutuallyTrustedCertificates() }?;
        let mut count = 0;
        unsafe { certificates.Count(&raw mut count) }?;
        for index in 0..count {
            let candidate = unsafe { certificates.GetValueAtIndex(index)? };
            if certificate_matches(&candidate, &client)? {
                unsafe {
                    args.SetSelectedCertificate(&candidate)?;
                    args.SetHandled(true)?;
                }
                return Ok(());
            }
        }
        unsafe { args.SetCancel(true) }
    }
}

fn permission_challenge(
    host: String,
    name: String,
) -> impl FnMut(
    Option<ICoreWebView2>,
    Option<ICoreWebView2PermissionRequestedEventArgs>,
) -> windows::core::Result<()> {
    move |_, args| {
        let Some(args) = args else { return Ok(()) };
        let uri = webview_string(|value| unsafe { args.Uri(value) })?;
        let mut kind = COREWEBVIEW2_PERMISSION_KIND_UNKNOWN_PERMISSION;
        unsafe { args.PermissionKind(&raw mut kind) }?;
        let Some(prompt) = app_permission_prompt(kind, &uri, &host) else {
            return Ok(());
        };
        let message = HSTRING::from(format!("{name} wants to {}.", prompt.request));
        let caption = HSTRING::from(prompt.caption);
        let state = if unsafe { MessageBoxW(None, &message, &caption, MB_YESNO | MB_ICONQUESTION) }
            == IDYES
        {
            COREWEBVIEW2_PERMISSION_STATE_ALLOW
        } else {
            COREWEBVIEW2_PERMISSION_STATE_DENY
        };
        unsafe { args.SetState(state) }
    }
}

#[derive(Debug, PartialEq)]
struct PermissionPrompt {
    request: &'static str,
    caption: &'static str,
}

/// The shell asks the user about these requests from the app origin; others keep the default.
fn app_permission_prompt(
    kind: COREWEBVIEW2_PERMISSION_KIND,
    uri: &str,
    host: &str,
) -> Option<PermissionPrompt> {
    if !is_app_origin(uri, host) {
        return None;
    }
    let (request, caption) = match kind {
        COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION => ("access your location", "Location access"),
        COREWEBVIEW2_PERMISSION_KIND_CAMERA => ("use your camera", "Camera access"),
        COREWEBVIEW2_PERMISSION_KIND_MICROPHONE => ("use your microphone", "Microphone access"),
        _ => return None,
    };
    Some(PermissionPrompt { request, caption })
}

fn is_app_origin(uri: &str, host: &str) -> bool {
    request_authority(uri).is_some_and(|authority| {
        authority
            .strip_suffix(":443")
            .unwrap_or(authority)
            .eq_ignore_ascii_case(host)
    })
}

fn server_challenge(
    certificates: std::sync::Arc<Certificates>,
    host: String,
) -> impl FnMut(
    Option<ICoreWebView2>,
    Option<ICoreWebView2ServerCertificateErrorDetectedEventArgs>,
) -> windows::core::Result<()> {
    move |_, args| {
        let Some(args) = args else { return Ok(()) };
        let certificate = unsafe { args.ServerCertificate()? };
        let pem = webview_string(|value| unsafe { certificate.ToPemEncoding(value) })?;
        let request_uri = webview_string(|value| unsafe { args.RequestUri(value) })?;
        let app_origin = is_app_origin(&request_uri, &host);
        let action = if app_origin && certificates.trusts_server_certificate(&host, &pem) {
            COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_ALWAYS_ALLOW
        } else {
            if app_origin {
                eprintln!("tokamak refused the gateway's certificate for {request_uri}");
            }
            COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_CANCEL
        };
        unsafe { args.SetAction(action) }
    }
}

fn request_authority(url: &str) -> Option<&str> {
    let (scheme, remainder) = url.split_once("://")?;
    scheme.eq_ignore_ascii_case("https").then_some(())?;
    remainder.split(['/', '?', '#']).next()
}

fn certificate_matches(
    certificate: &ICoreWebView2ClientCertificate,
    expected: &[u8],
) -> windows::core::Result<bool> {
    let pem = webview_string(|value| unsafe { certificate.ToPemEncoding(value) })?;
    Ok(CertificateDer::from_pem_slice(pem.as_bytes()).is_ok_and(|der| der.as_ref() == expected))
}

fn webview_string(
    read: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>,
) -> windows::core::Result<String> {
    let mut value = PWSTR::null();
    read(&raw mut value)?;
    Ok(take_pwstr(value))
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS;

    use super::{
        COREWEBVIEW2_PERMISSION_KIND_CAMERA, COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION,
        COREWEBVIEW2_PERMISSION_KIND_MICROPHONE, PermissionPrompt, app_permission_prompt,
        browser_arguments, is_app_origin,
    };

    #[test]
    fn routes_only_the_app_host_through_the_local_proxy() -> anyhow::Result<()> {
        let arguments = browser_arguments("app.tokamak.local", 1234);
        let (_, pac) = arguments
            .split_once("--proxy-pac-url=data:application/x-ns-proxy-autoconfig;base64,")
            .ok_or_else(|| anyhow::anyhow!("no proxy PAC in {arguments}"))?;
        let pac = String::from_utf8(base64::engine::general_purpose::STANDARD.decode(pac)?)?;
        assert!(
            pac.contains(
                "host.toLowerCase()==='app.tokamak.local'?'PROXY 127.0.0.1:1234':'DIRECT'"
            )
        );
        Ok(())
    }

    #[test]
    fn recognizes_only_the_exact_app_origin() {
        for (uri, app_origin) in [
            ("https://app.tokamak.local/path", true),
            ("HTTPS://APP.TOKAMAK.LOCAL:443/path", true),
            ("https://app.tokamak.local.evil/path", false),
            ("https://app.tokamak.local:444/path", false),
            ("http://app.tokamak.local/path", false),
        ] {
            assert_eq!(is_app_origin(uri, "app.tokamak.local"), app_origin, "{uri}");
        }
    }

    #[test]
    fn prompts_for_location_camera_and_microphone_from_the_app_origin() {
        let app = "https://app.tokamak.local/";
        let host = "app.tokamak.local";
        for (kind, request, caption) in [
            (
                COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION,
                "access your location",
                "Location access",
            ),
            (
                COREWEBVIEW2_PERMISSION_KIND_CAMERA,
                "use your camera",
                "Camera access",
            ),
            (
                COREWEBVIEW2_PERMISSION_KIND_MICROPHONE,
                "use your microphone",
                "Microphone access",
            ),
        ] {
            assert_eq!(
                app_permission_prompt(kind, app, host),
                Some(PermissionPrompt { request, caption })
            );
        }
    }

    #[test]
    fn leaves_other_origins_and_permissions_to_webview2() {
        let host = "app.tokamak.local";
        for uri in [
            "https://other.tokamak.local/",
            "https://app.tokamak.local:444/",
        ] {
            assert_eq!(
                app_permission_prompt(COREWEBVIEW2_PERMISSION_KIND_CAMERA, uri, host),
                None
            );
        }
        assert_eq!(
            app_permission_prompt(
                COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS,
                "https://app.tokamak.local/",
                host
            ),
            None
        );
    }
}
