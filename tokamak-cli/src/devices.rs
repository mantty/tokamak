//! Development target discovery and table rendering.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Stdio};
use std::time::Duration;

use anyhow::{Result, bail};
use serde_json::Value;
use tokamak_cli::Platform;

#[cfg(target_os = "macos")]
use anyhow::Context;
#[cfg(any(target_os = "macos", test))]
use std::path::Path;

const MANAGED_IOS_NAME: &str = "tokamak iPhone";
const MANAGED_ANDROID_NAME: &str = "tokamak-managed";
const ANDROID_BOOT_TIMEOUT: usize = 120;

#[derive(Clone, Debug, Eq, PartialEq)]
struct Device {
    id: String,
    kind: String,
    status: DeviceStatus,
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
    if let Some(device) = prepare_android_device(selector)? {
        return Ok(device);
    }
    if let Some(device) = prepare_ios_simulator(selector)? {
        return Ok(device);
    }
    if let Some(device) = prepare_ios_physical(selector)? {
        return Ok(device);
    }

    bail!("device `{selector}` was not found; run `tok devices` to list available targets");
}

fn prepare_host(selector: &str) -> Result<Option<PreparedDevice>> {
    if !matches!(selector, "macos" | "windows" | "linux") {
        return Ok(None);
    }
    let host = host_device();
    if host.id != selector {
        bail!(
            "device `{selector}` is not available on this host; local host is `{}`",
            host.id
        );
    }
    Ok(Some(prepared_device(host)?))
}

fn prepared_device(device: Device) -> Result<PreparedDevice> {
    match device.status {
        DeviceStatus::Available => Ok(PreparedDevice {
            id: device.id,
            platform: device_platform(&device.kind),
            kind: device.kind,
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
pub(super) fn open_ios_simulator_ui(device_id: &str) -> Result<()> {
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
        let reason = target
            .availability_error
            .as_deref()
            .filter(|reason| !reason.is_empty())
            .unwrap_or("the Simulator runtime is unavailable");
        bail!("iOS Simulator `{selector}` is unavailable: {reason}");
    }
    boot_ios_simulator(&target)?;
    Ok(Some(PreparedDevice {
        id: target.id,
        kind: format!("{} / iOS Simulator", target.name),
        platform: Platform::IosSimulator,
    }))
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
        return Ok(PreparedDevice {
            id: target.id.clone(),
            kind: format!("{} / iOS Simulator", target.name),
            platform: Platform::IosSimulator,
        });
    }
    if let Some(target) = targets
        .iter()
        .find(|target| target.name == MANAGED_IOS_NAME)
    {
        let reason = target
            .availability_error
            .as_deref()
            .filter(|reason| !reason.is_empty())
            .unwrap_or("the Simulator runtime is unavailable");
        bail!("managed iOS Simulator is unavailable: {reason}");
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
    Ok(PreparedDevice {
        id: target.id,
        kind: format!("{} / iOS Simulator", target.name),
        platform: Platform::IosSimulator,
    })
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
    let Some(device) = discover_ios_physical_devices()
        .into_iter()
        .find(|device| device.id == selector)
    else {
        return Ok(None);
    };
    Ok(Some(prepared_device(device)?))
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

fn prepare_android_device(selector: &str) -> Result<Option<PreparedDevice>> {
    let emulator = run_android_tool("emulator", "emulator", &["-list-avds"]);
    if emulator.success
        && parse_android_avds(&emulator.stdout)
            .iter()
            .any(|name| name == selector)
    {
        return prepare_android_avd(selector).map(Some);
    }

    let adb = run_android_tool("adb", "platform-tools", &["devices", "-l"]);
    if !adb.available {
        return Ok(None);
    }
    if !adb.success {
        bail!(
            "unable to query Android devices: {}",
            tool_failure(&adb, "adb devices failed")
        );
    }
    if let Some(device) = parse_android_adb_devices(&adb.stdout)
        .into_iter()
        .find(|device| device.id == selector)
    {
        return Ok(Some(prepared_device(device)?));
    }
    Ok(None)
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
            return Ok(PreparedDevice {
                id: serial,
                kind: format!("{avd_name} / Android emulator (AVD)"),
                platform: Platform::Android,
            });
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
    Ok(PreparedDevice {
        id: serial,
        kind: format!("{avd_name} / Android emulator (AVD)"),
        platform: Platform::Android,
    })
}

fn device_platform(kind: &str) -> Platform {
    if kind.contains("iOS Simulator") {
        Platform::IosSimulator
    } else if kind.contains("iOS") || kind.contains("iPhone") || kind.contains("iPad") {
        Platform::Ios
    } else if kind.contains("Android") {
        Platform::Android
    } else if kind.starts_with("macOS") {
        Platform::Macos
    } else {
        Platform::Windows
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

fn discover_devices() -> Vec<Device> {
    let mut devices = vec![host_device()];
    let mut discovered = discover_ios_devices();
    discovered.extend(discover_android_devices());
    order_devices(&mut discovered);
    devices.extend(discovered);
    devices
}

fn is_physical_device(device: &Device) -> bool {
    device.kind.contains("physical")
}

fn is_managed_device(device: &Device) -> bool {
    device.kind.starts_with("managed ")
}

fn order_devices(devices: &mut [Device]) {
    devices.sort_by_key(|device| {
        if is_managed_device(device) {
            0
        } else if is_physical_device(device) {
            1
        } else {
            2
        }
    });
}

fn host_device() -> Device {
    let (id, device_type, status) = if cfg!(target_os = "macos") {
        ("macos", "macOS desktop", DeviceStatus::Available)
    } else if cfg!(target_os = "windows") {
        ("windows", "Windows desktop", DeviceStatus::Available)
    } else {
        (
            "linux",
            "Linux desktop",
            DeviceStatus::Blocked(
                "the tokamak desktop target is unavailable on this host".to_owned(),
            ),
        )
    };
    Device {
        id: id.to_owned(),
        kind: device_type.to_owned(),
        status,
    }
}

fn discover_ios_devices() -> Vec<Device> {
    let mut managed = Device {
        id: "ios".to_owned(),
        kind: "managed iOS Simulator".to_owned(),
        status: DeviceStatus::Blocked("iOS Simulator requires macOS and Xcode".to_owned()),
    };
    if !cfg!(target_os = "macos") {
        return vec![managed];
    }

    let runtimes = run_tool("xcrun", &["simctl", "list", "runtimes", "--json"]);
    if !runtimes.available {
        managed.status =
            DeviceStatus::Blocked("Xcode command-line tools are not installed".to_owned());
        return with_ios_physical_devices(managed);
    }
    if !runtimes.success {
        managed.status =
            DeviceStatus::Blocked("Xcode Simulator services are unavailable".to_owned());
        return with_ios_physical_devices(managed);
    }
    if !has_available_ios_runtime(&runtimes.stdout) {
        managed.status = DeviceStatus::Blocked("no iOS Simulator runtime is installed".to_owned());
        return with_ios_physical_devices(managed);
    }

    let simulators = run_tool("xcrun", &["simctl", "list", "devices", "--json"]);
    if !simulators.success {
        managed.status =
            DeviceStatus::Blocked("Xcode Simulator services are unavailable".to_owned());
        return with_ios_physical_devices(managed);
    }
    let Some(simulator_devices) = parse_ios_simulator_devices(&simulators.stdout) else {
        managed.status = DeviceStatus::Blocked("unable to read iOS Simulator devices".to_owned());
        return with_ios_physical_devices(managed);
    };

    managed.status = DeviceStatus::Available;
    let mut devices = vec![managed];
    devices.extend(simulator_devices);
    devices.extend(discover_ios_physical_devices());
    devices
}

fn with_ios_physical_devices(managed: Device) -> Vec<Device> {
    let mut devices = vec![managed];
    devices.extend(discover_ios_physical_devices());
    devices
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
        status: alias_status.clone(),
    }];
    devices.extend(avds.into_iter().map(|id| Device {
        id,
        kind: "Android emulator (AVD)".to_owned(),
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

fn run_tool(program: &str, arguments: &[&str]) -> ToolOutput {
    match ProcessCommand::new(program).args(arguments).output() {
        Ok(output) => ToolOutput {
            available: true,
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        },
        Err(_) => ToolOutput::default(),
    }
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
    let output = child.wait_with_output()?;
    Ok(ToolOutput {
        available: true,
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
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
    if let Some(path) = executable_in_path(name) {
        return Some(path.to_string_lossy().into_owned());
    }
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
    directories
        .into_iter()
        .flat_map(|directory| {
            executable_suffixes()
                .iter()
                .map(move |suffix| directory.join(format!("{name}{suffix}")))
        })
        .find(|candidate| candidate.is_file())
        .map(|candidate| candidate.to_string_lossy().into_owned())
}

fn executable_suffixes() -> &'static [&'static str] {
    if cfg!(windows) {
        &["", ".exe", ".bat", ".cmd"]
    } else {
        &[""]
    }
}

fn executable_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        for suffix in executable_suffixes() {
            let candidate = directory.join(format!("{name}{suffix}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn android_sdk_root() -> Option<PathBuf> {
    ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
        .iter()
        .filter_map(|name| std::env::var_os(name).map(PathBuf::from))
        .find(|path| path.is_dir())
}

fn has_available_ios_runtime(source: &str) -> bool {
    latest_ios_runtime(source).is_some()
}

fn parse_ios_simulator_devices(source: &str) -> Option<Vec<Device>> {
    Some(
        parse_ios_simulator_targets(source)?
            .into_iter()
            .filter(|target| target.available && target.has_been_booted)
            .map(|target| Device {
                id: target.id,
                kind: format!("{} / iOS Simulator", target.name),
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
    let mut candidates = runtimes
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
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    candidates.pop().map(|(_, identifier)| identifier)
}

fn default_ios_device_type(source: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(source).ok()?;
    let device_types = value.get("devicetypes")?.as_array()?;
    let mut candidates = device_types
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
        .collect::<Vec<_>>();
    candidates.sort();
    candidates
        .into_iter()
        .next()
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
        let is_emulator = id.starts_with("emulator-");
        let kind = if is_emulator {
            model.map_or_else(
                || "Android emulator".to_owned(),
                |model| format!("{model} / Android emulator"),
            )
        } else {
            model.map_or_else(
                || "physical Android device".to_owned(),
                |model| format!("{model} / physical Android device"),
            )
        };
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
            status,
        });
    }
    devices
}

fn first_json_string(value: &Value, paths: &[&[&str]]) -> Option<String> {
    paths.iter().find_map(|path| {
        let mut current = value;
        for key in *path {
            current = current.get(*key)?;
        }
        current.as_str().map(str::to_owned)
    })
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
        if json_bool_at(value, path) == Some(false) {
            return DeviceStatus::Blocked("connect or trust the device".to_owned());
        }
    }
    DeviceStatus::Available
}

fn json_bool_at(value: &Value, path: &[&str]) -> Option<bool> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_bool()
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
        .max()
        .unwrap_or(2)
        .max(2)
        + 2;
    let type_width = devices
        .iter()
        .map(|device| device.kind.len())
        .max()
        .unwrap_or(4)
        .max(4)
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
        Device, DeviceStatus, IosSimulatorUi, default_ios_device_type,
        ios_simulator_ui_for_developer_dir, latest_ios_runtime, order_devices,
        parse_android_adb_devices, parse_android_system_images, parse_ios_simulator_devices,
        parse_ios_simulator_targets, parse_xctrace_devices, render_devices,
        select_android_system_image,
    };

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
                status: DeviceStatus::Available,
            },
            Device {
                id: "phone".to_owned(),
                kind: "physical Android device".to_owned(),
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
    fn orders_managed_then_physical_then_other_devices() {
        let mut devices = vec![
            Device {
                id: "simulator".to_owned(),
                kind: "iPhone / iOS Simulator".to_owned(),
                status: DeviceStatus::Available,
            },
            Device {
                id: "phone".to_owned(),
                kind: "physical iPhone".to_owned(),
                status: DeviceStatus::Available,
            },
            Device {
                id: "ios".to_owned(),
                kind: "managed iOS Simulator".to_owned(),
                status: DeviceStatus::Available,
            },
        ];

        order_devices(&mut devices);

        assert_eq!(
            devices
                .iter()
                .map(|device| device.id.as_str())
                .collect::<Vec<_>>(),
            ["ios", "phone", "simulator"]
        );
    }
}
