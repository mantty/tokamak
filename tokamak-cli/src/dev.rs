//! Development-session orchestration.

use std::fs;
use std::io::{self, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcessCommand, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName};
use tokamak_cli::Platform;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio_util::either::Either;

use super::devices::{
    PreparedDevice, install_and_launch_android, install_and_launch_ios_device,
    install_and_launch_ios_simulator,
};
use super::packs::PlatformPack;
use super::vite::{ConfigReport, PLUGIN_HINT, ServerReport, VitePlugin};
use super::{devices, pinned_tls, pipeline, settings};

const SERVER_READY_TIMEOUT: Duration = Duration::from_mins(1);
const APP_CONNECTION_TIMEOUT: Duration = Duration::from_mins(1);
const SERVER_POLL_INTERVAL: Duration = Duration::from_millis(100);
#[cfg(any(unix, windows))]
const PROCESS_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const RELAY_HEADER_LIMIT: usize = 64 * 1024;

/// Development command inputs collected by the CLI.
pub(crate) struct Request {
    pub(crate) device_id: String,
    pub(crate) project_dir: PathBuf,
    pub(crate) platform_pack_dir: Option<PathBuf>,
    pub(crate) top: settings::TopOptions,
    pub(crate) platform_options: settings::PlatformOptions,
    pub(crate) host_address: Option<String>,
    pub(crate) command: Vec<std::ffi::OsString>,
}

/// Run one development session until the framework process or native app
/// exits.
pub(crate) fn run(request: &Request) -> Result<()> {
    validate_request(request)?;
    let shutdown = shutdown_flag()?;
    let project = fs::canonicalize(&request.project_dir).with_context(|| {
        format!(
            "resolve project directory: {}",
            request.project_dir.display()
        )
    })?;
    let device = devices::prepare(&request.device_id)?;
    let pack = PlatformPack::load(device.platform, request.platform_pack_dir.as_deref())?;
    pipeline::check_settings(
        &request.top,
        &request.platform_options,
        device.platform,
        &pack.manifest,
    )?;
    let relay_host = relay_host(&device, request.host_address.as_deref())?;
    if device.platform == Platform::Ios && request.host_address.is_none() {
        println!("Using detected host address {relay_host} for physical iOS development");
    }
    let plugin = VitePlugin::new(
        project
            .join("build")
            .join(".tokamak")
            .join("dev")
            .join("vite"),
    );
    plugin.clear()?;
    let mut framework = spawn_framework(&request.command, &project, &plugin)?;

    if shutdown.load(Ordering::Acquire) {
        stop_process(&mut framework)?;
        return Ok(());
    }

    let result = run_session(&mut DevelopmentSession {
        request,
        project: &project,
        pack: &pack,
        device: &device,
        relay_host,
        plugin: &plugin,
        framework: &mut framework,
        shutdown: &shutdown,
    });
    if result.is_err() {
        stop_process(&mut framework)?;
    }
    result
}

fn relay_host(device: &PreparedDevice, configured: Option<&str>) -> Result<IpAddr> {
    if device.platform != Platform::Ios {
        return Ok(IpAddr::V4(Ipv4Addr::LOCALHOST));
    }
    if let Some(configured) = configured {
        return configured
            .parse()
            .with_context(|| format!("invalid physical-device host address `{configured}`"));
    }

    let interface = netdev::get_default_interface()
        .map_err(|error| anyhow::anyhow!("detect default host network interface: {error}"))?;
    usable_ipv4_address(interface.ipv4_addrs())
        .map(IpAddr::V4)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "could not detect a usable IPv4 address on the default host network interface; pass --host-address with an address reachable by the device"
            )
        })
}

fn usable_ipv4_address(addresses: impl IntoIterator<Item = Ipv4Addr>) -> Option<Ipv4Addr> {
    addresses.into_iter().find(|address| {
        !address.is_loopback()
            && !address.is_link_local()
            && !address.is_unspecified()
            && !address.is_multicast()
            && !address.is_broadcast()
    })
}

fn validate_request(request: &Request) -> Result<()> {
    if request.command.is_empty() {
        bail!("a development command is required after `--`, for example `-- astro dev`");
    }
    if !request.project_dir.is_dir() {
        bail!(
            "project directory does not exist: {}",
            request.project_dir.display()
        );
    }
    Ok(())
}

struct DevelopmentSession<'a> {
    request: &'a Request,
    project: &'a Path,
    pack: &'a PlatformPack,
    device: &'a PreparedDevice,
    relay_host: IpAddr,
    plugin: &'a VitePlugin,
    framework: &'a mut Child,
    shutdown: &'a AtomicBool,
}

fn run_session(session: &mut DevelopmentSession<'_>) -> Result<()> {
    let Some((config, report)) =
        wait_for_plugin(session.framework, session.plugin, session.shutdown)?
    else {
        stop_process(session.framework)?;
        return Ok(());
    };
    let tokamak = config.parse()?;
    let server = ServerEndpoint::parse(&report.url, &report.certificates)?;
    println!("Development server is ready at {}", server.display_url());
    let session_token = session_token()?;
    let relay = DevRelay::bind(server, session_token.clone(), session.relay_host)?;
    let summary = pipeline::run_development(&pipeline::DevelopmentRequest {
        platform: session.device.platform,
        project: session.project,
        pack: session.pack,
        tokamak: &tokamak,
        worker_name: &report.worker_name,
        endpoint: &relay.device_endpoint(),
        session_token: &session_token,
        device_id: &session.device.id,
        top: &session.request.top,
        platform_options: &session.request.platform_options,
    })?;
    if session.shutdown.load(Ordering::Acquire) {
        stop_process(session.framework)?;
        return Ok(());
    }
    let mut server = FollowedServer {
        plugin: session.plugin,
        relay: &relay,
        server: report,
        config,
    };
    let mut app = launch_app(&summary, session.device, relay.port())?;
    let mut stdout = io::stdout();
    if let Err(error) = wait_for_app_connection(
        session.framework,
        &mut server,
        session.shutdown,
        session.device,
        &mut stdout,
        APP_CONNECTION_TIMEOUT,
    ) {
        app.stop();
        return Err(error);
    }
    if session.shutdown.load(Ordering::Acquire) {
        app.stop();
        stop_process(session.framework)?;
        return Ok(());
    }
    supervise(
        session.framework,
        &mut app,
        &mut server,
        session.shutdown,
        &mut stdout,
    )
}

fn spawn_framework(
    command: &[std::ffi::OsString],
    project: &Path,
    plugin: &VitePlugin,
) -> Result<Child> {
    let Some(program) = command.first() else {
        bail!("a development command is required");
    };
    let mut process = ProcessCommand::new(program);
    process
        .args(&command[1..])
        .current_dir(project)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .envs(plugin.environment());
    configure_process_group(&mut process);
    process.spawn().with_context(|| {
        format!(
            "failed to start development command `{}`",
            program.to_string_lossy()
        )
    })
}

/// The app's configuration and the development server, once the plugin has
/// reported them, or `None` when shutdown is requested first.
fn wait_for_plugin(
    child: &mut Child,
    plugin: &VitePlugin,
    shutdown: &AtomicBool,
) -> Result<Option<(ConfigReport, ServerReport)>> {
    let deadline = Instant::now() + SERVER_READY_TIMEOUT;
    loop {
        if shutdown.load(Ordering::Acquire) {
            return Ok(None);
        }
        if let Some(status) = child.try_wait()? {
            bail!("development command exited before its server was ready ({status})");
        }
        // The plugin reports the configuration before the server address.
        if let Some(server) = plugin.server()? {
            let config = plugin.config("the development command")?;
            return Ok(Some((config, server)));
        }
        if Instant::now() >= deadline {
            bail!(
                "the development command did not report its server address within {} seconds; {PLUGIN_HINT}",
                SERVER_READY_TIMEOUT.as_secs()
            );
        }
        thread::sleep(SERVER_POLL_INTERVAL);
    }
}

fn wait_for_app_connection(
    framework: &mut Child,
    server: &mut FollowedServer<'_>,
    shutdown: &AtomicBool,
    device: &PreparedDevice,
    output: &mut impl Write,
    timeout: Duration,
) -> Result<()> {
    writeln!(output, "Waiting for the development app to connect...")?;
    output.flush()?;
    let deadline = Instant::now() + timeout;
    loop {
        if shutdown.load(Ordering::Acquire) {
            return Ok(());
        }
        if let Some(status) = framework.try_wait()? {
            bail!("development command exited before the app connected ({status})");
        }
        server.follow(output)?;
        if server.relay.app_connected() {
            writeln!(
                output,
                "Development app connected from {} ({})",
                device.id, device.kind
            )?;
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!(
                "development app did not connect to {} within {} seconds; check app startup and device connectivity",
                server.relay.device_endpoint(),
                timeout.as_secs()
            );
        }
        thread::sleep(SERVER_POLL_INTERVAL);
    }
}

fn supervise(
    framework: &mut Child,
    app: &mut LaunchedApp,
    server: &mut FollowedServer<'_>,
    shutdown: &AtomicBool,
    output: &mut impl Write,
) -> Result<()> {
    loop {
        if shutdown.load(Ordering::Acquire) {
            app.stop();
            stop_process(framework)?;
            return Ok(());
        }
        if let Some(status) = framework.try_wait()? {
            app.stop();
            stop_process(framework)?;
            if shutdown.load(Ordering::Acquire) {
                return Ok(());
            }
            return status_result("development command", status);
        }
        if app.has_exited()? {
            stop_process(framework)?;
            return Ok(());
        }
        if let Err(error) = server.follow(output) {
            app.stop();
            return Err(error);
        }
        thread::sleep(SERVER_POLL_INTERVAL);
    }
}

/// The development server as the relay follows it: a restart of the server
/// reruns the plugin, which reports it again.
struct FollowedServer<'a> {
    plugin: &'a VitePlugin,
    relay: &'a DevRelay,
    /// The server and configuration the plugin last reported.
    server: ServerReport,
    config: ConfigReport,
}

impl FollowedServer<'_> {
    /// Point the relay at the server the plugin reports, and say when the
    /// reported address or configuration changes.
    fn follow(&mut self, output: &mut impl Write) -> Result<()> {
        if let Some(server) = self.plugin.server()?
            && server != self.server
        {
            let endpoint = ServerEndpoint::parse(&server.url, &server.certificates)?;
            if server.url != self.server.url {
                writeln!(
                    output,
                    "Development server moved to {}",
                    endpoint.display_url()
                )?;
            }
            self.relay.set_server(endpoint);
            self.server = server;
        }
        let config = self.plugin.config("the development command")?;
        if config != self.config {
            writeln!(
                output,
                "The tokamak configuration changed; restart `tok dev` to apply it to the app"
            )?;
            self.config = config;
        }
        Ok(())
    }
}

fn status_result(label: &str, status: ExitStatus) -> Result<()> {
    if status.success() {
        Ok(())
    } else {
        bail!("{label} exited with status {status}");
    }
}

/// A flag set once the user stops the session with SIGINT, SIGTERM or
/// SIGHUP, or a Windows console control event.
fn shutdown_flag() -> Result<Arc<AtomicBool>> {
    let requested = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&requested);
    ctrlc::set_handler(move || flag.store(true, Ordering::Release))
        .context("install the development signal handler")?;
    Ok(requested)
}

#[cfg(unix)]
fn configure_process_group(command: &mut ProcessCommand) {
    use std::os::unix::process::CommandExt;

    command.process_group(0);
}

#[cfg(not(unix))]
const fn configure_process_group(_command: &mut ProcessCommand) {}

fn stop_process(child: &mut Child) -> Result<()> {
    terminate_process_tree(child)?;
    let _ = child.wait();
    Ok(())
}

#[cfg(unix)]
fn terminate_process_tree(child: &mut Child) -> Result<()> {
    send_process_group_signal(child.id(), "TERM");

    let deadline = Instant::now() + PROCESS_SHUTDOWN_TIMEOUT;
    while Instant::now() < deadline {
        let root_running = child.try_wait()?.is_none();
        if !root_running && !process_group_is_running(child.id()) {
            return Ok(());
        }
        thread::sleep(SERVER_POLL_INTERVAL);
    }

    send_process_group_signal(child.id(), "KILL");
    if child.try_wait()?.is_none() {
        child.kill().context("stop development command")?;
    }
    Ok(())
}

#[cfg(unix)]
fn process_group_is_running(pid: u32) -> bool {
    send_process_group_signal(pid, "0")
}

/// Whether `kill` delivered `signal` to the process group `pid` leads.
// `--` ends kill's options, so it reads the group as a process ID; procps-ng
// 4.0.4 reads `-1234` after a signal as `-1`, which signals every process.
#[cfg(unix)]
fn send_process_group_signal(pid: u32, signal: &str) -> bool {
    let flag = format!("-{signal}");
    let group = format!("-{pid}");
    ProcessCommand::new("kill")
        .args([flag.as_str(), "--", group.as_str()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(windows)]
fn terminate_process_tree(child: &mut Child) -> Result<()> {
    let pid = child.id().to_string();
    let _ = ProcessCommand::new("taskkill")
        .args(["/PID", &pid, "/T"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("stop development command")?;

    let deadline = Instant::now() + PROCESS_SHUTDOWN_TIMEOUT;
    while Instant::now() < deadline && child.try_wait()?.is_none() {
        thread::sleep(SERVER_POLL_INTERVAL);
    }
    if child.try_wait()?.is_none() {
        let status = ProcessCommand::new("taskkill")
            .args(["/PID", &pid, "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("force-stop development command")?;
        if !status.success() && child.try_wait()?.is_none() {
            child.kill().context("force-stop development command")?;
        }
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn terminate_process_tree(child: &mut Child) -> Result<()> {
    child.kill().context("stop development command")?;
    Ok(())
}

fn launch_app(
    summary: &pipeline::DevelopmentSummary,
    device: &PreparedDevice,
    relay_port: u16,
) -> Result<LaunchedApp> {
    let (id, bundle, identifier) = (&device.id, &summary.bundle_dir, &summary.identifier);
    match summary.platform {
        Platform::Macos => return launch_macos(summary),
        Platform::Windows => return launch_windows(summary),
        Platform::IosSimulator => install_and_launch_ios_simulator(id, bundle, identifier)?,
        Platform::Ios => install_and_launch_ios_device(id, bundle, identifier)?,
        Platform::Android => install_and_launch_android(id, bundle, identifier, relay_port)?,
    }
    Ok(LaunchedApp::detached())
}

fn launch_macos(summary: &pipeline::DevelopmentSummary) -> Result<LaunchedApp> {
    let executable = summary
        .bundle_dir
        .join("Contents/MacOS")
        .join(&summary.app_slug);
    let mut command = ProcessCommand::new(&executable);
    configure_process_group(&mut command);
    let process = command
        .spawn()
        .with_context(|| format!("launch macOS app {}", executable.display()))?;
    Ok(LaunchedApp::process(process))
}

fn launch_windows(summary: &pipeline::DevelopmentSummary) -> Result<LaunchedApp> {
    let executable = summary.bundle_dir.join(format!("{}.exe", summary.app_slug));
    let process = ProcessCommand::new(&executable)
        .spawn()
        .with_context(|| format!("launch Windows app {}", executable.display()))?;
    Ok(LaunchedApp {
        #[cfg(windows)]
        _job: end_with_session(&process),
        process: Some(process),
    })
}

/// A job that ends `process` once the job is dropped or `tok` exits, however
/// `tok` exits.
#[cfg(windows)]
fn end_with_session(process: &Child) -> Option<win32job::Job> {
    use std::os::windows::io::AsRawHandle;
    use win32job::{ExtendedLimitInfo, Job};

    Job::create_with_limit_info(ExtendedLimitInfo::new().limit_kill_on_job_close())
        .and_then(|job| {
            job.assign_process(process.as_raw_handle() as isize)
                .map(|()| job)
        })
        .inspect_err(|error| eprintln!("warning: the Windows app may outlive tok: {error}"))
        .ok()
}

fn session_token() -> Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).context("generate development session token")?;
    Ok(hex::encode(bytes))
}

struct LaunchedApp {
    process: Option<Child>,
    #[cfg(windows)]
    _job: Option<win32job::Job>,
}

impl LaunchedApp {
    fn process(process: Child) -> Self {
        Self {
            process: Some(process),
            #[cfg(windows)]
            _job: None,
        }
    }

    const fn detached() -> Self {
        Self {
            process: None,
            #[cfg(windows)]
            _job: None,
        }
    }

    fn has_exited(&mut self) -> Result<bool> {
        self.process
            .as_mut()
            .map_or(Ok(false), |process| Ok(process.try_wait()?.is_some()))
    }

    fn stop(&mut self) {
        if let Some(process) = &mut self.process {
            let _ = stop_process(process);
        }
    }
}

#[derive(Clone)]
struct ServerEndpoint {
    url: String,
    authority: String,
    addresses: Vec<SocketAddr>,
    /// The connector and name of an `https://` server.
    tls: Option<(TlsConnector, ServerName<'static>)>,
}

impl ServerEndpoint {
    /// The server at `url`, an `http://` or `https://` URL whose path is
    /// ignored; an `https://` server must serve one of `certificates`, as PEM.
    fn parse(url: &str, certificates: &str) -> Result<Self> {
        let (scheme, address) = url
            .split_once("://")
            .filter(|(scheme, _)| matches!(*scheme, "http" | "https"))
            .ok_or_else(|| {
                anyhow::anyhow!("development server must use an http:// or https:// URL")
            })?;
        let authority = address
            .split_once('/')
            .map_or(address, |(authority, _)| authority);
        if authority.is_empty()
            || authority.contains('?')
            || authority.contains('#')
            || authority.chars().any(char::is_whitespace)
        {
            bail!("development server URL must contain only a host and port");
        }
        let (host, port) = parse_authority(authority)?;
        let addresses = (host.as_str(), port)
            .to_socket_addrs()
            .with_context(|| format!("resolve development server {authority}"))?
            .collect::<Vec<_>>();
        if addresses.is_empty() {
            bail!("development server host did not resolve: {host}");
        }
        let tls = if scheme == "https" {
            let name = ServerName::try_from(host).context("development server host is invalid")?;
            Some((pinned_connector(certificates)?, name))
        } else {
            None
        };
        Ok(Self {
            url: format!("{scheme}://{authority}"),
            authority: authority.to_owned(),
            addresses,
            tls,
        })
    }

    async fn connect(&self) -> io::Result<Either<TcpStream, TlsStream<TcpStream>>> {
        let stream = TcpStream::connect(self.addresses.as_slice()).await?;
        match &self.tls {
            None => Ok(Either::Left(stream)),
            Some((connector, name)) => Ok(Either::Right(
                connector.connect(name.clone(), stream).await?,
            )),
        }
    }

    fn display_url(&self) -> &str {
        &self.url
    }
}

/// A TLS connector that trusts only `certificates`, as PEM.
fn pinned_connector(certificates: &str) -> Result<TlsConnector> {
    let certificates = CertificateDer::pem_slice_iter(certificates.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .context("read the development server's certificate")?;
    if certificates.is_empty() {
        bail!(
            "the development server uses HTTPS, but the tokamak Vite plugin found no certificate in Vite's `server.https.cert`, which tok dev needs to trust the server"
        );
    }
    let config = pinned_tls::client_config(certificates)?;
    Ok(TlsConnector::from(Arc::new(config)))
}

fn parse_authority(authority: &str) -> Result<(String, u16)> {
    if let Some(rest) = authority.strip_prefix('[') {
        let end = rest
            .find(']')
            .ok_or_else(|| anyhow::anyhow!("development server has an invalid IPv6 host"))?;
        let host = rest[..end].to_owned();
        let port = rest[end + 1..]
            .strip_prefix(':')
            .ok_or_else(|| anyhow::anyhow!("development server port is required"))?
            .parse()
            .context("development server port is invalid")?;
        return Ok((host, port));
    }
    let (host, port) = authority
        .rsplit_once(':')
        .ok_or_else(|| anyhow::anyhow!("development server port is required"))?;
    if host.is_empty() || host.contains(':') {
        bail!("development server host is invalid");
    }
    Ok((
        host.to_owned(),
        port.parse().context("development server port is invalid")?,
    ))
}

struct DevRelay {
    address: SocketAddr,
    advertised_host: IpAddr,
    server: Arc<Mutex<ServerEndpoint>>,
    app_connected: Arc<AtomicBool>,
    /// Runs the relay until it is dropped.
    _runtime: tokio::runtime::Runtime,
}

impl DevRelay {
    fn bind(server: ServerEndpoint, session_token: String, host: IpAddr) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("tokamak-dev-relay")
            .enable_io()
            .build()
            .context("start development relay")?;
        let listener = runtime
            .block_on(TcpListener::bind((host, 0)))
            .context("bind development relay")?;
        let address = listener.local_addr()?;
        let server = Arc::new(Mutex::new(server));
        let app_connected = Arc::new(AtomicBool::new(false));
        runtime.spawn(relay_loop(
            listener,
            Arc::clone(&server),
            session_token,
            Arc::clone(&app_connected),
        ));
        Ok(Self {
            address,
            advertised_host: host,
            server,
            app_connected,
            _runtime: runtime,
        })
    }

    fn port(&self) -> u16 {
        self.address.port()
    }

    fn device_endpoint(&self) -> String {
        match self.advertised_host {
            IpAddr::V4(host) => format!("http://{host}:{}", self.port()),
            IpAddr::V6(host) => format!("http://[{host}]:{}", self.port()),
        }
    }

    fn app_connected(&self) -> bool {
        self.app_connected.load(Ordering::Acquire)
    }

    /// Relay later connections to `server`.
    fn set_server(&self, server: ServerEndpoint) {
        *self.server.lock().unwrap_or_else(PoisonError::into_inner) = server;
    }
}

async fn relay_loop(
    listener: TcpListener,
    server: Arc<Mutex<ServerEndpoint>>,
    session_token: String,
    app_connected: Arc<AtomicBool>,
) {
    while let Ok((stream, _)) = listener.accept().await {
        let server = server
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let session_token = session_token.clone();
        let app_connected = Arc::clone(&app_connected);
        tokio::spawn(async move {
            if let Err(error) =
                relay_connection(stream, &server, &session_token, &app_connected).await
            {
                eprintln!("tokamak development relay connection failed: {error}");
            }
        });
    }
}

async fn relay_connection(
    mut downstream: TcpStream,
    server: &ServerEndpoint,
    session_token: &str,
    app_connected: &AtomicBool,
) -> io::Result<()> {
    let initial = read_headers(&mut downstream).await?;
    if !authorized(&initial, session_token) {
        let _ = downstream
            .write_all(
                b"HTTP/1.1 401 Unauthorized\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
            )
            .await;
        return Ok(());
    }
    let mut upstream = server.connect().await?;
    let request = rewrite_request(&initial, &server.authority)?;
    upstream.write_all(&request).await?;
    upstream.flush().await?;
    app_connected.store(true, Ordering::Release);
    match tokio::io::copy_bidirectional(&mut downstream, &mut upstream).await {
        // HTTP delimits its own messages, so an HTTPS server closing without
        // TLS close_notify ends the connection as an HTTP server's close does.
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(()),
        result => result.map(drop),
    }
}

async fn read_headers(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(4096);
    let mut chunk = [0_u8; 4096];
    loop {
        let count = stream.read(&mut chunk).await?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "development relay received an incomplete request",
            ));
        }
        bytes.extend_from_slice(&chunk[..count]);
        if find_header_end(&bytes).is_some() {
            return Ok(bytes);
        }
        if bytes.len() > RELAY_HEADER_LIMIT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "development relay request headers are too large",
            ));
        }
    }
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn authorized(request: &[u8], expected: &str) -> bool {
    let Some(end) = find_header_end(request) else {
        return false;
    };
    let mut found = false;
    for line in request[..end].split(|byte| *byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Some((name, value)) = header_parts(line) else {
            continue;
        };
        if name.eq_ignore_ascii_case(b"x-tokamak-session") {
            found = true;
            if value.trim_ascii() != expected.as_bytes() {
                return false;
            }
        }
    }
    found
}

fn rewrite_request(request: &[u8], authority: &str) -> io::Result<Vec<u8>> {
    let end = find_header_end(request).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "development relay received an incomplete request",
        )
    })?;
    let lines = request[..end]
        .split(|byte| *byte == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line));
    let upgrade = lines
        .clone()
        .skip(1)
        .filter_map(header_parts)
        .any(|(name, _)| name.eq_ignore_ascii_case(b"upgrade"));
    let mut rewritten = Vec::with_capacity(request.len());
    for (index, line) in lines.enumerate() {
        let name = header_parts(line)
            .filter(|_| index > 0)
            .map(|(name, _)| name);
        let named = |expected: &[u8]| name.is_some_and(|name| name.eq_ignore_ascii_case(expected));
        if named(b"x-tokamak-session") || (named(b"connection") && !upgrade) {
            continue;
        }
        if named(b"host") {
            rewritten.extend_from_slice(b"Host: ");
            rewritten.extend_from_slice(authority.as_bytes());
        } else {
            rewritten.extend_from_slice(line);
        }
        rewritten.extend_from_slice(b"\r\n");
    }
    // The relay rewrites only a connection's first request, so a request
    // that does not upgrade the connection closes it.
    if !upgrade {
        rewritten.extend_from_slice(b"Connection: close\r\n");
    }
    rewritten.extend_from_slice(b"\r\n");
    rewritten.extend_from_slice(&request[end + 4..]);
    Ok(rewritten)
}

fn header_parts(line: &[u8]) -> Option<(&[u8], &[u8])> {
    let separator = line.iter().position(|byte| *byte == b':')?;
    Some((&line[..separator], &line[separator + 1..]))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{Read, Write};
    use std::net::{IpAddr, Ipv4Addr, TcpListener, TcpStream};
    #[cfg(unix)]
    use std::process::Command;
    use std::sync::Arc;
    #[cfg(unix)]
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::Duration;

    use anyhow::Context;
    use rustls::pki_types::PrivatePkcs8KeyDer;
    use rustls::{ServerConnection, StreamOwned};
    use serde_json::{Value, json};

    use super::{
        DevRelay, FollowedServer, PreparedDevice, ServerEndpoint, VitePlugin, authorized,
        parse_authority, relay_host, rewrite_request, usable_ipv4_address,
    };
    #[cfg(unix)]
    use super::{
        configure_process_group, process_group_is_running, stop_process, wait_for_app_connection,
        wait_for_plugin,
    };
    use tokamak_cli::Platform;

    /// A relay to a server that accepts no connections, and the plugin, in a
    /// temporary directory, having reported that server and no options.
    struct Reported {
        _server: TcpListener,
        directory: tempfile::TempDir,
        plugin: VitePlugin,
        relay: DevRelay,
    }

    impl Reported {
        fn new() -> anyhow::Result<Self> {
            let server = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
            let url = format!("http://{}", server.local_addr()?);
            let directory = tempfile::tempdir()?;
            let relay = DevRelay::bind(
                ServerEndpoint::parse(&url, "")?,
                "token".to_owned(),
                IpAddr::V4(Ipv4Addr::LOCALHOST),
            )?;
            let reported = Self {
                _server: server,
                plugin: VitePlugin::new(directory.path().to_owned()),
                directory,
                relay,
            };
            reported.report("server.json", &json!({ "url": url }))?;
            reported.report("config.json", &json!({ "root": "/app", "config": {} }))?;
            Ok(reported)
        }

        /// Write the plugin's report `name`.
        fn report(&self, name: &str, report: &Value) -> std::io::Result<()> {
            fs::write(self.directory.path().join(name), report.to_string())
        }

        /// The relay following what the plugin reports.
        fn following(&self) -> anyhow::Result<FollowedServer<'_>> {
            Ok(FollowedServer {
                plugin: &self.plugin,
                relay: &self.relay,
                server: self.plugin.server()?.context("no server report")?,
                config: self.plugin.config("the test")?,
            })
        }
    }

    #[cfg(unix)]
    #[test]
    fn stops_only_the_process_group_it_names() -> Result<(), Box<dyn std::error::Error>> {
        let spawn = || {
            let mut command = Command::new("sleep");
            command.arg("30");
            configure_process_group(&mut command);
            command.spawn()
        };
        let mut stopped = spawn()?;
        let mut other = spawn()?;

        assert!(process_group_is_running(stopped.id()));
        stop_process(&mut stopped)?;

        assert!(!process_group_is_running(stopped.id()));
        assert!(process_group_is_running(other.id()));
        stop_process(&mut other)?;
        Ok(())
    }

    #[test]
    fn parses_server_authorities() {
        assert_eq!(
            parse_authority("127.0.0.1:5173").ok(),
            Some(("127.0.0.1".to_owned(), 5173))
        );
        assert_eq!(
            parse_authority("[::1]:3000").ok(),
            Some(("::1".to_owned(), 3000))
        );
    }

    #[test]
    fn takes_the_server_address_from_a_url() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerEndpoint::parse("http://127.0.0.1:5174/", "")?;
        assert_eq!(server.display_url(), "http://127.0.0.1:5174");
        assert_eq!(server.authority, "127.0.0.1:5174");
        let certificate = TlsServer::new()?.certificate;
        let server = ServerEndpoint::parse("https://localhost:5174/", &certificate)?;
        assert_eq!(server.display_url(), "https://localhost:5174");
        assert!(ServerEndpoint::parse("ftp://127.0.0.1:5174/", "").is_err());
        Ok(())
    }

    #[test]
    fn requires_the_certificate_of_an_https_server() {
        assert!(
            ServerEndpoint::parse("https://localhost:5174/", "").is_err_and(|error| error
                .to_string()
                .contains("found no certificate in Vite's `server.https.cert`"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn stops_waiting_when_the_plugin_reports_an_error() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let plugin = VitePlugin::new(directory.path().to_owned());
        let error = "Vite is running in middleware mode";
        fs::write(
            directory.path().join("server.json"),
            json!({ "error": error }).to_string(),
        )?;
        let mut framework = Command::new("sleep").arg("30").spawn()?;

        let result = wait_for_plugin(&mut framework, &plugin, &AtomicBool::new(false));
        let _ = framework.kill();
        let _ = framework.wait();

        assert_eq!(
            result.err().map(|error| error.to_string()),
            Some(error.to_owned())
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn waits_for_app_connection_before_reporting_ready() -> Result<(), Box<dyn std::error::Error>> {
        let reported = Reported::new()?;
        let relay = &reported.relay;
        let mut server = reported.following()?;
        let mut framework = Command::new("sleep").arg("30").spawn()?;
        let shutdown = AtomicBool::new(false);
        let device = PreparedDevice {
            id: "device".to_owned(),
            kind: "iPhone".to_owned(),
            platform: Platform::Ios,
        };
        let mut output = Vec::new();

        let not_ready = wait_for_app_connection(
            &mut framework,
            &mut server,
            &shutdown,
            &device,
            &mut output,
            Duration::ZERO,
        );
        let not_ready_output = String::from_utf8(output)?;
        assert!(not_ready_output.contains("Waiting for the development app to connect"));
        assert!(!not_ready_output.contains("Development app connected"));

        relay.app_connected.store(true, Ordering::Release);
        let mut output = Vec::new();
        let ready = wait_for_app_connection(
            &mut framework,
            &mut server,
            &shutdown,
            &device,
            &mut output,
            Duration::ZERO,
        );
        let _ = framework.kill();
        let _ = framework.wait();

        assert!(not_ready.is_err_and(|error| {
            let message = error.to_string();
            message.contains("did not connect") && message.contains(&relay.device_endpoint())
        }));
        assert!(ready.is_ok());
        assert_eq!(
            String::from_utf8(output)?,
            "Waiting for the development app to connect...\nDevelopment app connected from device (iPhone)\n"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn stops_waiting_when_shutdown_is_requested() -> Result<(), Box<dyn std::error::Error>> {
        let reported = Reported::new()?;
        let mut server = reported.following()?;
        let mut framework = Command::new("sleep").arg("30").spawn()?;
        let shutdown = AtomicBool::new(true);
        let device = PreparedDevice {
            id: "device".to_owned(),
            kind: "iPhone".to_owned(),
            platform: Platform::Ios,
        };
        let mut output = Vec::new();

        let result = wait_for_app_connection(
            &mut framework,
            &mut server,
            &shutdown,
            &device,
            &mut output,
            Duration::ZERO,
        );
        let _ = framework.kill();
        let _ = framework.wait();

        assert!(result.is_ok());
        assert!(!String::from_utf8(output)?.contains("Development app connected"));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn reports_framework_exit_while_waiting_for_app_connection()
    -> Result<(), Box<dyn std::error::Error>> {
        let reported = Reported::new()?;
        let mut server = reported.following()?;
        let mut framework = Command::new("true").spawn()?;
        let _ = framework.wait()?;
        let shutdown = AtomicBool::new(false);
        let device = PreparedDevice {
            id: "device".to_owned(),
            kind: "iPhone".to_owned(),
            platform: Platform::Ios,
        };
        let mut output = Vec::new();

        let result = wait_for_app_connection(
            &mut framework,
            &mut server,
            &shutdown,
            &device,
            &mut output,
            Duration::from_secs(1),
        );

        assert!(result.is_err_and(|error| {
            error
                .to_string()
                .contains("exited before the app connected")
        }));
        assert!(!String::from_utf8(output)?.contains("Development app connected"));
        Ok(())
    }

    #[test]
    fn requires_the_exact_session_header() {
        let request = b"GET / HTTP/1.1\r\nX-Tokamak-Session: token\r\n\r\n";
        assert!(authorized(request, "token"));
        assert!(!authorized(request, "wrong"));
        assert!(!authorized(b"GET / HTTP/1.1\r\n\r\n", "token"));
    }

    #[test]
    fn rewrites_host_and_removes_the_session_secret() {
        let request =
            b"GET / HTTP/1.1\r\nHost: app.tokamak.local\r\nX-Tokamak-Session: token\r\n\r\n";
        let rewritten = rewrite_request(request, "127.0.0.1:5173").ok();
        assert_eq!(
            rewritten,
            Some(b"GET / HTTP/1.1\r\nHost: 127.0.0.1:5173\r\nConnection: close\r\n\r\n".to_vec())
        );
    }

    #[test]
    fn closes_the_connection_after_one_request_unless_it_upgrades() {
        let request = b"POST / HTTP/1.1\r\nconnection: keep-alive\r\nContent-Length: 2\r\n\r\nhi";
        assert_eq!(
            rewrite_request(request, "127.0.0.1:5173").ok(),
            Some(b"POST / HTTP/1.1\r\nContent-Length: 2\r\nConnection: close\r\n\r\nhi".to_vec())
        );
        let upgrade = b"GET / HTTP/1.1\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n";
        assert_eq!(
            rewrite_request(upgrade, "127.0.0.1:5173").ok(),
            Some(upgrade.to_vec())
        );
    }

    #[test]
    fn selects_the_first_usable_ipv4_address() {
        assert_eq!(
            usable_ipv4_address([
                Ipv4Addr::LOCALHOST,
                Ipv4Addr::new(169, 254, 1, 2),
                Ipv4Addr::new(192, 168, 1, 42),
            ]),
            Some(Ipv4Addr::new(192, 168, 1, 42))
        );
        assert_eq!(
            usable_ipv4_address([Ipv4Addr::LOCALHOST, Ipv4Addr::UNSPECIFIED]),
            None
        );
    }

    #[test]
    fn explicit_host_address_is_used_for_physical_ios() -> Result<(), Box<dyn std::error::Error>> {
        let device = PreparedDevice {
            id: "device".to_owned(),
            kind: "iPhone".to_owned(),
            platform: Platform::Ios,
        };
        assert_eq!(
            relay_host(&device, Some("192.168.1.42"))?,
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 42))
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn launches_macos_using_the_app_slug() -> Result<(), Box<dyn std::error::Error>> {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempfile::tempdir()?;
        let executable_dir = temporary.path().join("Contents/MacOS");
        fs::create_dir_all(&executable_dir)?;
        let executable = executable_dir.join("my-app");
        fs::write(&executable, "#!/bin/sh\nsleep 10\n")?;
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))?;

        let summary = super::pipeline::DevelopmentSummary {
            platform: Platform::Macos,
            bundle_dir: temporary.path().to_owned(),
            app_slug: "my-app".to_owned(),
            identifier: "com.example.my-app".to_owned(),
        };
        let mut app = super::launch_macos(&summary)?;
        assert!(!app.has_exited()?);
        app.stop();
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn launches_windows_using_the_app_slug() -> Result<(), Box<dyn std::error::Error>> {
        use std::fs;

        let temporary = tempfile::tempdir()?;
        let executable = temporary.path().join("my-app.exe");
        let system_root = std::env::var_os("SystemRoot").ok_or("SystemRoot is unavailable")?;
        fs::copy(
            std::path::PathBuf::from(system_root)
                .join("System32")
                .join("timeout.exe"),
            &executable,
        )?;

        let summary = super::pipeline::DevelopmentSummary {
            platform: Platform::Windows,
            bundle_dir: temporary.path().to_owned(),
            app_slug: "my-app".to_owned(),
            identifier: "com.example.my-app".to_owned(),
        };
        let mut app = super::launch_windows(&summary)?;
        app.stop();
        Ok(())
    }

    #[test]
    fn relays_an_authenticated_http_request() -> Result<(), Box<dyn std::error::Error>> {
        let upstream = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let upstream_endpoint = format!("http://{}", upstream.local_addr()?);
        let endpoint = ServerEndpoint::parse(&upstream_endpoint, "")?;
        let upstream_thread = thread::spawn(move || -> std::io::Result<()> {
            let (mut stream, _) = upstream.accept()?;
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            loop {
                let count = stream.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            if !request.starts_with(b"GET / HTTP/1.1\r\n")
                || !request.windows(4).any(|window| window == b"\r\n\r\n")
                || !request
                    .windows(b"Host: 127.0.0.1:".len())
                    .any(|window| window == b"Host: 127.0.0.1:")
                || request
                    .windows(b"X-Tokamak-Session".len())
                    .any(|window| window == b"X-Tokamak-Session")
            {
                return Err(std::io::Error::other("relay did not forward the request"));
            }
            stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
            )?;
            Ok(())
        });

        let relay = DevRelay::bind(
            endpoint,
            "token".to_owned(),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
        )?;
        assert!(!relay.app_connected());

        let mut unauthorized = TcpStream::connect((Ipv4Addr::LOCALHOST, relay.port()))?;
        unauthorized.write_all(b"GET / HTTP/1.1\r\nHost: app.tokamak.local\r\n\r\n")?;
        unauthorized.shutdown(std::net::Shutdown::Write)?;
        let mut unauthorized_response = String::new();
        unauthorized.read_to_string(&mut unauthorized_response)?;
        assert!(unauthorized_response.starts_with("HTTP/1.1 401 Unauthorized"));
        assert!(!relay.app_connected());

        let mut client = TcpStream::connect((Ipv4Addr::LOCALHOST, relay.port()))?;
        write!(
            client,
            "GET / HTTP/1.1\r\nHost: app.tokamak.local\r\nX-Tokamak-Session: token\r\n\r\n"
        )?;
        client.shutdown(std::net::Shutdown::Write)?;
        let mut response = String::new();
        client.read_to_string(&mut response)?;
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.ends_with("\r\n\r\nok"));
        assert!(relay.app_connected());
        upstream_thread
            .join()
            .map_err(|_| "upstream thread panicked")??;
        Ok(())
    }

    #[test]
    fn relays_to_the_server_the_plugin_reports_after_a_restart()
    -> Result<(), Box<dyn std::error::Error>> {
        let restarted = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let restarted_url = format!("http://{}", restarted.local_addr()?);
        let upstream_thread = thread::spawn(move || -> std::io::Result<()> {
            let (mut stream, _) = restarted.accept()?;
            let _ = stream.read(&mut [0_u8; 1024])?;
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\nrestarted")
        });
        let reported = Reported::new()?;
        let mut server = reported.following()?;
        let mut output = Vec::new();

        server.follow(&mut output)?;
        assert!(output.is_empty());
        reported.report("server.json", &json!({ "url": restarted_url }))?;
        server.follow(&mut output)?;
        assert_eq!(
            String::from_utf8(output)?,
            format!("Development server moved to {restarted_url}\n")
        );

        let mut client = TcpStream::connect((Ipv4Addr::LOCALHOST, reported.relay.port()))?;
        client.set_read_timeout(Some(Duration::from_secs(5)))?;
        client.write_all(b"GET / HTTP/1.1\r\nX-Tokamak-Session: token\r\n\r\n")?;
        let mut response = String::new();
        client.read_to_string(&mut response)?;
        assert!(response.ends_with("\r\n\r\nrestarted"));
        upstream_thread
            .join()
            .map_err(|_| "upstream thread panicked")??;
        Ok(())
    }

    #[test]
    fn relays_requests_and_upgrades_to_an_https_server() -> Result<(), Box<dyn std::error::Error>> {
        let server = TlsServer::new()?;
        let relay = DevRelay::bind(
            ServerEndpoint::parse(&server.url()?, &server.certificate)?,
            "token".to_owned(),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
        )?;
        let upstream_thread = thread::spawn(move || -> std::io::Result<()> {
            let mut request = server.accept()?;
            if !read_head(&mut request)?.starts_with("GET / HTTP/1.1\r\nHost: localhost:") {
                return Err(std::io::Error::other("relay did not forward the request"));
            }
            request.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")?;
            request.conn.send_close_notify();
            request.flush()?;

            let mut upgrade = server.accept()?;
            read_head(&mut upgrade)?;
            upgrade.write_all(b"HTTP/1.1 101 Switching Protocols\r\n\r\nhello")?;
            let mut ping = [0_u8; 4];
            upgrade.read_exact(&mut ping)?;
            if &ping != b"ping" {
                return Err(std::io::Error::other(
                    "relay did not forward upgraded bytes",
                ));
            }
            upgrade.write_all(b"pong")
        });

        let mut client = relay_client(&relay)?;
        client.write_all(
            b"GET / HTTP/1.1\r\nHost: app.tokamak.local\r\nX-Tokamak-Session: token\r\n\r\n",
        )?;
        let mut response = String::new();
        client.read_to_string(&mut response)?;
        assert!(response.ends_with("\r\n\r\nok"));

        let mut client = relay_client(&relay)?;
        client.write_all(
            b"GET /ws HTTP/1.1\r\nUpgrade: websocket\r\nX-Tokamak-Session: token\r\n\r\n",
        )?;
        assert_eq!(
            read_head(&mut client)?,
            "HTTP/1.1 101 Switching Protocols\r\n\r\n"
        );
        let mut hello = [0_u8; 5];
        client.read_exact(&mut hello)?;
        assert_eq!(&hello, b"hello");
        client.write_all(b"ping")?;
        let mut pong = [0_u8; 4];
        client.read_exact(&mut pong)?;
        assert_eq!(&pong, b"pong");
        upstream_thread
            .join()
            .map_err(|_| "upstream thread panicked")??;
        Ok(())
    }

    #[test]
    fn refuses_an_https_server_with_another_certificate() -> Result<(), Box<dyn std::error::Error>>
    {
        let server = TlsServer::new()?;
        let other = TlsServer::new()?.certificate;
        let relay = DevRelay::bind(
            ServerEndpoint::parse(&server.url()?, &other)?,
            "token".to_owned(),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
        )?;
        let upstream_thread = thread::spawn(move || server.accept().map(drop));

        let mut client = relay_client(&relay)?;
        client.write_all(b"GET / HTTP/1.1\r\nX-Tokamak-Session: token\r\n\r\n")?;
        let mut response = Vec::new();
        client.read_to_end(&mut response)?;
        assert!(response.is_empty());
        assert!(
            upstream_thread
                .join()
                .map_err(|_| "upstream thread panicked")?
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn follows_an_https_server_and_its_certificate() -> Result<(), Box<dyn std::error::Error>> {
        let server = TlsServer::new()?;
        let url = server.url()?;
        let reported = Reported::new()?;
        let mut followed = reported.following()?;
        let mut output = Vec::new();

        let previous = TlsServer::new()?.certificate;
        reported.report(
            "server.json",
            &json!({ "url": url, "certificates": previous }),
        )?;
        followed.follow(&mut output)?;
        reported.report(
            "server.json",
            &json!({ "url": url, "certificates": server.certificate }),
        )?;
        followed.follow(&mut output)?;
        assert_eq!(
            String::from_utf8(output)?,
            format!(
                "Development server moved to {}\n",
                url.trim_end_matches('/')
            )
        );

        let upstream_thread = thread::spawn(move || -> std::io::Result<()> {
            let mut stream = server.accept()?;
            read_head(&mut stream)?;
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\nrestarted")?;
            stream.conn.send_close_notify();
            stream.flush()
        });
        let mut client = relay_client(&reported.relay)?;
        client.write_all(b"GET / HTTP/1.1\r\nX-Tokamak-Session: token\r\n\r\n")?;
        let mut response = String::new();
        client.read_to_string(&mut response)?;
        assert!(response.ends_with("\r\n\r\nrestarted"));
        upstream_thread
            .join()
            .map_err(|_| "upstream thread panicked")??;
        Ok(())
    }

    /// An HTTPS server on any port of 127.0.0.1, with a new self-signed
    /// certificate for localhost.
    struct TlsServer {
        listener: TcpListener,
        config: Arc<rustls::ServerConfig>,
        /// The certificate, as PEM.
        certificate: String,
    }

    impl TlsServer {
        fn new() -> anyhow::Result<Self> {
            let rcgen::CertifiedKey { cert, signing_key } =
                rcgen::generate_simple_self_signed(["localhost".to_owned()])?;
            let key = PrivatePkcs8KeyDer::from(signing_key.serialize_der());
            let config = rustls::ServerConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_single_cert(vec![cert.der().clone()], key.into())?;
            Ok(Self {
                listener: TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?,
                config: Arc::new(config),
                certificate: cert.pem(),
            })
        }

        /// The server's URL, as Vite reports it.
        fn url(&self) -> std::io::Result<String> {
            Ok(format!(
                "https://localhost:{}/",
                self.listener.local_addr()?.port()
            ))
        }

        /// The next connection, once its handshake completes.
        fn accept(&self) -> std::io::Result<StreamOwned<ServerConnection, TcpStream>> {
            let (mut stream, _) = self.listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;
            let mut connection =
                ServerConnection::new(Arc::clone(&self.config)).map_err(std::io::Error::other)?;
            while connection.is_handshaking() {
                connection.complete_io(&mut stream)?;
            }
            Ok(StreamOwned::new(connection, stream))
        }
    }

    fn relay_client(relay: &DevRelay) -> std::io::Result<TcpStream> {
        let client = TcpStream::connect((Ipv4Addr::LOCALHOST, relay.port()))?;
        client.set_read_timeout(Some(Duration::from_secs(5)))?;
        Ok(client)
    }

    /// The bytes of `stream` up to and including the blank line that ends an
    /// HTTP head.
    fn read_head(stream: &mut impl Read) -> std::io::Result<String> {
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            let mut byte = [0_u8];
            stream.read_exact(&mut byte)?;
            head.push(byte[0]);
        }
        String::from_utf8(head).map_err(std::io::Error::other)
    }

    #[test]
    fn says_once_when_the_reported_configuration_changes() -> anyhow::Result<()> {
        let reported = Reported::new()?;
        let mut server = reported.following()?;
        let changed = json!({ "root": "/app", "config": { "name": "Changed" } });
        reported.report("config.json", &changed)?;
        let mut output = Vec::new();

        server.follow(&mut output)?;
        server.follow(&mut output)?;
        assert_eq!(
            String::from_utf8(output)?,
            "The tokamak configuration changed; restart `tok dev` to apply it to the app\n"
        );
        Ok(())
    }
}
