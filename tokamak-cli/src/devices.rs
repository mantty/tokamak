//! Development target discovery, preparation, and app installation.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Output, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use tokamak_cli::Platform;

const MANAGED_IOS_NAME: &str = "tokamak iPhone";
const MANAGED_ANDROID_NAME: &str = "tokamak-managed";
const ANDROID_BOOT_TIMEOUT: usize = 120;
const CORE_DEVICE_MAX_RETRIES: usize = 2;
const CORE_DEVICE_RETRY_DELAY: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, Eq, PartialEq)]
struct Device {
    id: String,
    kind: String,
    platform: Platform,
    group: DeviceGroup,
    status: DeviceStatus,
}

/// Where a device lists: the host first, then managed targets, physical
/// devices, and other virtual devices.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum DeviceGroup {
    Host,
    Managed,
    Physical,
    Virtual,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum DeviceStatus {
    Available,
    Blocked(String),
}

/// The concrete device selected after preparation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PreparedDevice {
    /// Native identifier used by the platform tooling.
    pub(super) id: String,
    /// Human-readable target type.
    pub(super) kind: String,
    /// Platform family used to build and launch the development app.
    pub(super) platform: Platform,
}

impl DeviceStatus {
    fn display(&self) -> String {
        match self {
            Self::Available => "available".to_owned(),
            Self::Blocked(reason) => format!("blocked: {reason}"),
        }
    }
}

/// Print concrete and provisionable development targets.
pub(super) fn list() {
    print!("{}", render_devices(&discover_devices()));
}

/// Prepare the selected target and return its concrete native identifier.
pub(super) fn prepare(selector: &str) -> Result<PreparedDevice> {
    if selector.trim().is_empty() {
        bail!("device ID cannot be empty");
    }

    if let Some(device) = prepare_host(selector)? {
        return Ok(device);
    }
    match selector {
        "ios" => return prepare_managed_ios(),
        "android" => return prepare_managed_android(),
        _ => {}
    }
    let adb = run_android_tool("adb", "platform-tools", &["devices", "-l"]);
    if let Some(device) = prepare_android_device(selector, &adb)? {
        return Ok(device);
    }
    if let Some(device) = prepare_ios_simulator(selector)? {
        return Ok(device);
    }
    if let Some(device) = prepare_ios_physical(selector)? {
        return Ok(device);
    }

    // A failed Android query matters only when no other platform has the device.
    if adb.available && !adb.success {
        bail!(
            "device `{selector}` was not found; unable to query Android devices: {}",
            tool_failure(&adb, "adb devices failed")
        );
    }
    bail!("device `{selector}` was not found; run `tok devices` to list available targets");
}

fn prepare_host(selector: &str) -> Result<Option<PreparedDevice>> {
    if !matches!(selector, "macos" | "windows" | "linux") {
        return Ok(None);
    }
    match host_device() {
        Some(host) if host.id == selector => prepared_device(host).map(Some),
        Some(host) => bail!(
            "device `{selector}` is not available on this host; local host is `{}`",
            host.id
        ),
        None => bail!("device `{selector}` is not available; tokamak has no desktop target here"),
    }
}

fn prepared_device(device: Device) -> Result<PreparedDevice> {
    match device.status {
        DeviceStatus::Available => Ok(PreparedDevice {
            id: device.id,
            kind: device.kind,
            platform: device.platform,
        }),
        DeviceStatus::Blocked(reason) => bail!("device `{}` is blocked: {reason}", device.id),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct IosSimulatorTarget {
    id: String,
    name: String,
    available: bool,
    state: String,
    has_been_booted: bool,
    availability_error: Option<String>,
}

impl IosSimulatorTarget {
    fn kind(&self) -> String {
        format!("{} / iOS Simulator", self.name)
    }

    fn unavailable_reason(&self) -> &str {
        self.availability_error
            .as_deref()
            .filter(|reason| !reason.is_empty())
            .unwrap_or("the Simulator runtime is unavailable")
    }

    fn prepared_device(&self) -> PreparedDevice {
        PreparedDevice {
            id: self.id.clone(),
            kind: self.kind(),
            platform: Platform::IosSimulator,
        }
    }
}

#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Debug, Eq, PartialEq)]
enum IosSimulatorUi {
    StandaloneSimulator(PathBuf),
    DeviceHub(PathBuf),
}

#[cfg(target_os = "macos")]
impl IosSimulatorUi {
    fn path(&self) -> &Path {
        match self {
            Self::StandaloneSimulator(path) | Self::DeviceHub(path) => path,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::StandaloneSimulator(_) => "Simulator.app",
            Self::DeviceHub(_) => "Device Hub",
        }
    }
}

#[cfg(any(target_os = "macos", test))]
fn ios_simulator_ui_for_developer_dir(developer_dir: &Path) -> Option<IosSimulatorUi> {
    let standalone = developer_dir.join("Applications/Simulator.app");
    if standalone.is_dir() {
        return Some(IosSimulatorUi::StandaloneSimulator(standalone));
    }

    let device_hub = developer_dir.parent()?.join("Applications/DeviceHub.app");
    device_hub
        .is_dir()
        .then_some(IosSimulatorUi::DeviceHub(device_hub))
}

#[cfg(target_os = "macos")]
fn open_ios_simulator_ui(device_id: &str) -> Result<()> {
    let simctl = run_tool("xcrun", &["--find", "simctl"]);
    if !simctl.available {
        bail!("Xcode command-line tools are not installed");
    }
    if !simctl.success {
        bail!(
            "could not locate the active Xcode Simulator tools: {}",
            tool_failure(&simctl, "xcrun --find simctl failed")
        );
    }

    let simctl_path = Path::new(simctl.stdout.trim());
    let Some(developer_dir) = simctl_path.ancestors().nth(3) else {
        bail!("could not determine Xcode's developer directory from simctl's path");
    };
    let Some(ui) = ios_simulator_ui_for_developer_dir(developer_dir) else {
        bail!("the active Xcode installation contains neither Simulator.app nor DeviceHub.app");
    };

    let mut command = ProcessCommand::new("open");
    command.arg("-a").arg(ui.path());
    if matches!(ui, IosSimulatorUi::StandaloneSimulator(_)) {
        command
            .args(["--args", "-CurrentDeviceUDID"])
            .arg(device_id);
    }
    let output = command
        .output()
        .with_context(|| format!("could not open {}", ui.name()))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        let detail = detail.trim();
        if detail.is_empty() {
            bail!("could not open {} (status {})", ui.name(), output.status);
        }
        bail!("could not open {}: {detail}", ui.name());
    }
    Ok(())
}

fn prepare_ios_simulator(selector: &str) -> Result<Option<PreparedDevice>> {
    if !cfg!(target_os = "macos") {
        return Ok(None);
    }
    let simulators = run_tool("xcrun", &["simctl", "list", "devices", "--json"]);
    if !simulators.available {
        bail!("Xcode command-line tools are not installed");
    }
    if !simulators.success {
        bail!("Xcode Simulator services are unavailable");
    }
    let Some(targets) = parse_ios_simulator_targets(&simulators.stdout) else {
        bail!("unable to read iOS Simulator devices");
    };
    let Some(target) = targets.into_iter().find(|target| target.id == selector) else {
        return Ok(None);
    };
    if !target.available {
        bail!(
            "iOS Simulator `{selector}` is unavailable: {}",
            target.unavailable_reason()
        );
    }
    boot_ios_simulator(&target)?;
    Ok(Some(target.prepared_device()))
}

fn prepare_managed_ios() -> Result<PreparedDevice> {
    if !cfg!(target_os = "macos") {
        bail!("iOS Simulator requires macOS and Xcode");
    }

    let runtimes = run_tool("xcrun", &["simctl", "list", "runtimes", "--json"]);
    if !runtimes.available {
        bail!("Xcode command-line tools are not installed");
    }
    if !runtimes.success {
        bail!("Xcode Simulator services are unavailable");
    }
    let Some(runtime) = latest_ios_runtime(&runtimes.stdout) else {
        bail!("no available iOS Simulator runtime is installed");
    };

    let simulators = run_tool("xcrun", &["simctl", "list", "devices", "--json"]);
    if !simulators.success {
        bail!("Xcode Simulator services are unavailable");
    }
    let Some(targets) = parse_ios_simulator_targets(&simulators.stdout) else {
        bail!("unable to read iOS Simulator devices");
    };
    if let Some(target) = targets
        .iter()
        .find(|target| target.name == MANAGED_IOS_NAME && target.available)
    {
        boot_ios_simulator(target)?;
        return Ok(target.prepared_device());
    }
    if let Some(target) = targets
        .iter()
        .find(|target| target.name == MANAGED_IOS_NAME)
    {
        bail!(
            "managed iOS Simulator is unavailable: {}",
            target.unavailable_reason()
        );
    }

    let device_types = run_tool("xcrun", &["simctl", "list", "devicetypes", "--json"]);
    if !device_types.success {
        bail!("unable to read iOS Simulator device types");
    }
    let Some(device_type) = default_ios_device_type(&device_types.stdout) else {
        bail!("no iPhone Simulator device type is installed");
    };
    let created = run_tool(
        "xcrun",
        &["simctl", "create", MANAGED_IOS_NAME, &device_type, &runtime],
    );
    if !created.success {
        bail!(
            "could not create managed iOS Simulator: {}",
            tool_failure(&created, "simctl create failed")
        );
    }
    let id = created.stdout.trim();
    if id.is_empty() {
        bail!("simctl create returned no simulator identifier");
    }
    let target = IosSimulatorTarget {
        id: id.to_owned(),
        name: MANAGED_IOS_NAME.to_owned(),
        available: true,
        state: "Shutdown".to_owned(),
        has_been_booted: false,
        availability_error: None,
    };
    boot_ios_simulator(&target)?;
    Ok(target.prepared_device())
}

fn boot_ios_simulator(target: &IosSimulatorTarget) -> Result<()> {
    if !target.state.eq_ignore_ascii_case("booted") {
        let boot = run_tool("xcrun", &["simctl", "boot", &target.id]);
        if !boot.success {
            bail!(
                "could not boot iOS Simulator `{}`: {}",
                target.id,
                tool_failure(&boot, "simctl boot failed")
            );
        }
    }
    let status = run_tool("xcrun", &["simctl", "bootstatus", &target.id, "-b"]);
    if !status.success {
        bail!(
            "iOS Simulator `{}` did not become ready: {}",
            target.id,
            tool_failure(&status, "simctl bootstatus failed")
        );
    }
    Ok(())
}

fn prepare_ios_physical(selector: &str) -> Result<Option<PreparedDevice>> {
    if !cfg!(target_os = "macos") {
        return Ok(None);
    }
    discover_ios_physical_devices()
        .into_iter()
        .find(|device| device.id == selector)
        .map(prepared_device)
        .transpose()
}

fn prepare_managed_android() -> Result<PreparedDevice> {
    let emulator = run_android_tool("emulator", "emulator", &["-list-avds"]);
    if !emulator.available {
        bail!("Android emulator tools are not installed");
    }
    if !emulator.success {
        bail!("unable to query Android emulator profiles");
    }
    let avds = parse_android_avds(&emulator.stdout);
    if !avds.iter().any(|name| name == MANAGED_ANDROID_NAME) {
        create_managed_android_avd()?;
    }
    prepare_android_avd(MANAGED_ANDROID_NAME)
}

fn create_managed_android_avd() -> Result<()> {
    let sdkmanager = run_android_tool("sdkmanager", "cmdline-tools", &["--list"]);
    if !sdkmanager.available {
        bail!("Android command-line tools are not installed");
    }
    if !sdkmanager.success {
        bail!(
            "unable to query installed Android system images: {}",
            tool_failure(&sdkmanager, "sdkmanager failed")
        );
    }
    let Some(system_image) = select_android_system_image(&sdkmanager.stdout) else {
        bail!(
            "no Android system image is installed; install one with sdkmanager before using `tok dev android`"
        );
    };
    let Some(avdmanager) = android_tool_program("avdmanager", "cmdline-tools") else {
        bail!("Android command-line tools are not installed");
    };
    let created = run_tool_with_input(
        &avdmanager,
        &[
            "create",
            "avd",
            "--force",
            "--name",
            MANAGED_ANDROID_NAME,
            "--package",
            &system_image,
        ],
        "no\n",
    )?;
    if !created.success {
        bail!(
            "could not create managed Android emulator: {}",
            tool_failure(&created, "avdmanager create failed")
        );
    }
    Ok(())
}

fn prepare_android_device(selector: &str, adb: &ToolOutput) -> Result<Option<PreparedDevice>> {
    let emulator = run_android_tool("emulator", "emulator", &["-list-avds"]);
    if emulator.success
        && parse_android_avds(&emulator.stdout)
            .iter()
            .any(|name| name == selector)
    {
        return prepare_android_avd(selector).map(Some);
    }

    if !adb.success {
        return Ok(None);
    }
    parse_android_adb_devices(&adb.stdout)
        .into_iter()
        .find(|device| device.id == selector)
        .map(prepared_device)
        .transpose()
}

fn prepare_android_avd(avd_name: &str) -> Result<PreparedDevice> {
    let adb = run_android_tool("adb", "platform-tools", &["devices", "-l"]);
    if !adb.available {
        bail!("Android platform-tools are not installed");
    }
    if !adb.success {
        bail!(
            "unable to query Android devices: {}",
            tool_failure(&adb, "adb devices failed")
        );
    }
    if let Some(serial) = find_android_avd(&adb.stdout, avd_name) {
        if android_boot_completed(&serial) {
            return Ok(android_avd_device(serial, avd_name));
        }
    } else {
        let Some(emulator) = android_tool_program("emulator", "emulator") else {
            bail!("Android emulator tools are not installed");
        };
        ProcessCommand::new(emulator)
            .args(["-avd", avd_name])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
    }

    let Some(serial) = wait_for_android_avd(avd_name) else {
        bail!("Android emulator `{avd_name}` did not become ready within 120 seconds");
    };
    Ok(android_avd_device(serial, avd_name))
}

fn android_avd_device(serial: String, avd_name: &str) -> PreparedDevice {
    PreparedDevice {
        id: serial,
        kind: format!("{avd_name} / Android emulator (AVD)"),
        platform: Platform::Android,
    }
}

fn find_android_avd(source: &str, avd_name: &str) -> Option<String> {
    parse_android_adb_devices(source)
        .into_iter()
        .filter(|device| {
            device.id.starts_with("emulator-") && device.status == DeviceStatus::Available
        })
        .find(|device| android_avd_name(&device.id).is_some_and(|name| name == avd_name))
        .map(|device| device.id)
}

fn wait_for_android_avd(avd_name: &str) -> Option<String> {
    for _ in 0..ANDROID_BOOT_TIMEOUT {
        let adb = run_android_tool("adb", "platform-tools", &["devices", "-l"]);
        if adb.success
            && let Some(serial) = find_android_avd(&adb.stdout, avd_name)
            && android_boot_completed(&serial)
        {
            return Some(serial);
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    None
}

fn android_boot_completed(serial: &str) -> bool {
    let boot = run_android_tool(
        "adb",
        "platform-tools",
        &["-s", serial, "shell", "getprop", "sys.boot_completed"],
    );
    boot.success && boot.stdout.trim() == "1"
}

fn android_avd_name(serial: &str) -> Option<String> {
    let output = run_android_tool(
        "adb",
        "platform-tools",
        &["-s", serial, "shell", "getprop", "ro.boot.qemu.avd_name"],
    );
    if !output.success {
        return None;
    }
    let name = output.stdout.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// Install the app `bundle` in the iOS Simulator `device_id`, launch the app
/// `identifier`, and show the Simulator.
pub(super) fn install_and_launch_ios_simulator(
    device_id: &str,
    bundle: &Path,
    identifier: &str,
) -> Result<()> {
    let bundle = bundle.to_string_lossy();
    run_action(
        "xcrun",
        &["simctl", "install", device_id, &bundle],
        "install the app in the iOS Simulator",
    )?;
    run_action(
        "xcrun",
        &["simctl", "launch", device_id, identifier],
        "launch the app in the iOS Simulator",
    )?;
    #[cfg(target_os = "macos")]
    if let Err(error) = open_ios_simulator_ui(device_id) {
        eprintln!("warning: could not open the iOS Simulator UI: {error:#}");
    }
    Ok(())
}

/// Install the app `bundle` on the iOS device `device_id` and launch the app
/// `identifier`.
pub(super) fn install_and_launch_ios_device(
    device_id: &str,
    bundle: &Path,
    identifier: &str,
) -> Result<()> {
    let bundle = bundle.to_string_lossy();
    run_devicectl(
        &["device", "install", "app", "--device", device_id, &bundle],
        "install the app on the iOS device",
    )?;
    run_devicectl(
        &[
            "device", "process", "launch", "--device", device_id, identifier,
        ],
        "launch the app on the iOS device",
    )
}

/// Forward `relay_port` to the Android device `device_id`, install the app
/// `bundle`, and launch the app `identifier`.
pub(super) fn install_and_launch_android(
    device_id: &str,
    bundle: &Path,
    identifier: &str,
    relay_port: u16,
) -> Result<()> {
    let Some(adb) = android_tool_program("adb", "platform-tools") else {
        bail!("Android platform-tools are not installed");
    };
    let relay = format!("tcp:{relay_port}");
    let bundle = bundle.to_string_lossy();
    run_action(
        &adb,
        &["-s", device_id, "reverse", &relay, &relay],
        "forward the development relay to Android",
    )?;
    run_action(
        &adb,
        &["-s", device_id, "install", "-r", &bundle],
        "install the app on Android",
    )?;
    // Monkey fails to send system key events on devices without hardware
    // keys, such as emulators.
    run_action(
        &adb,
        &[
            "-s",
            device_id,
            "shell",
            "monkey",
            "--pct-syskeys",
            "0",
            "-p",
            identifier,
            "1",
        ],
        "launch the app on Android",
    )
}

/// Run `devicectl` with `arguments` to `action`, retrying transient Apple
/// device connection errors.
fn run_devicectl(arguments: &[&str], action: &str) -> Result<()> {
    let arguments = [&["devicectl"], arguments].concat();
    let mut retries = 0;
    loop {
        let output = action_output("xcrun", &arguments, action)?;
        let detail = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if output.status.success()
            || retries == CORE_DEVICE_MAX_RETRIES
            || !is_transient_devicectl_error(&detail)
        {
            return action_result(&output, action);
        }
        retries += 1;
        eprintln!(
            "{action} encountered a transient Apple device connection error; retrying ({retries}/{CORE_DEVICE_MAX_RETRIES})"
        );
        std::thread::sleep(CORE_DEVICE_RETRY_DELAY);
    }
}

fn is_transient_devicectl_error(detail: &str) -> bool {
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

fn discover_devices() -> Vec<Device> {
    let mut devices = Vec::from_iter(host_device());
    devices.extend(discover_ios_devices());
    devices.extend(discover_android_devices());
    order_devices(&mut devices);
    devices
}

fn order_devices(devices: &mut [Device]) {
    devices.sort_by_key(|device| device.group);
}

/// This host's desktop, when tokamak builds desktop apps for it.
fn host_device() -> Option<Device> {
    let platform = if cfg!(target_os = "macos") {
        Platform::Macos
    } else if cfg!(target_os = "windows") {
        Platform::Windows
    } else {
        return None;
    };
    Some(Device {
        id: platform.directory_name().to_owned(),
        kind: format!("{} desktop", platform.display_name()),
        platform,
        group: DeviceGroup::Host,
        status: DeviceStatus::Available,
    })
}

fn discover_ios_devices() -> Vec<Device> {
    let managed = |status| Device {
        id: "ios".to_owned(),
        kind: "managed iOS Simulator".to_owned(),
        platform: Platform::IosSimulator,
        group: DeviceGroup::Managed,
        status,
    };
    if !cfg!(target_os = "macos") {
        return vec![managed(DeviceStatus::Blocked(
            "iOS Simulator requires macOS and Xcode".to_owned(),
        ))];
    }

    let mut devices = match discover_ios_simulators() {
        Ok(simulators) => [managed(DeviceStatus::Available)]
            .into_iter()
            .chain(simulators)
            .collect(),
        Err(reason) => vec![managed(DeviceStatus::Blocked(reason.to_owned()))],
    };
    devices.extend(discover_ios_physical_devices());
    devices
}

/// Available simulators that have booted, or why the managed iOS Simulator is
/// blocked.
fn discover_ios_simulators() -> Result<Vec<Device>, &'static str> {
    let runtimes = run_tool("xcrun", &["simctl", "list", "runtimes", "--json"]);
    if !runtimes.available {
        return Err("Xcode command-line tools are not installed");
    }
    if !runtimes.success {
        return Err("Xcode Simulator services are unavailable");
    }
    if latest_ios_runtime(&runtimes.stdout).is_none() {
        return Err("no iOS Simulator runtime is installed");
    }

    let simulators = run_tool("xcrun", &["simctl", "list", "devices", "--json"]);
    if !simulators.success {
        return Err("Xcode Simulator services are unavailable");
    }
    parse_ios_simulator_devices(&simulators.stdout).ok_or("unable to read iOS Simulator devices")
}

fn discover_ios_physical_devices() -> Vec<Device> {
    let devicectl = run_tool(
        "xcrun",
        &[
            "devicectl",
            "list",
            "devices",
            "--json-output",
            "-",
            "--timeout",
            "3",
        ],
    );
    if devicectl.success
        && let Some(devices) = parse_devicectl_devices(&devicectl.stdout)
        && !devices.is_empty()
    {
        return devices;
    }

    let xctrace = run_tool("xcrun", &["xctrace", "list", "devices"]);
    if xctrace.success {
        parse_xctrace_devices(&xctrace.stdout)
    } else {
        Vec::new()
    }
}

fn discover_android_devices() -> Vec<Device> {
    let adb = run_android_tool("adb", "platform-tools", &["devices", "-l"]);
    let emulator = run_android_tool("emulator", "emulator", &["-list-avds"]);
    let avds = if emulator.success {
        parse_android_avds(&emulator.stdout)
    } else {
        Vec::new()
    };

    let alias_status = if !adb.available {
        DeviceStatus::Blocked("Android platform-tools are not installed".to_owned())
    } else if !emulator.available {
        DeviceStatus::Blocked("Android emulator tools are not installed".to_owned())
    } else if !emulator.success {
        DeviceStatus::Blocked("unable to query Android emulator profiles".to_owned())
    } else if avds.is_empty() {
        DeviceStatus::Blocked("no Android emulator profile is installed".to_owned())
    } else {
        DeviceStatus::Available
    };

    let mut devices = vec![Device {
        id: "android".to_owned(),
        kind: "managed Android emulator".to_owned(),
        platform: Platform::Android,
        group: DeviceGroup::Managed,
        status: alias_status.clone(),
    }];
    devices.extend(avds.into_iter().map(|id| Device {
        id,
        kind: "Android emulator (AVD)".to_owned(),
        platform: Platform::Android,
        group: DeviceGroup::Virtual,
        status: alias_status.clone(),
    }));
    if adb.success {
        devices.extend(parse_android_adb_devices(&adb.stdout));
    }
    devices
}

#[derive(Clone, Debug, Default)]
struct ToolOutput {
    available: bool,
    success: bool,
    stdout: String,
    stderr: String,
}

impl From<Output> for ToolOutput {
    fn from(output: Output) -> Self {
        Self {
            available: true,
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }
}

fn run_tool(program: &str, arguments: &[&str]) -> ToolOutput {
    ProcessCommand::new(program)
        .args(arguments)
        .output()
        .map_or_else(|_| ToolOutput::default(), ToolOutput::from)
}

fn run_tool_with_input(program: &str, arguments: &[&str], input: &str) -> Result<ToolOutput> {
    let mut child = ProcessCommand::new(program)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input.as_bytes())?;
    }
    Ok(child.wait_with_output()?.into())
}

fn run_action(program: &str, arguments: &[&str], action: &str) -> Result<()> {
    action_result(&action_output(program, arguments, action)?, action)
}

fn action_output(program: &str, arguments: &[&str], action: &str) -> Result<Output> {
    ProcessCommand::new(program)
        .args(arguments)
        .output()
        .with_context(|| format!("{action}: failed to start {program}"))
}

/// Fail with `output`'s error output, or else its status, unless it succeeded.
fn action_result(output: &Output, action: &str) -> Result<()> {
    let detail = String::from_utf8_lossy(&output.stderr);
    match detail.trim() {
        _ if output.status.success() => Ok(()),
        "" => bail!("{action} failed with status {}", output.status),
        detail => bail!("{action} failed: {detail}"),
    }
}

fn tool_failure(output: &ToolOutput, fallback: &str) -> String {
    output
        .stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

fn run_android_tool(name: &str, directory: &str, arguments: &[&str]) -> ToolOutput {
    let Some(program) = android_tool_program(name, directory) else {
        return ToolOutput::default();
    };
    run_tool(&program, arguments)
}

fn android_tool_program(name: &str, directory: &str) -> Option<String> {
    let program = executable_in_path(name).or_else(|| {
        let sdk_root = android_sdk_root()?;
        let directories = if directory == "cmdline-tools" {
            vec![
                sdk_root.join("cmdline-tools/latest/bin"),
                sdk_root.join("cmdline-tools/bin"),
                sdk_root.join("tools/bin"),
            ]
        } else {
            vec![sdk_root.join(directory)]
        };
        executable_in(directories, name)
    })?;
    Some(program.to_string_lossy().into_owned())
}

fn executable_suffixes() -> &'static [&'static str] {
    if cfg!(windows) {
        &["", ".exe", ".bat", ".cmd"]
    } else {
        &[""]
    }
}

fn executable_in_path(name: &str) -> Option<PathBuf> {
    executable_in(std::env::split_paths(&std::env::var_os("PATH")?), name)
}

/// The first `name` executable in `directories`.
fn executable_in(directories: impl IntoIterator<Item = PathBuf>, name: &str) -> Option<PathBuf> {
    directories
        .into_iter()
        .flat_map(|directory| {
            executable_suffixes()
                .iter()
                .map(move |suffix| directory.join(format!("{name}{suffix}")))
        })
        .find(|candidate| candidate.is_file())
}

fn android_sdk_root() -> Option<PathBuf> {
    ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
        .iter()
        .filter_map(|name| std::env::var_os(name).map(PathBuf::from))
        .find(|path| path.is_dir())
}

fn parse_ios_simulator_devices(source: &str) -> Option<Vec<Device>> {
    Some(
        parse_ios_simulator_targets(source)?
            .into_iter()
            .filter(|target| target.available && target.has_been_booted)
            .map(|target| Device {
                kind: target.kind(),
                id: target.id,
                platform: Platform::IosSimulator,
                group: DeviceGroup::Virtual,
                status: DeviceStatus::Available,
            })
            .collect(),
    )
}

fn parse_ios_simulator_targets(source: &str) -> Option<Vec<IosSimulatorTarget>> {
    let value = serde_json::from_str::<Value>(source).ok()?;
    let runtimes = value.get("devices")?.as_object()?;
    let mut targets = Vec::new();
    for (runtime, entries) in runtimes {
        if !is_ios_runtime(runtime) {
            continue;
        }
        let Some(entries) = entries.as_array() else {
            continue;
        };
        for entry in entries {
            let Some(id) = entry.get("udid").and_then(Value::as_str) else {
                continue;
            };
            let name = entry
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("iOS Simulator");
            let availability_error = entry
                .get("availabilityError")
                .and_then(Value::as_str)
                .filter(|reason| !reason.is_empty())
                .map(str::to_owned);
            targets.push(IosSimulatorTarget {
                id: id.to_owned(),
                name: name.to_owned(),
                available: entry
                    .get("isAvailable")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                state: entry
                    .get("state")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                has_been_booted: simulator_has_been_booted(entry),
                availability_error,
            });
        }
    }
    Some(targets)
}

fn simulator_has_been_booted(entry: &Value) -> bool {
    entry
        .get("state")
        .and_then(Value::as_str)
        .is_some_and(|state| state.eq_ignore_ascii_case("booted"))
        || entry
            .get("lastBootedAt")
            .and_then(Value::as_str)
            .is_some_and(|timestamp| !timestamp.is_empty())
}

fn latest_ios_runtime(source: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(source).ok()?;
    let runtimes = value.get("runtimes")?.as_array()?;
    runtimes
        .iter()
        .filter_map(|runtime| {
            let identifier = first_json_string(runtime, &[&["identifier"]])?;
            if !is_ios_runtime(&identifier)
                || !runtime
                    .get("isAvailable")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            {
                return None;
            }
            let version = first_json_string(runtime, &[&["version"], &["name"]])
                .unwrap_or_else(|| identifier.clone());
            Some((version_key(&version), identifier))
        })
        .max()
        .map(|(_, identifier)| identifier)
}

fn default_ios_device_type(source: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(source).ok()?;
    let device_types = value.get("devicetypes")?.as_array()?;
    device_types
        .iter()
        .filter_map(|device_type| {
            if device_type.get("isAvailable").and_then(Value::as_bool) == Some(false) {
                return None;
            }
            let identifier = first_json_string(device_type, &[&["identifier"]])?;
            let name =
                first_json_string(device_type, &[&["name"]]).unwrap_or_else(|| identifier.clone());
            let lower_name = name.to_ascii_lowercase();
            if !lower_name.contains("iphone") {
                return None;
            }
            Some((name, identifier))
        })
        .min()
        .map(|(_, identifier)| identifier)
}

fn select_android_system_image(source: &str) -> Option<String> {
    let mut images = parse_android_system_images(source);
    let preferred_abi = if cfg!(target_arch = "aarch64") {
        "arm64-v8a"
    } else {
        "x86_64"
    };
    images.sort_by(|left, right| {
        android_api_level(left)
            .cmp(&android_api_level(right))
            .then_with(|| left.cmp(right))
    });
    images
        .iter()
        .rev()
        .find(|image| image.contains(preferred_abi))
        .or_else(|| images.last())
        .cloned()
}

fn parse_android_system_images(source: &str) -> Vec<String> {
    let mut installed_packages = false;
    let mut images = Vec::new();
    for line in source.lines() {
        let line = line.trim();
        if line.eq_ignore_ascii_case("installed packages:") {
            installed_packages = true;
            continue;
        }
        if installed_packages && line.to_ascii_lowercase().ends_with("packages:") {
            installed_packages = false;
        }
        if !installed_packages {
            continue;
        }
        let Some(package) = line.split_whitespace().next() else {
            continue;
        };
        if package.starts_with("system-images;") {
            images.push(package.to_owned());
        }
    }
    images
}

fn android_api_level(image: &str) -> u32 {
    image
        .split(';')
        .find_map(|part| part.strip_prefix("android-")?.parse().ok())
        .unwrap_or_default()
}

fn version_key(value: &str) -> Vec<u32> {
    value
        .split(|character: char| !character.is_ascii_digit())
        .filter_map(|part| part.parse().ok())
        .collect()
}

fn parse_devicectl_devices(source: &str) -> Option<Vec<Device>> {
    let value = serde_json::from_str::<Value>(source).ok()?;
    let entries = value
        .get("devices")
        .and_then(Value::as_array)
        .or_else(|| value.get("result")?.get("devices")?.as_array())?;
    let mut devices = Vec::new();
    for entry in entries {
        let platform = first_json_string(
            entry,
            &[
                &["platform"],
                &["hardwareProperties", "platform"],
                &["deviceProperties", "platform"],
            ],
        )
        .unwrap_or_default()
        .to_ascii_lowercase();
        if !(platform.contains("ios") || platform.contains("iphone") || platform.contains("ipad"))
            || platform.contains("simulator")
        {
            continue;
        }
        let Some(id) = first_json_string(
            entry,
            &[
                &["identifier"],
                &["udid"],
                &["deviceProperties", "identifier"],
            ],
        ) else {
            continue;
        };
        let name = first_json_string(
            entry,
            &[
                &["name"],
                &["deviceProperties", "name"],
                &["hardwareProperties", "marketingName"],
                &["hardwareProperties", "modelName"],
            ],
        )
        .unwrap_or_else(|| "iOS device".to_owned());
        devices.push(Device {
            id,
            kind: physical_ios_type(&name),
            platform: Platform::Ios,
            group: DeviceGroup::Physical,
            status: physical_ios_status(entry),
        });
    }
    Some(devices)
}

fn parse_xctrace_devices(source: &str) -> Vec<Device> {
    let mut in_physical_devices = false;
    let mut devices = Vec::new();
    for line in source.lines() {
        let line = line.trim();
        if line.starts_with("== Devices ==") {
            in_physical_devices = true;
            continue;
        }
        if line.starts_with("== Simulators ==") {
            in_physical_devices = false;
            continue;
        }
        if !in_physical_devices {
            continue;
        }
        let Some(close) = line.rfind(')') else {
            continue;
        };
        let Some(open) = line[..close].rfind('(') else {
            continue;
        };
        let id = line[open + 1..close].trim();
        if id.is_empty() {
            continue;
        }
        let name = line[..open]
            .trim()
            .split(" (")
            .next()
            .unwrap_or("iOS device")
            .to_owned();
        let lower_name = name.to_ascii_lowercase();
        if !(lower_name.contains("iphone")
            || lower_name.contains("ipad")
            || lower_name.contains("ipod"))
        {
            continue;
        }
        devices.push(Device {
            id: id.to_owned(),
            kind: physical_ios_type(&name),
            platform: Platform::Ios,
            group: DeviceGroup::Physical,
            status: DeviceStatus::Available,
        });
    }
    devices
}

fn parse_android_avds(source: &str) -> Vec<String> {
    source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

fn parse_android_adb_devices(source: &str) -> Vec<Device> {
    let mut devices = Vec::new();
    for line in source.lines().skip(1) {
        let mut fields = line.split_whitespace();
        let Some(id) = fields.next() else {
            continue;
        };
        let Some(state) = fields.next() else {
            continue;
        };
        let model = line.split_whitespace().find_map(|field| {
            field
                .strip_prefix("model:")
                .map(|model| model.replace('_', " "))
        });
        let (generic_kind, group) = if id.starts_with("emulator-") {
            ("Android emulator", DeviceGroup::Virtual)
        } else {
            ("physical Android device", DeviceGroup::Physical)
        };
        let kind = model.map_or_else(
            || generic_kind.to_owned(),
            |model| format!("{model} / {generic_kind}"),
        );
        let status = match state {
            "device" => DeviceStatus::Available,
            "unauthorized" => {
                DeviceStatus::Blocked("authorize USB debugging on the device".to_owned())
            }
            "offline" => DeviceStatus::Blocked("the device is offline".to_owned()),
            "no" if line.contains("permissions") => {
                DeviceStatus::Blocked("grant this user USB access to the device".to_owned())
            }
            state => DeviceStatus::Blocked(format!("adb reports {state}")),
        };
        devices.push(Device {
            id: id.to_owned(),
            kind,
            platform: Platform::Android,
            group,
            status,
        });
    }
    devices
}

fn first_json_string(value: &Value, paths: &[&[&str]]) -> Option<String> {
    paths
        .iter()
        .find_map(|path| json_at(value, path)?.as_str().map(str::to_owned))
}

fn json_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))
}

fn physical_ios_type(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let kind = if lower.contains("iphone") {
        "physical iPhone"
    } else if lower.contains("ipad") {
        "physical iPad"
    } else {
        "physical iOS device"
    };
    if name == "iOS device" {
        kind.to_owned()
    } else {
        format!("{name} / {kind}")
    }
}

fn physical_ios_status(value: &Value) -> DeviceStatus {
    let pairing = first_json_string(
        value,
        &[
            &["pairingState"],
            &["connectionProperties", "pairingState"],
            &["deviceProperties", "pairingState"],
        ],
    )
    .unwrap_or_default()
    .to_ascii_lowercase();
    if pairing.contains("untrusted") || pairing.contains("unpaired") {
        return DeviceStatus::Blocked("trust and pair the device".to_owned());
    }
    let connection = first_json_string(
        value,
        &[
            &["connectionState"],
            &["connectionProperties", "connectionState"],
            &["deviceProperties", "state"],
        ],
    )
    .unwrap_or_default()
    .to_ascii_lowercase();
    if connection.contains("disconnect") || connection.contains("offline") {
        return DeviceStatus::Blocked("connect the device".to_owned());
    }
    for path in [
        &["isAvailable"][..],
        &["isConnected"][..],
        &["connectionProperties", "isConnected"][..],
    ] {
        if json_at(value, path).and_then(Value::as_bool) == Some(false) {
            return DeviceStatus::Blocked("connect or trust the device".to_owned());
        }
    }
    DeviceStatus::Available
}

fn is_ios_runtime(identifier: &str) -> bool {
    let identifier = identifier.to_ascii_lowercase();
    identifier.contains("ios")
        && !identifier.contains("watch")
        && !identifier.contains("tvos")
        && !identifier.contains("vision")
}

fn render_devices(devices: &[Device]) -> String {
    let id_width = devices
        .iter()
        .map(|device| device.id.len())
        .fold(2, usize::max)
        + 2;
    let type_width = devices
        .iter()
        .map(|device| device.kind.len())
        .fold(4, usize::max)
        + 2;
    let mut output = format!("{:<id_width$}{:<type_width$}Status\n", "ID", "Type");
    for device in devices {
        let _ = writeln!(
            output,
            "{:<id_width$}{:<type_width$}{}",
            device.id,
            device.kind,
            device.status.display()
        );
    }
    output
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        Device, DeviceGroup, DeviceStatus, IosSimulatorUi, default_ios_device_type,
        ios_simulator_ui_for_developer_dir, is_transient_devicectl_error, latest_ios_runtime,
        order_devices, parse_android_adb_devices, parse_android_system_images,
        parse_ios_simulator_devices, parse_ios_simulator_targets, parse_xctrace_devices,
        prepared_device, render_devices, select_android_system_image,
    };
    use tokamak_cli::Platform;

    #[test]
    fn prefers_standalone_simulator_when_both_xcode_uis_exist()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let developer_dir = temp.path().join("Xcode.app/Contents/Developer");
        let simulator = developer_dir.join("Applications/Simulator.app");
        let device_hub = developer_dir
            .parent()
            .ok_or("Xcode Contents directory missing")?
            .join("Applications/DeviceHub.app");
        fs::create_dir_all(&simulator)?;
        fs::create_dir_all(&device_hub)?;

        assert_eq!(
            ios_simulator_ui_for_developer_dir(&developer_dir),
            Some(IosSimulatorUi::StandaloneSimulator(simulator))
        );
        Ok(())
    }

    #[test]
    fn uses_device_hub_when_xcode_has_no_standalone_simulator()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let developer_dir = temp.path().join("Xcode.app/Contents/Developer");
        let device_hub = developer_dir
            .parent()
            .ok_or("Xcode Contents directory missing")?
            .join("Applications/DeviceHub.app");
        fs::create_dir_all(&device_hub)?;

        assert_eq!(
            ios_simulator_ui_for_developer_dir(&developer_dir),
            Some(IosSimulatorUi::DeviceHub(device_hub))
        );
        Ok(())
    }

    #[test]
    fn reports_no_simulator_ui_when_xcode_contains_neither_app()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let developer_dir = temp.path().join("Xcode.app/Contents/Developer");

        assert_eq!(ios_simulator_ui_for_developer_dir(&developer_dir), None);
        Ok(())
    }

    #[test]
    fn parses_ios_simulator_devices() {
        let devices = parse_ios_simulator_devices(
            r#"{
                "devices": {
                    "com.apple.CoreSimulator.SimRuntime.iOS-18-0": [
                        {"name": "iPhone 15", "udid": "SIM-1", "isAvailable": true,
                         "state": "Booted"},
                        {"name": "iPhone 14", "udid": "SIM-2", "isAvailable": true,
                         "state": "Shutdown", "lastBootedAt": "2026-08-01T12:00:00Z"},
                        {"name": "iPhone 13", "udid": "SIM-3", "isAvailable": true,
                         "state": "Shutdown"},
                        {"name": "iPhone 12", "udid": "SIM-4", "isAvailable": false,
                         "availabilityError": "runtime missing"}
                    ]
                }
            }"#,
        )
        .unwrap_or_default();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].status, DeviceStatus::Available);
        assert_eq!(devices[0].id, "SIM-1");
        assert_eq!(devices[1].id, "SIM-2");
    }

    #[test]
    fn parses_ios_simulator_targets_for_preparation() {
        let targets = parse_ios_simulator_targets(
            r#"{
                "devices": {
                    "com.apple.CoreSimulator.SimRuntime.iOS-18-0": [
                        {"name": "iPhone 15", "udid": "SIM-1", "isAvailable": true,
                         "state": "Shutdown"},
                        {"name": "iPhone 14", "udid": "SIM-2", "isAvailable": false,
                         "availabilityError": "runtime missing"}
                    ]
                }
            }"#,
        )
        .unwrap_or_default();
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].id, "SIM-1");
        assert!(targets[0].available);
        assert_eq!(
            targets[1].availability_error.as_deref(),
            Some("runtime missing")
        );
        assert!(!targets[1].available);
    }

    #[test]
    fn selects_latest_ios_runtime_and_default_device_type() {
        let runtimes = r#"{
            "runtimes": [
                {"identifier": "com.apple.CoreSimulator.SimRuntime.iOS-17-0",
                 "version": "17.0", "isAvailable": true},
                {"identifier": "com.apple.CoreSimulator.SimRuntime.iOS-18-2",
                 "version": "18.2", "isAvailable": true},
                {"identifier": "com.apple.CoreSimulator.SimRuntime.iOS-19-0",
                 "version": "19.0", "isAvailable": false}
            ]
        }"#;
        assert_eq!(
            latest_ios_runtime(runtimes).as_deref(),
            Some("com.apple.CoreSimulator.SimRuntime.iOS-18-2")
        );

        let device_types = r#"{
            "devicetypes": [
                {"name": "iPad Pro", "identifier": "ipad", "isAvailable": true},
                {"name": "iPhone 15", "identifier": "iphone-15", "isAvailable": true}
            ]
        }"#;
        assert_eq!(
            default_ios_device_type(device_types).as_deref(),
            Some("iphone-15")
        );
    }

    #[test]
    fn parses_android_devices_and_states() {
        let devices = parse_android_adb_devices(
            "List of devices attached\n emulator-5554 device product:sdk model:Pixel_8\n phone-1 unauthorized usb:1\n phone-2 no permissions usb:2\n",
        );
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].id, "emulator-5554");
        assert_eq!(devices[0].status, DeviceStatus::Available);
        assert_eq!(
            devices[1].status,
            DeviceStatus::Blocked("authorize USB debugging on the device".to_owned())
        );
        assert_eq!(
            devices[2].status,
            DeviceStatus::Blocked("grant this user USB access to the device".to_owned())
        );
    }

    #[test]
    fn prepares_adb_devices_as_android_whatever_their_model() -> anyhow::Result<()> {
        let devices = parse_android_adb_devices(
            "List of devices attached\nphone-1 device model:iPad_Pro\nemulator-5554 device model:iOS_Simulator\n",
        );
        for device in devices {
            assert_eq!(prepared_device(device)?.platform, Platform::Android);
        }
        Ok(())
    }

    #[test]
    fn selects_installed_android_system_image_for_host_architecture() {
        let packages = "Installed packages:\n  Path | Version | Description\n  system-images;android-34;google_apis;x86_64 | 1 | image\n  system-images;android-35;google_apis;arm64-v8a | 1 | image\nAvailable Packages:\n  system-images;android-36;google_apis;arm64-v8a | 1 | image\n";
        let images = parse_android_system_images(packages);
        assert_eq!(images.len(), 2);
        let selected = select_android_system_image(packages).unwrap_or_default();
        assert!(selected.starts_with("system-images;android-"));
        assert!(!selected.contains("android-36"));
    }

    #[test]
    fn parses_physical_ios_devices_from_xctrace() {
        let devices = parse_xctrace_devices(
            "== Devices ==\nTom's iPhone (iOS 18.0) (PHONE-1)\nMacBook Pro (macOS 15.0) (MAC-1)\n== Simulators ==\niPhone 15 (iOS 18.0) (SIM-1)\n",
        );
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id, "PHONE-1");
        assert_eq!(devices[0].kind, "Tom's iPhone / physical iPhone");
    }

    #[test]
    fn renders_device_table() {
        let output = render_devices(&[
            Device {
                id: "macos".to_owned(),
                kind: "macOS desktop".to_owned(),
                platform: Platform::Macos,
                group: DeviceGroup::Host,
                status: DeviceStatus::Available,
            },
            Device {
                id: "phone".to_owned(),
                kind: "physical Android device".to_owned(),
                platform: Platform::Android,
                group: DeviceGroup::Physical,
                status: DeviceStatus::Blocked("authorize USB debugging".to_owned()),
            },
        ]);
        assert!(
            output
                .lines()
                .next()
                .is_some_and(|line| line.contains("ID") && line.contains("Type"))
        );
        assert!(output.contains("macos"));
        assert!(output.contains("blocked: authorize USB debugging"));
    }

    #[test]
    fn orders_managed_then_physical_then_other_devices_whatever_their_names() {
        let mut devices = parse_android_adb_devices(
            "List of devices attached\nemulator-5554 device model:managed_physical\nphone-1 device model:Pixel_8\n",
        );
        devices.push(Device {
            id: "android".to_owned(),
            kind: "managed Android emulator".to_owned(),
            platform: Platform::Android,
            group: DeviceGroup::Managed,
            status: DeviceStatus::Available,
        });

        order_devices(&mut devices);

        assert_eq!(
            devices
                .iter()
                .map(|device| device.id.as_str())
                .collect::<Vec<_>>(),
            ["android", "phone-1", "emulator-5554"]
        );
    }

    #[test]
    fn retries_only_transient_devicectl_errors() {
        assert!(is_transient_devicectl_error("Connection reset by peer"));
        assert!(is_transient_devicectl_error(
            "CoreDevice.ControlChannelConnectionError"
        ));
        assert!(!is_transient_devicectl_error(
            "The executable contains an invalid signature"
        ));
    }
}
