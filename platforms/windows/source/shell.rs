use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result, bail};
use base64::Engine;
use openssl::pkcs12::Pkcs12;
use openssl::pkey::PKey;
use openssl::x509::X509;
use serde::Deserialize;
use tao::event::{Event as TaoEvent, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::platform::windows::IconExtWindows;
use tao::window::{Icon, WindowBuilder};
use tokamak::{
    Certificates, Config, DevProxyConfig, DevelopmentConfig, Event, PackageLayout, Runtime,
    app_host, frontend_url,
};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_PERMISSION_KIND, COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION,
    COREWEBVIEW2_PERMISSION_KIND_UNKNOWN_PERMISSION, COREWEBVIEW2_PERMISSION_STATE_ALLOW,
    COREWEBVIEW2_PERMISSION_STATE_DENY, COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_ALWAYS_ALLOW,
    COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_CANCEL, ICoreWebView2, ICoreWebView2_5,
    ICoreWebView2_14, ICoreWebView2Certificate, ICoreWebView2ClientCertificate,
    ICoreWebView2ClientCertificateRequestedEventArgs, ICoreWebView2PermissionRequestedEventArgs,
    ICoreWebView2ServerCertificateErrorDetectedEventArgs,
};
use webview2_com::take_pwstr;
use webview2_com::{
    ClientCertificateRequestedEventHandler, PermissionRequestedEventHandler,
    ServerCertificateErrorDetectedEventHandler,
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
    NewWindowResponse, WebContext, WebViewBuilder, WebViewBuilderExtWindows, WebViewExtWindows,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShellConfig {
    name: String,
    host: String,
    #[serde(rename = "devEndpoint")]
    dev_endpoint: Option<String>,
    #[serde(rename = "devSessionToken")]
    dev_session_token: Option<String>,
}

pub(crate) fn run() -> Result<()> {
    let mut event_loop = EventLoopBuilder::<Event>::with_user_event().build();
    let events = event_loop.create_proxy();
    let root = executable_dir()?;
    let config = read_config(&root)?;
    let state = state_dir(&config.name)?;
    let runtime = start_runtime(&config, &root, &state, events)?;
    let identity = ClientIdentity::install(&runtime.certificates(), &config.host)?;
    let client_certificate = Arc::new(RwLock::new(identity.certificate().to_vec()));
    let icon = load_window_icon(&root)?;
    let window = WindowBuilder::new()
        .with_title(&config.name)
        .with_window_icon(icon)
        .build(&event_loop)
        .context("create app window")?;
    let mut context = WebContext::new(Some(state.join("webview")));
    let navigation_host = config.host.clone();
    let new_window_host = config.host.clone();
    let webview = WebViewBuilder::new_with_web_context(&mut context)
        .with_additional_browser_args(browser_arguments(&config.host, runtime.port()))
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
        .build(&window)
        .context("create WebView2")?;
    let handlers = Handlers::install(
        &webview,
        runtime.certificates(),
        &config.host,
        &config.name,
        Arc::clone(&client_certificate),
    )?;
    webview
        .load_url(&frontend_url(&config.host))
        .context("load app URL")?;

    let certificates = runtime.certificates();
    let host = config.host.clone();
    let mut identity = identity;
    event_loop.run_return(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            TaoEvent::UserEvent(Event::CertificatesRenewed) => {
                if let Err(error) = identity.replace(&certificates, &host, &client_certificate) {
                    eprintln!("tokamak client certificate renewal failed: {error:#}");
                }
            }
            TaoEvent::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *flow = ControlFlow::Exit,
            _ => {}
        }
    });
    drop(handlers);
    drop(webview);
    drop(runtime);
    Ok(())
}

fn start_runtime(
    config: &ShellConfig,
    root: &Path,
    state: &Path,
    events: EventLoopProxy<Event>,
) -> Result<Runtime> {
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
            },
            move |event| {
                report(&event);
                let _ = events.send_event(event);
            },
        )?),
        (None, None) => Ok(Runtime::start(
            Config {
                app: PackageLayout::new(root.join("app")),
                state_dir: state.join("runtime"),
                host: config.host.clone(),
            },
            move |event| {
                report(&event);
                let _ = events.send_event(event);
            },
        )?),
        _ => bail!("tok dev endpoint and session token must be provided together"),
    }
}

fn executable_dir() -> Result<PathBuf> {
    env::current_exe()?
        .parent()
        .map(Path::to_path_buf)
        .context("app executable has no parent directory")
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
    let config: ShellConfig =
        serde_json::from_slice(&fs::read(root.join("tokamak.json")).context("read tokamak.json")?)?;
    let expected = app_host(&config.name).context("tokamak.json name is not a DNS label")?;
    if config.host == expected {
        Ok(config)
    } else {
        bail!("tokamak.json host does not match its app name")
    }
}

fn state_dir(name: &str) -> Result<PathBuf> {
    let root = env::var_os(concat!("LOCALAPP", "DATA"))
        .context("Windows local application data directory is unavailable")?;
    let path = PathBuf::from(root).join(name).join("tokamak");
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

    fn certificate(&self) -> &[u8] {
        &self.der
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
    let certificate = X509::from_der(certificate)?;
    let private_key = PKey::private_key_from_der(private_key)?;
    Ok(Pkcs12::builder()
        .name("tokamak")
        .pkey(&private_key)
        .cert(&certificate)
        .build2("")?
        .to_der()?)
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
        webview: &wry::WebView,
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
        if !is_app_geolocation_request(kind, &uri, &host) {
            return Ok(());
        }
        let message = HSTRING::from(format!("{name} wants to access your location."));
        let caption = HSTRING::from("Location access");
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

fn is_app_geolocation_request(kind: COREWEBVIEW2_PERMISSION_KIND, uri: &str, host: &str) -> bool {
    kind == COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION && is_app_origin(uri, host)
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
        let pem = certificate_pem(&certificate)?;
        let request_uri = webview_string(|value| unsafe { args.RequestUri(value) })?;
        let action = if is_app_origin(&request_uri, &host)
            && certificates.trusts_server_certificate(&host, &pem)
        {
            COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_ALWAYS_ALLOW
        } else {
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
    Ok(X509::from_pem(certificate_pem(certificate)?.as_bytes())
        .ok()
        .and_then(|certificate| certificate.to_der().ok())
        .is_some_and(|der| der == expected))
}

fn certificate_pem(certificate: &impl PemCertificate) -> windows::core::Result<String> {
    let mut value = PWSTR::null();
    unsafe { certificate.pem(&raw mut value)? };
    Ok(take_pwstr(value))
}

trait PemCertificate {
    unsafe fn pem(&self, value: *mut PWSTR) -> windows::core::Result<()>;
}

impl PemCertificate for ICoreWebView2Certificate {
    unsafe fn pem(&self, value: *mut PWSTR) -> windows::core::Result<()> {
        unsafe { self.ToPemEncoding(value) }
    }
}

impl PemCertificate for ICoreWebView2ClientCertificate {
    unsafe fn pem(&self, value: *mut PWSTR) -> windows::core::Result<()> {
        unsafe { self.ToPemEncoding(value) }
    }
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
    use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS;

    use super::{
        COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION, browser_arguments, is_app_geolocation_request,
        is_app_origin,
    };

    #[test]
    fn routes_only_the_app_host_through_the_local_proxy() {
        let arguments = browser_arguments("app.tokamak.local", 1234);
        assert!(
            arguments.contains("--proxy-pac-url=data:application/x-ns-proxy-autoconfig;base64,")
        );
    }

    #[test]
    fn recognizes_only_the_exact_app_origin() {
        assert!(is_app_origin(
            "https://app.tokamak.local/path",
            "app.tokamak.local"
        ));
        assert!(is_app_origin(
            "HTTPS://APP.TOKAMAK.LOCAL:443/path",
            "app.tokamak.local"
        ));
        assert!(!is_app_origin(
            "https://app.tokamak.local.evil/path",
            "app.tokamak.local"
        ));
        assert!(!is_app_origin(
            "https://app.tokamak.local:444/path",
            "app.tokamak.local"
        ));
        assert!(!is_app_origin(
            "http://app.tokamak.local/path",
            "app.tokamak.local"
        ));
    }

    #[test]
    fn permits_only_the_app_origin_to_request_location() {
        assert!(is_app_geolocation_request(
            COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION,
            "https://app.tokamak.local/",
            "app.tokamak.local",
        ));
        assert!(!is_app_geolocation_request(
            COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION,
            "https://other.tokamak.local/",
            "app.tokamak.local",
        ));
        assert!(!is_app_geolocation_request(
            COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION,
            "https://app.tokamak.local:444/",
            "app.tokamak.local",
        ));
        assert!(!is_app_geolocation_request(
            COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS,
            "https://app.tokamak.local/",
            "app.tokamak.local",
        ));
    }
}
