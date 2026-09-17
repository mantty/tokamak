//! Development-session orchestration.

use std::fmt::Write as _;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcessCommand, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tokamak::{WranglerConfig, load_wrangler_config, resolve_wrangler_config_path};
use tokamak_cli::Platform;

use super::devices::PreparedDevice;
use super::{devices, pipeline, variables};

const SERVER_READY_TIMEOUT: Duration = Duration::from_mins(1);
const APP_CONNECTION_TIMEOUT: Duration = Duration::from_mins(1);
const SERVER_POLL_INTERVAL: Duration = Duration::from_millis(100);
#[cfg(any(unix, windows))]
const PROCESS_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const CORE_DEVICE_MAX_RETRIES: usize = 2;
const CORE_DEVICE_RETRY_DELAY: Duration = Duration::from_secs(1);
const RELAY_HEADER_LIMIT: usize = 64 * 1024;

/// Development command inputs collected by the CLI.
pub(crate) struct Request {
    pub(crate) device_id: String,
    pub(crate) project_dir: PathBuf,
    pub(crate) target_pack_dir: Option<PathBuf>,
    pub(crate) tokamak_config_path: PathBuf,
    pub(crate) wrangler_config_path: Option<PathBuf>,
    pub(crate) set: Vec<variables::SetVariable>,
    pub(crate) server: String,
    pub(crate) host_address: Option<String>,
    pub(crate) command: Vec<std::ffi::OsString>,
}

/// Run one development session until the framework process or native app
/// exits.
pub(crate) fn run(request: &Request) -> Result<()> {
    validate_request(request)?;
    let shutdown = ShutdownSignal::start()?;
    let project = fs::canonicalize(&request.project_dir).with_context(|| {
        format!(
            "resolve project directory: {}",
            request.project_dir.display()
        )
    })?;
    let wrangler = load_development_config(&project, request.wrangler_config_path.as_deref())?;
    warn_unsupported_bindings(&wrangler);
    let server = ServerEndpoint::parse(&request.server)?;
    server.ensure_unused()?;
    let device = devices::prepare(&request.device_id)?;
    let session_token = session_token()?;
    let relay_host = relay_host(&device, request.host_address.as_deref())?;
    if device.platform == Platform::Ios && request.host_address.is_none() {
        println!("Using detected host address {relay_host} for physical iOS development");
    }
    let relay = DevRelay::bind(server.clone(), session_token.clone(), relay_host)?;
    let mut framework =
        spawn_framework(&request.command, &project, &relay, &server, &session_token)?;

    if shutdown.requested() {
        stop_process(&mut framework)?;
        return Ok(());
    }

    let result = run_session(&mut DevelopmentSession {
        request,
        device: &device,
        session_token: &session_token,
        relay: &relay,
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

fn load_development_config(project: &Path, explicit: Option<&Path>) -> Result<WranglerConfig> {
    let base = if explicit.is_some() {
        std::env::current_dir()?
    } else {
        project.to_path_buf()
    };
    let path = resolve_wrangler_config_path(&base, explicit)?;
    load_wrangler_config(&path).with_context(|| format!("load Wrangler config {}", path.display()))
}

fn warn_unsupported_bindings(config: &WranglerConfig) {
    if config.bindings.is_empty() {
        return;
    }
    let mut warning =
        "\nWARNING: this app declares bindings that tok dev does not provide.\n\n".to_owned();
    warning.push_str(
        "The host development server will continue. Avoid these bindings when running on tokamak,\n",
    );
    warning.push_str("or guard their use with the appropriate platform or feature flag.\n\n");
    warning.push_str("Unsupported bindings:\n\n");
    for binding in &config.bindings {
        let _ = writeln!(
            &mut warning,
            "  - {} ({}): {}",
            binding.name,
            binding.kind,
            unsupported_binding_reason(&binding.kind)
        );
    }
    eprintln!("{warning}");
}

fn unsupported_binding_reason(kind: &str) -> &'static str {
    match kind {
        "d1_databases" => "tokamak development does not provide D1",
        "kv_namespaces" => "tokamak development does not provide KV",
        "r2_buckets" => "tokamak development does not provide R2",
        "durable_objects" => "tokamak development does not provide Durable Objects",
        "queues" => "tokamak development does not provide Queues",
        "services" => "tokamak development does not provide service bindings",
        "vectorize" => "tokamak development does not provide Vectorize",
        "hyperdrive" => "tokamak development does not provide Hyperdrive",
        "ai" => "tokamak development does not provide Workers AI",
        "browser" => "tokamak development does not provide Browser Rendering",
        "images" => "tokamak development does not provide Images",
        "dispatch_namespaces" => "tokamak development does not provide dispatch namespaces",
        "mtls_certificates" => "tokamak development does not provide mTLS bindings",
        "pipelines" => "tokamak development does not provide Pipelines",
        "rate_limiting" => "tokamak development does not provide rate limiting",
        "secrets_store_secrets" => "tokamak development does not provide Secrets Store",
        "send_email" => "tokamak development does not provide Email Routing",
        "analytics_engine_datasets" => "tokamak development does not provide Analytics Engine",
        _ => "tokamak development does not provide this binding",
    }
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
    device: &'a PreparedDevice,
    session_token: &'a str,
    relay: &'a DevRelay,
    framework: &'a mut Child,
    shutdown: &'a ShutdownSignal,
}

fn run_session(session: &mut DevelopmentSession<'_>) -> Result<()> {
    let server = ServerEndpoint::parse(&session.request.server)?;
    wait_for_server(session.framework, &server, session.shutdown)?;
    if session.shutdown.requested() {
        stop_process(session.framework)?;
        return Ok(());
    }
    println!("Development server is ready at {}", server.display_url());
    let summary = pipeline::run_development(&pipeline::DevelopmentRequest {
        platform: session.device.platform,
        project_dir: session.request.project_dir.clone(),
        target_pack_dir: session.request.target_pack_dir.clone(),
        tokamak_config_path: session.request.tokamak_config_path.clone(),
        wrangler_config_path: session.request.wrangler_config_path.clone(),
        endpoint: session.relay.device_endpoint(),
        session_token: session.session_token.to_owned(),
        device_id: Some(session.device.id.clone()),
        set: session.request.set.clone(),
    })?;
    if session.shutdown.requested() {
        stop_process(session.framework)?;
        return Ok(());
    }
    let mut app = launch_app(&summary, session.device, session.relay.port())?;
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    if let Err(error) = wait_for_app_connection(
        session.framework,
        session.relay,
        &session.shutdown.requested,
        session.device,
        &mut stdout,
        APP_CONNECTION_TIMEOUT,
    ) {
        app.stop();
        return Err(error);
    }
    if session.shutdown.requested() {
        app.stop();
        stop_process(session.framework)?;
        return Ok(());
    }
    supervise(session.framework, &mut app, session.shutdown)
}

fn spawn_framework(
    command: &[std::ffi::OsString],
    project: &Path,
    relay: &DevRelay,
    server: &ServerEndpoint,
    session_token: &str,
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
        .env("TOKAMAK_DEV_SESSION_TOKEN", session_token)
        .env("TOKAMAK_DEV_RELAY_ENDPOINT", relay.device_endpoint())
        .env("TOKAMAK_DEV_SERVER_ENDPOINT", server.display_url());
    configure_process_group(&mut process);
    process.spawn().with_context(|| {
        format!(
            "failed to start development command `{}`",
            program.to_string_lossy()
        )
    })
}

fn wait_for_server(
    child: &mut Child,
    endpoint: &ServerEndpoint,
    shutdown: &ShutdownSignal,
) -> Result<()> {
    let deadline = Instant::now() + SERVER_READY_TIMEOUT;
    loop {
        if shutdown.requested() {
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            bail!("development command exited before its server was ready ({status})");
        }
        if endpoint.connect().is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!(
                "development server did not become ready at {} within {} seconds; pass `--server` with its actual HTTP endpoint",
                endpoint.display_url(),
                SERVER_READY_TIMEOUT.as_secs()
            );
        }
        thread::sleep(SERVER_POLL_INTERVAL);
    }
}

fn wait_for_app_connection(
    framework: &mut Child,
    relay: &DevRelay,
    shutdown_requested: &AtomicBool,
    device: &PreparedDevice,
    output: &mut impl Write,
    timeout: Duration,
) -> Result<()> {
    writeln!(output, "Waiting for the development app to connect...")?;
    output.flush()?;
    let deadline = Instant::now() + timeout;
    loop {
        if shutdown_requested.load(Ordering::Acquire) {
            return Ok(());
        }
        if let Some(status) = framework.try_wait()? {
            bail!("development command exited before the app connected ({status})");
        }
        if relay.app_connected() {
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
                relay.device_endpoint(),
                timeout.as_secs()
            );
        }
        thread::sleep(SERVER_POLL_INTERVAL);
    }
}

fn supervise(
    framework: &mut Child,
    app: &mut LaunchedApp,
    shutdown: &ShutdownSignal,
) -> Result<()> {
    loop {
        if shutdown.requested() {
            app.stop();
            stop_process(framework)?;
            return Ok(());
        }
        if let Some(status) = framework.try_wait()? {
            app.stop();
            stop_process(framework)?;
            if shutdown.requested() {
                return Ok(());
            }
            return status_result("development command", status);
        }
        if app.has_exited()? {
            stop_process(framework)?;
            return Ok(());
        }
        thread::sleep(SERVER_POLL_INTERVAL);
    }
}

fn status_result(label: &str, status: ExitStatus) -> Result<()> {
    if status.success() {
        Ok(())
    } else {
        bail!("{label} exited with status {status}");
    }
}

struct ShutdownSignal {
    requested: Arc<AtomicBool>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl ShutdownSignal {
    fn start() -> Result<Self> {
        let requested = Arc::new(AtomicBool::new(false));
        let (ready_sender, ready_receiver) = std::sync::mpsc::sync_channel(1);
        let (stop_sender, stop_receiver) = tokio::sync::oneshot::channel();
        let watched = Arc::clone(&requested);
        let thread = thread::Builder::new()
            .name("tokamak-dev-signals".to_owned())
            .spawn(move || {
                let runtime = match tokio::runtime::Runtime::new() {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = ready_sender
                            .send(Err(format!("create development signal runtime: {error}")));
                        return;
                    }
                };
                runtime.block_on(wait_for_shutdown(watched, ready_sender, stop_receiver));
            })
            .context("start development signal listener")?;

        let ready = ready_receiver
            .recv()
            .context("wait for development signal listener");
        let ready = match ready {
            Ok(ready) => ready,
            Err(error) => {
                let _ = stop_sender.send(());
                let _ = thread.join();
                return Err(error);
            }
        };
        match ready {
            Ok(()) => Ok(Self {
                requested,
                stop: Some(stop_sender),
                thread: Some(thread),
            }),
            Err(error) => {
                let _ = stop_sender.send(());
                let _ = thread.join();
                bail!(error);
            }
        }
    }

    fn requested(&self) -> bool {
        self.requested.load(Ordering::Acquire)
    }
}

impl Drop for ShutdownSignal {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

async fn wait_for_shutdown(
    requested: Arc<AtomicBool>,
    ready: std::sync::mpsc::SyncSender<Result<(), String>>,
    mut stop: tokio::sync::oneshot::Receiver<()>,
) {
    #[cfg(unix)]
    {
        let mut interrupt =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()) {
                Ok(signal) => signal,
                Err(error) => {
                    let _ = ready.send(Err(format!("install SIGINT listener: {error}")));
                    return;
                }
            };
        let mut terminate =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(signal) => signal,
                Err(error) => {
                    let _ = ready.send(Err(format!("install SIGTERM listener: {error}")));
                    return;
                }
            };
        let mut hangup = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
        {
            Ok(signal) => signal,
            Err(error) => {
                let _ = ready.send(Err(format!("install SIGHUP listener: {error}")));
                return;
            }
        };
        let _ = ready.send(Ok(()));
        loop {
            tokio::select! {
                Some(()) = interrupt.recv() => requested.store(true, Ordering::Release),
                Some(()) = terminate.recv() => requested.store(true, Ordering::Release),
                Some(()) = hangup.recv() => requested.store(true, Ordering::Release),
                _ = &mut stop => break,
            }
        }
    }

    #[cfg(windows)]
    {
        let mut ctrl_c = match tokio::signal::windows::ctrl_c() {
            Ok(signal) => signal,
            Err(error) => {
                let _ = ready.send(Err(format!("install Ctrl-C listener: {error}")));
                return;
            }
        };
        let mut ctrl_break = match tokio::signal::windows::ctrl_break() {
            Ok(signal) => signal,
            Err(error) => {
                let _ = ready.send(Err(format!("install Ctrl-Break listener: {error}")));
                return;
            }
        };
        let mut ctrl_close = match tokio::signal::windows::ctrl_close() {
            Ok(signal) => signal,
            Err(error) => {
                let _ = ready.send(Err(format!("install console-close listener: {error}")));
                return;
            }
        };
        let _ = ready.send(Ok(()));
        loop {
            tokio::select! {
                Some(()) = ctrl_c.recv() => requested.store(true, Ordering::Release),
                Some(()) = ctrl_break.recv() => requested.store(true, Ordering::Release),
                Some(()) = ctrl_close.recv() => requested.store(true, Ordering::Release),
                _ = &mut stop => break,
            }
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = ready.send(Err(
            "development signal handling is unsupported on this host".to_owned(),
        ));
        let _ = stop.await;
    }
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
    let group = format!("-{pid}");
    ProcessCommand::new("kill")
        .args(["-0", &group])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(unix)]
fn send_process_group_signal(pid: u32, signal: &str) {
    let group = format!("-{pid}");
    let flag = format!("-{signal}");
    let _ = ProcessCommand::new("kill")
        .args([flag.as_str(), group.as_str()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
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
    match summary.platform {
        Platform::Macos => launch_macos(summary),
        Platform::Windows => launch_windows(summary),
        Platform::IosSimulator => {
            install_and_launch_ios_simulator(summary, &device.id)?;
            Ok(LaunchedApp::detached())
        }
        Platform::Ios => {
            install_and_launch_ios_device(summary, &device.id)?;
            Ok(LaunchedApp::detached())
        }
        Platform::Android => {
            install_and_launch_android(summary, &device.id, relay_port)?;
            Ok(LaunchedApp::detached())
        }
    }
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
    Ok(LaunchedApp::process(process))
}

fn install_and_launch_ios_simulator(
    summary: &pipeline::DevelopmentSummary,
    device_id: &str,
) -> Result<()> {
    run_platform_command(
        "xcrun",
        ["simctl", "install", device_id]
            .into_iter()
            .map(String::from)
            .chain(std::iter::once(
                summary.bundle_dir.to_string_lossy().into_owned(),
            )),
        "install the app in the iOS Simulator",
    )?;
    run_platform_command(
        "xcrun",
        ["simctl", "launch", device_id, &summary.identifier]
            .into_iter()
            .map(String::from),
        "launch the app in the iOS Simulator",
    )?;
    #[cfg(target_os = "macos")]
    if let Err(error) = devices::open_ios_simulator_ui(device_id) {
        eprintln!("warning: could not open the iOS Simulator UI: {error:#}");
    }
    Ok(())
}

fn install_and_launch_ios_device(
    summary: &pipeline::DevelopmentSummary,
    device_id: &str,
) -> Result<()> {
    run_platform_command(
        "xcrun",
        [
            "devicectl",
            "device",
            "install",
            "app",
            "--device",
            device_id,
        ]
        .into_iter()
        .map(String::from)
        .chain(std::iter::once(
            summary.bundle_dir.to_string_lossy().into_owned(),
        )),
        "install the app on the iOS device",
    )?;
    run_platform_command(
        "xcrun",
        [
            "devicectl",
            "device",
            "process",
            "launch",
            "--device",
            device_id,
            &summary.identifier,
        ]
        .into_iter()
        .map(String::from),
        "launch the app on the iOS device",
    )
}

fn install_and_launch_android(
    summary: &pipeline::DevelopmentSummary,
    device_id: &str,
    relay_port: u16,
) -> Result<()> {
    run_platform_command(
        "adb",
        [
            "-s",
            device_id,
            "reverse",
            &format!("tcp:{relay_port}"),
            &format!("tcp:{relay_port}"),
        ]
        .into_iter()
        .map(String::from),
        "forward the development relay to Android",
    )?;
    run_platform_command(
        "adb",
        ["-s", device_id, "install", "-r"]
            .into_iter()
            .map(String::from)
            .chain(std::iter::once(
                summary.bundle_dir.to_string_lossy().into_owned(),
            )),
        "install the app on Android",
    )?;
    run_platform_command(
        "adb",
        [
            "-s",
            device_id,
            "shell",
            "monkey",
            "-p",
            &summary.identifier,
            "1",
        ]
        .into_iter()
        .map(String::from),
        "launch the app on Android",
    )
}

fn run_platform_command(
    program: &str,
    arguments: impl IntoIterator<Item = String>,
    action: &str,
) -> Result<()> {
    let arguments = arguments.into_iter().collect::<Vec<_>>();
    let mut retries = 0;
    loop {
        let output = ProcessCommand::new(program)
            .args(&arguments)
            .output()
            .with_context(|| format!("{action}: failed to start {program}"))?;
        if output.status.success() {
            return Ok(());
        }
        let transient_detail = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if retries < CORE_DEVICE_MAX_RETRIES
            && is_transient_devicectl_error(program, &arguments, &transient_detail)
        {
            retries += 1;
            eprintln!(
                "{action} encountered a transient Apple device connection error; retrying ({retries}/{CORE_DEVICE_MAX_RETRIES})"
            );
            thread::sleep(CORE_DEVICE_RETRY_DELAY);
            continue;
        }
        let detail = String::from_utf8_lossy(&output.stderr);
        let detail = detail.trim();
        if detail.is_empty() {
            bail!("{action} failed with status {}", output.status);
        }
        bail!("{action} failed: {detail}");
    }
}

fn is_transient_devicectl_error(program: &str, arguments: &[String], detail: &str) -> bool {
    if program != "xcrun" || arguments.first().map(String::as_str) != Some("devicectl") {
        return false;
    }
    let detail = detail.to_ascii_lowercase();
    [
        "connection was invalidated",
        "connection reset by peer",
        "could not be established",
        "controlchannelconnectionerror",
        "timed out waiting for coredeviceservice",
        "transport error",
        "xpcerror",
    ]
    .iter()
    .any(|fragment| detail.contains(fragment))
}

fn session_token() -> Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).context("generate development session token")?;
    let mut token = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut token, "{byte:02x}");
    }
    Ok(token)
}

struct LaunchedApp {
    process: Option<Child>,
}

impl LaunchedApp {
    fn process(process: Child) -> Self {
        Self {
            process: Some(process),
        }
    }

    const fn detached() -> Self {
        Self { process: None }
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

#[derive(Clone, Debug)]
struct ServerEndpoint {
    url: String,
    authority: String,
    addresses: Vec<SocketAddr>,
}

impl ServerEndpoint {
    fn parse(value: &str) -> Result<Self> {
        let authority = value
            .strip_prefix("http://")
            .ok_or_else(|| anyhow::anyhow!("development server must use an http:// URL"))?;
        if authority.is_empty()
            || authority.contains('/')
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
        Ok(Self {
            url: format!("http://{authority}"),
            authority: authority.to_owned(),
            addresses,
        })
    }

    fn connect(&self) -> io::Result<TcpStream> {
        let mut last_error = None;
        for address in &self.addresses {
            match TcpStream::connect_timeout(address, Duration::from_secs(1)) {
                Ok(stream) => return Ok(stream),
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or_else(|| io::Error::other("development server did not resolve")))
    }

    fn ensure_unused(&self) -> Result<()> {
        if self.connect().is_ok() {
            bail!(
                "development server endpoint {} is already accepting connections; stop the existing server or use a different `--server` endpoint",
                self.display_url()
            );
        }
        Ok(())
    }

    fn display_url(&self) -> &str {
        &self.url
    }
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
    app_connected: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl DevRelay {
    fn bind(server: ServerEndpoint, session_token: String, host: IpAddr) -> Result<Self> {
        let listener = TcpListener::bind((host, 0)).context("bind development relay")?;
        listener
            .set_nonblocking(true)
            .context("configure development relay")?;
        let address = listener.local_addr()?;
        let app_connected = Arc::new(AtomicBool::new(false));
        let connected = Arc::clone(&app_connected);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("tokamak-dev-relay".to_owned())
            .spawn(move || relay_loop(listener, server, session_token, connected, stopped))
            .context("start development relay")?;
        Ok(Self {
            address,
            advertised_host: host,
            app_connected,
            stop,
            thread: Some(thread),
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
}

impl Drop for DevRelay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
fn relay_loop(
    listener: TcpListener,
    server: ServerEndpoint,
    session_token: String,
    app_connected: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => {
                let server = server.clone();
                let session_token = session_token.clone();
                let app_connected = Arc::clone(&app_connected);
                let _ = thread::Builder::new()
                    .name("tokamak-dev-relay-connection".to_owned())
                    .spawn(move || {
                        if let Err(error) =
                            relay_connection(stream, &server, &session_token, &app_connected)
                        {
                            eprintln!("tokamak development relay connection failed: {error}");
                        }
                    });
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(SERVER_POLL_INTERVAL);
            }
            Err(_) => break,
        }
    }
}

fn relay_connection(
    mut downstream: TcpStream,
    server: &ServerEndpoint,
    session_token: &str,
    app_connected: &AtomicBool,
) -> io::Result<()> {
    downstream.set_nonblocking(false)?;
    let initial = read_headers(&mut downstream)?;
    if !authorized(&initial, session_token) {
        let _ = downstream.write_all(
            b"HTTP/1.1 401 Unauthorized\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
        );
        return Ok(());
    }
    let mut upstream = server.connect()?;
    upstream.set_nonblocking(false)?;
    let request = rewrite_request(&initial, &server.authority)?;
    upstream.write_all(&request)?;
    upstream.flush()?;
    app_connected.store(true, Ordering::Release);

    let mut downstream_read = downstream.try_clone()?;
    let mut upstream_write = upstream.try_clone()?;
    let forward = thread::spawn(move || {
        let result = io::copy(&mut downstream_read, &mut upstream_write);
        let _ = upstream_write.shutdown(Shutdown::Write);
        result
    });
    let upstream_result = io::copy(&mut upstream, &mut downstream);
    if let Err(error) = upstream_result {
        eprintln!("tokamak relay upstream-to-downstream failed: {error}");
    }
    let _ = downstream.shutdown(Shutdown::Write);
    match forward.join() {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => eprintln!("tokamak relay downstream-to-upstream failed: {error}"),
        Err(_) => eprintln!("tokamak relay downstream-to-upstream thread failed"),
    }
    Ok(())
}

fn read_headers(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(4096);
    let mut chunk = [0_u8; 4096];
    loop {
        let count = stream.read(&mut chunk)?;
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
            if trim_ascii(value) != expected.as_bytes() {
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
    let mut rewritten = Vec::with_capacity(request.len());
    for (index, line) in request[..end].split(|byte| *byte == b'\n').enumerate() {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if index > 0
            && let Some((name, _)) = header_parts(line)
            && name.eq_ignore_ascii_case(b"x-tokamak-session")
        {
            continue;
        }
        if index > 0
            && let Some((name, _)) = header_parts(line)
            && name.eq_ignore_ascii_case(b"host")
        {
            rewritten.extend_from_slice(b"Host: ");
            rewritten.extend_from_slice(authority.as_bytes());
        } else {
            rewritten.extend_from_slice(line);
        }
        rewritten.extend_from_slice(b"\r\n");
    }
    rewritten.extend_from_slice(b"\r\n");
    rewritten.extend_from_slice(&request[end + 4..]);
    Ok(rewritten)
}

fn trim_ascii(value: &[u8]) -> &[u8] {
    let mut start = 0;
    let mut end = value.len();
    while start < end && value[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && value[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &value[start..end]
}

fn header_parts(line: &[u8]) -> Option<(&[u8], &[u8])> {
    let separator = line.iter().position(|byte| *byte == b':')?;
    Some((&line[..separator], &line[separator + 1..]))
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{IpAddr, Ipv4Addr, TcpListener, TcpStream};
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::Duration;

    use super::{
        DevRelay, PreparedDevice, ServerEndpoint, authorized, is_transient_devicectl_error,
        parse_authority, relay_host, rewrite_request, usable_ipv4_address, wait_for_app_connection,
    };
    use tokamak_cli::Platform;

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
    fn rejects_an_already_listening_development_server() -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let endpoint = ServerEndpoint::parse(&format!("http://{}", listener.local_addr()?))?;

        assert!(
            endpoint
                .ensure_unused()
                .is_err_and(|error| error.to_string().contains("already accepting connections"))
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn waits_for_app_connection_before_reporting_ready() -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let server = ServerEndpoint::parse(&format!("http://{}", listener.local_addr()?))?;
        let relay = DevRelay::bind(server, "token".to_owned(), IpAddr::V4(Ipv4Addr::LOCALHOST))?;
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
            &relay,
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
            &relay,
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
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let server = ServerEndpoint::parse(&format!("http://{}", listener.local_addr()?))?;
        let relay = DevRelay::bind(server, "token".to_owned(), IpAddr::V4(Ipv4Addr::LOCALHOST))?;
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
            &relay,
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
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let server = ServerEndpoint::parse(&format!("http://{}", listener.local_addr()?))?;
        let relay = DevRelay::bind(server, "token".to_owned(), IpAddr::V4(Ipv4Addr::LOCALHOST))?;
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
            &relay,
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
            Some(b"GET / HTTP/1.1\r\nHost: 127.0.0.1:5173\r\n\r\n".to_vec())
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
    fn retries_only_transient_devicectl_errors() {
        let arguments = vec!["devicectl".to_owned(), "device".to_owned()];
        assert!(is_transient_devicectl_error(
            "xcrun",
            &arguments,
            "Connection reset by peer"
        ));
        assert!(is_transient_devicectl_error(
            "xcrun",
            &arguments,
            "CoreDevice.ControlChannelConnectionError"
        ));
        assert!(!is_transient_devicectl_error(
            "xcrun",
            &arguments,
            "The executable contains an invalid signature"
        ));
        assert!(!is_transient_devicectl_error(
            "adb",
            &arguments,
            "Connection reset by peer"
        ));
    }

    #[test]
    fn relays_an_authenticated_http_request() -> Result<(), Box<dyn std::error::Error>> {
        let upstream = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let upstream_endpoint = format!("http://{}", upstream.local_addr()?);
        let endpoint = ServerEndpoint::parse(&upstream_endpoint)?;
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
}
