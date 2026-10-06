use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use assert_cmd::Command;
#[cfg(unix)]
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;
use tokamak::compile_module;
use tokamak::{
    HtmlHandling, ModuleType, NotFoundHandling, PackageLayout, read_asset_manifest,
    read_worker_manifest, read_worker_module,
};
use tokamak_cli::{
    MANIFEST_FILE, PackVariable, PlatformPackManifest, Target, VariableKind, write_manifest,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// A project whose earlier build left Worker output as Cloudflare's Vite
/// plugin writes it, and no configuration file as the tokamak Vite plugin
/// reports it. Its build command stands in for that build.
fn create_project(root: &Path) -> TestResult {
    fs::write(
        root.join("package.json"),
        r#"{"name":"demo-app","scripts":{"build":"node build.cjs"}}"#,
    )?;
    fs::write(
        root.join("build.cjs"),
        include_str!("fixtures/vite-build.cjs"),
    )?;
    fs::create_dir_all(root.join("dist/app"))?;
    fs::create_dir_all(root.join("dist/client/styles"))?;
    fs::write(root.join("dist/app/index.js"), "export default {};")?;
    fs::write(root.join("dist/client/index.html"), "<html></html>")?;
    fs::write(root.join("dist/client/styles/app.css"), "body{}")?;
    write_worker_config(root, "{}")?;
    fs::create_dir_all(root.join(".wrangler/deploy"))?;
    fs::write(
        root.join(".wrangler/deploy/config.json"),
        r#"{"configPath":"../../dist/app/wrangler.json"}"#,
    )?;
    write_report(root, &serde_json::json!({ "config": {} }))
}

/// Write the Wrangler configuration Cloudflare's Vite plugin generates, with
/// `fields` over the demo app's.
fn write_worker_config(root: &Path, fields: &str) -> TestResult {
    let mut config = serde_json::json!({
        "name": "demo-app",
        "main": "index.js",
        "rules": [{ "type": "ESModule", "globs": ["**/*.js", "**/*.mjs"] }],
        "assets": { "directory": "../client", "binding": "ASSETS" },
        "no_bundle": true,
    });
    let serde_json::Value::Object(fields) = serde_json::from_str(fields)? else {
        return Err("Worker config fields must be an object".into());
    };
    config
        .as_object_mut()
        .ok_or("Worker config must be an object")?
        .extend(fields);
    fs::write(
        root.join("dist/app/wrangler.json"),
        serde_json::to_vec(&config)?,
    )?;
    Ok(())
}

/// Report `config` as the `config` export of `src/tokamak.ts`.
fn configure(root: &Path, config: &str) -> TestResult {
    write_report(
        root,
        &serde_json::json!({
            "file": root.join("src/tokamak.ts"),
            "config": serde_json::from_str::<serde_json::Value>(config)?,
        }),
    )
}

/// Write the tokamak Vite plugin's `report` where the earlier build left it,
/// and to `vite-report.json`, which the stand-in build and development
/// commands report.
fn write_report(root: &Path, report: &serde_json::Value) -> TestResult {
    let report = report.to_string();
    fs::write(root.join("vite-report.json"), &report)?;
    fs::create_dir_all(root.join("build/.tokamak/vite"))?;
    fs::write(root.join("build/.tokamak/vite/config.json"), report)?;
    Ok(())
}

fn declare_storage(root: &Path) -> TestResult {
    write_worker_config(
        root,
        r#"{ "kv_namespaces": [{ "binding": "SESSION", "id": "session" }] }"#,
    )
}

fn install_location_plugin(root: &Path) -> TestResult {
    fs::write(
        root.join("package.json"),
        r#"{"name":"demo-app","scripts":{"build":"echo already-built"},"dependencies":{"@tokamakdev/plugin-location":"1.0.0"}}"#,
    )?;
    let plugin = root.join("node_modules/@tokamakdev/plugin-location");
    fs::create_dir_all(plugin.join("apple"))?;
    fs::write(
        plugin.join("apple/LocationPlugin.swift"),
        include_str!("../../plugins/location/apple/LocationPlugin.swift"),
    )?;
    for (name, key) in [
        ("macos", "NSLocationUsageDescription"),
        ("ios", "NSLocationWhenInUseUsageDescription"),
    ] {
        fs::write(
            plugin.join(format!("apple/{name}.plist")),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>{key}</key><string>Location test</string></dict></plist>"#
            ),
        )?;
    }
    fs::write(
        plugin.join("tokamak-plugin.json"),
        r#"{
  "schemaVersion": 1,
  "id": "location",
  "platforms": {
    "macos": {
      "class": "TokamakLocationPlugin",
      "sources": ["apple/LocationPlugin.swift"],
      "frameworks": ["CoreLocation"],
      "plist": "apple/macos.plist"
    },
    "ios": {
      "class": "TokamakLocationPlugin",
      "sources": ["apple/LocationPlugin.swift"],
      "frameworks": ["CoreLocation"],
      "plist": "apple/ios.plist"
    },
    "ios-simulator": {
      "class": "TokamakLocationPlugin",
      "sources": ["apple/LocationPlugin.swift"],
      "frameworks": ["CoreLocation"],
      "plist": "apple/ios.plist"
    }
  }
}"#,
    )?;
    Ok(())
}

fn install_key_flow_plugins(root: &Path) -> TestResult {
    fs::write(
        root.join("package.json"),
        r#"{"name":"demo-app","scripts":{"build":"echo already-built"},"dependencies":{"@tokamakdev/plugin-secure-storage":"1.0.0","@tokamakdev/plugin-local-authentication":"1.0.0"}}"#,
    )?;
    for (package, manifest, source_name, source, plist) in [
        (
            "plugin-secure-storage",
            include_str!("../../plugins/secure-storage/tokamak-plugin.json"),
            "SecureStoragePlugin.swift",
            include_str!("../../plugins/secure-storage/apple/SecureStoragePlugin.swift"),
            include_str!("../../plugins/secure-storage/apple/Info.plist"),
        ),
        (
            "plugin-local-authentication",
            include_str!("../../plugins/local-authentication/tokamak-plugin.json"),
            "LocalAuthenticationPlugin.swift",
            include_str!(
                "../../plugins/local-authentication/apple/LocalAuthenticationPlugin.swift"
            ),
            include_str!("../../plugins/local-authentication/apple/Info.plist"),
        ),
    ] {
        let plugin = root.join("node_modules/@tokamakdev").join(package);
        fs::create_dir_all(plugin.join("apple"))?;
        fs::write(plugin.join("tokamak-plugin.json"), manifest)?;
        fs::write(plugin.join("apple").join(source_name), source)?;
        fs::write(plugin.join("apple/Info.plist"), plist)?;
    }
    Ok(())
}

fn create_platform_pack(root: &Path, target: &str) -> TestResult<PathBuf> {
    let target_name = target;
    let target = target_name.parse::<Target>()?;
    let framework = root.join(target.runtime_artifact_path());
    let shell = root.join("native-shell");
    fs::create_dir_all(&framework)?;
    fs::create_dir_all(&shell)?;
    fs::create_dir_all(
        root.join(target.build_entrypoint_path())
            .parent()
            .ok_or("entrypoint path has no parent")?,
    )?;
    create_test_framework(root, target_name)?;
    write_test_shell(&shell)?;
    fs::write(
        root.join(target.build_entrypoint_path()),
        include_str!("../../platforms/apple/build/entrypoint"),
    )?;
    #[cfg(unix)]
    if matches!(
        target,
        Target::MacosArm64
            | Target::MacosX64
            | Target::IosArm64
            | Target::IosSimulatorArm64
            | Target::IosSimulatorX64
    ) {
        write_executable(
            &root.join("tools/tokamak-apple-signing"),
            include_str!("fixtures/apple-signing"),
        )?;
    }
    write_test_manifest(root, target)?;
    Ok(root.to_path_buf())
}

#[cfg(unix)]
fn create_android_platform_pack(root: &Path) -> TestResult<PathBuf> {
    let target = Target::AndroidArm64;
    let runtime = root.join(target.runtime_artifact_path());
    fs::create_dir_all(&runtime)?;
    fs::write(runtime.join("libtokamak.a"), "runtime")?;
    fs::write(runtime.join("link-libraries"), "-llog")?;
    fs::create_dir_all(root.join("native-shell/app"))?;
    fs::create_dir_all(root.join("native-shell/plugin"))?;
    fs::File::create(root.join("native-shell/app/TokamakActivity.kt"))?;
    fs::File::create(root.join("native-shell/plugin/TokamakPlugin.kt"))?;
    let entrypoint = root.join(target.build_entrypoint_path());
    fs::create_dir_all(entrypoint.parent().ok_or("entrypoint path has no parent")?)?;
    fs::write(
        entrypoint,
        include_str!("../../platforms/android/build/entrypoint"),
    )?;
    write_test_manifest(root, target)?;
    Ok(root.to_path_buf())
}

#[cfg(unix)]
fn create_android_inputs() -> TestResult<(tempfile::TempDir, PathBuf, PathBuf)> {
    let temporary = tempfile::tempdir()?;
    let project = temporary.path().join("project");
    let pack = temporary.path().join("pack");
    fs::create_dir_all(&project)?;
    fs::create_dir_all(&pack)?;
    create_project(&project)?;
    let platform_pack = create_android_platform_pack(&pack)?;
    Ok((temporary, project, platform_pack))
}

#[cfg(unix)]
fn install_android_plugins(root: &Path, plugins: &[(&str, &[&str])]) -> TestResult {
    let dependencies: serde_json::Map<_, _> = plugins
        .iter()
        .map(|(id, _)| ((*id).to_owned(), "1.0.0".into()))
        .collect();
    fs::write(
        root.join("package.json"),
        serde_json::json!({ "name": "demo-app", "dependencies": dependencies }).to_string(),
    )?;
    for (id, permissions) in plugins {
        let plugin = root.join("node_modules").join(id);
        fs::create_dir_all(plugin.join("android"))?;
        fs::write(plugin.join("android/Plugin.kt"), "")?;
        let manifest = serde_json::json!({
            "schemaVersion": 1,
            "id": id,
            "platforms": {
                "android": {
                    "class": format!("test.{}.Plugin", id.replace('-', "")),
                    "sources": ["android/Plugin.kt"],
                    "permissions": permissions,
                }
            }
        });
        fs::write(plugin.join("tokamak-plugin.json"), manifest.to_string())?;
    }
    Ok(())
}

fn write_test_manifest(root: &Path, target: Target) -> TestResult {
    write_manifest(
        root.join(MANIFEST_FILE),
        &test_manifest(target, env!("CARGO_PKG_VERSION"))?,
    )?;
    Ok(())
}

/// A manifest declaring the pack's real variables and `test`, which fake
/// entrypoints write out.
fn test_manifest(target: Target, tokamak_version: &str) -> TestResult<PlatformPackManifest> {
    let declarations = match target.platform().repository_directory_name() {
        "apple" => include_str!("../../platforms/apple/build/variables.json"),
        "android" => include_str!("../../platforms/android/build/variables.json"),
        _ => include_str!("../../platforms/windows/build/variables.json"),
    };
    let mut namespaces: std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<String, PackVariable>,
    > = serde_json::from_str(declarations)?;
    let mut variables = namespaces
        .remove(target.platform().namespace())
        .ok_or("the pack does not declare its namespace")?;
    variables.insert(
        "test".to_owned(),
        PackVariable {
            kind: VariableKind::String,
            description: "Written out by fake entrypoints".to_owned(),
        },
    );
    Ok(PlatformPackManifest {
        tokamak_version: tokamak_version.to_owned(),
        target,
        artifacts: target.artifacts(),
        required_tools: target
            .required_tools()
            .iter()
            .map(|tool| (*tool).to_owned())
            .collect(),
        variables,
    })
}

fn write_test_shell(shell: &Path) -> TestResult {
    fs::write(
        shell.join("TokamakShell.swift"),
        r#"import Foundation

struct TokamakPluginError: Error {
  let name: String
  let message: String

  static func notSupported(_ message: String) -> Self {
    Self(name: "NotSupportedError", message: message)
  }
}

typealias TokamakPluginReply = (Result<Any?, TokamakPluginError>) -> Void

final class TokamakHost {}

protocol TokamakPlugin: AnyObject {
  var id: String { get }
  init(host: TokamakHost)
  func call(method: String, arguments: Any, reply: @escaping TokamakPluginReply)
  func subscribe(
    method: String,
    arguments: Any,
    reply: @escaping TokamakPluginReply
  ) -> (() -> Void)
}

extension TokamakPlugin {
  func subscribe(
    method: String,
    arguments: Any,
    reply: @escaping TokamakPluginReply
  ) -> (() -> Void) {
    reply(.failure(.notSupported("\(id).\(method) is not supported")))
    return {}
  }
}

@main struct App { static func main() {} }
"#,
    )?;
    Ok(())
}

fn create_test_framework(root: &Path, target: &str) -> TestResult {
    let source = root.join("runtime.c");
    let object = root.join("runtime.o");
    fs::write(
        &source,
        "void tokamak_test(void) {}\nconst char tokamak_storage = 0;",
    )?;

    let (sdk, triple) = match target {
        "macos-arm64" => ("macosx", "arm64-apple-macos14.0"),
        "macos-x64" => ("macosx", "x86_64-apple-macos14.0"),
        "ios-arm64" => ("iphoneos", "arm64-apple-ios17.0"),
        "ios-simulator-arm64" => ("iphonesimulator", "arm64-apple-ios17.0-simulator"),
        "ios-simulator-x64" => ("iphonesimulator", "x86_64-apple-ios17.0-simulator"),
        _ => return Err(format!("unsupported Apple test target: {target}").into()),
    };
    let status = ProcessCommand::new("xcrun")
        .args(["--sdk", sdk, "clang", "-target", triple, "-c"])
        .arg(&source)
        .args(["-o"])
        .arg(&object)
        .status()?;
    if !status.success() {
        return Err(format!("test framework compilation failed with {status}").into());
    }
    let status = ProcessCommand::new("xcrun")
        .args(["libtool", "-static", "-o"])
        .arg(root.join("frameworks/TokamakRuntime.framework/TokamakRuntime"))
        .arg(&object)
        .status()?;
    if !status.success() {
        return Err(format!("test framework archive failed with {status}").into());
    }
    fs::remove_file(source)?;
    fs::remove_file(object)?;
    Ok(())
}

fn create_inputs(target: &str) -> TestResult<(tempfile::TempDir, PathBuf, PathBuf)> {
    let temporary = tempfile::tempdir()?;
    let project = temporary.path().join("project");
    let pack = temporary.path().join("pack");
    fs::create_dir_all(&project)?;
    fs::create_dir_all(&pack)?;
    create_project(&project)?;
    let platform_pack = create_platform_pack(&pack, target)?;
    Ok((temporary, project, platform_pack))
}

fn create_windows_inputs() -> TestResult<(tempfile::TempDir, PathBuf, PathBuf)> {
    let temporary = tempfile::tempdir()?;
    let project = temporary.path().join("project");
    let pack = temporary.path().join("pack");
    fs::create_dir_all(&project)?;
    create_project(&project)?;
    let target = Target::WindowsX64;
    create_windows_runtime(&pack.join(target.runtime_artifact_path()))?;
    let entrypoint_path = target.build_entrypoint_path();
    fs::create_dir_all(
        pack.join(entrypoint_path)
            .parent()
            .ok_or("entrypoint path has no parent")?,
    )?;
    let entrypoint = if cfg!(windows) {
        include_str!("../../platforms/windows/build/entrypoint.ps1")
    } else {
        r#"#!/bin/sh
set -eu
input=$2
output=$3
app_name=$(cat "$input/metadata/app-name")
app_slug=$(cat "$input/metadata/app-slug")
host=$(cat "$input/metadata/host")
rm -rf "$output"
mkdir -p "$output/app"
cp -R "$input/app/." "$output/app/"
cp "$input/runtime/TokamakRuntime/tokamak.lib" "$output/$app_slug.exe"
if [ -f "$input/icons/windows/AppIcon.ico" ]; then
  cp "$input/icons/windows/AppIcon.ico" "$output/AppIcon.ico"
fi
if [ -n "${TOKAMAK_WINDOWS_TEST:-}" ]; then
  printf '%s' "$TOKAMAK_WINDOWS_TEST" > "$output/set-value"
fi
printf '{"name":"%s","slug":"%s","host":"%s"}\n' "$app_name" "$app_slug" "$host" > "$output/tokamak.json"
"#
    };
    fs::write(pack.join(entrypoint_path), entrypoint)?;
    write_test_manifest(&pack, target)?;
    Ok((temporary, project, pack))
}

/// A runtime library for the Windows entrypoint to link, whose app exits
/// with 0 while it exports the storage part and 1 otherwise.
const WINDOWS_TEST_RUNTIME: &str = r#"
#[unsafe(export_name = "tokamak_storage")]
pub static STORAGE: u8 = 0;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut u8;
    fn GetProcAddress(module: *mut u8, name: *const u8) -> *mut u8;
}

#[unsafe(export_name = "wWinMain")]
pub extern "system" fn win_main(_: *mut u8, _: *mut u8, _: *mut u16, _: i32) -> i32 {
    let storage = unsafe {
        GetProcAddress(GetModuleHandleW(std::ptr::null()), c"tokamak_storage".as_ptr().cast())
    };
    i32::from(storage.is_null())
}
"#;

fn create_windows_runtime(runtime: &Path) -> TestResult {
    fs::create_dir_all(runtime)?;
    if cfg!(not(windows)) {
        fs::write(runtime.join("tokamak.lib"), "runtime")?;
        fs::write(runtime.join("link-libraries"), "")?;
        return Ok(());
    }
    let source = runtime.join("runtime.rs");
    fs::write(&source, WINDOWS_TEST_RUNTIME)?;
    let libraries = runtime.join("link-libraries");
    let status = ProcessCommand::new("rustc")
        .args(["--edition", "2024", "--crate-type", "staticlib", "-o"])
        .arg(runtime.join("tokamak.lib"))
        .arg("--print")
        .arg(format!("native-static-libs={}", libraries.display()))
        .arg(&source)
        .status()?;
    if !status.success() {
        return Err(format!("test runtime compilation failed with {status}").into());
    }
    fs::remove_file(source)?;
    Ok(())
}

/// Whether `binary` contains SQLite, which writes this header into every
/// database.
fn contains_sqlite(binary: &[u8]) -> bool {
    binary
        .windows(15)
        .any(|window| window == b"SQLite format 3")
}

/// A `tok build` that uses the project's earlier build.
fn build_command(platform: &str, project: &Path, platform_pack: &Path) -> TestResult<Command> {
    let mut command = project_build_command(platform, project, platform_pack)?;
    command.arg("--skip-project-build");
    Ok(command)
}

/// A `tok build` that builds the project.
fn project_build_command(
    platform: &str,
    project: &Path,
    platform_pack: &Path,
) -> TestResult<Command> {
    let mut command = Command::cargo_bin("tok")?;
    command
        .args(["build", platform, "--project"])
        .arg(project)
        .arg("--platform-pack")
        .arg(platform_pack)
        .env("TOKAMAK_VERSION", "1.0.0")
        .env_remove("TOKAMAK_IOS_BUILD_NUMBER")
        .env_remove("TOKAMAK_MACOS_BUILD_NUMBER")
        .env_remove("TOKAMAK_MACOS_TEAM_ID")
        .env_remove("TOKAMAK_ANDROID_KEYSTORE")
        .env_remove("TOKAMAK_ANDROID_KEYSTORE_PASSWORD")
        .env_remove("TOKAMAK_ANDROID_KEY_ALIAS")
        .env_remove("TOKAMAK_ANDROID_KEY_PASSWORD");
    Ok(command)
}

#[cfg(unix)]
fn configure_fake_apple_tools(command: &mut Command, root: &Path) -> TestResult<PathBuf> {
    use std::ffi::OsString;

    let bin = root.join("fake-apple-tools");
    fs::create_dir_all(&bin)?;
    write_fake_apple_tools(&bin)?;

    let log = root.join("apple-tool.log");
    let mut path = OsString::from(bin);
    path.push(":");
    path.push(std::env::var_os("PATH").ok_or("PATH unavailable")?);
    command.env("PATH", path).env("TOKAMAK_TEST_TOOL_LOG", &log);
    Ok(log)
}

/// Fake Gradle and a fake NDK compiler, whose link writes an empty library.
#[cfg(unix)]
fn configure_fake_android_tools(command: &mut Command, root: &Path) -> TestResult<()> {
    configure_fake_gradle(command, root)?;
    let ndk = root.join("fake-ndk");
    let compiler = ndk.join("toolchains/llvm/prebuilt/host/bin");
    fs::create_dir_all(&compiler)?;
    write_executable(
        &compiler.join("aarch64-linux-android31-clang"),
        r#"#!/bin/sh
set -eu
while [ "$#" -gt 0 ]; do
  if [ "$1" = -o ]; then
    : > "$2"
  fi
  shift
done
"#,
    )?;
    command.env("ANDROID_NDK_HOME", ndk);
    Ok(())
}

#[cfg(unix)]
fn configure_fake_gradle(command: &mut Command, root: &Path) -> TestResult<()> {
    use std::ffi::OsString;

    let bin = root.join("fake-android-tools");
    fs::create_dir_all(&bin)?;
    write_executable(
        &bin.join("gradle"),
        r#"#!/bin/sh
set -eu
project=
next=
for arg in "$@"; do
  if [ "$next" = project ]; then
    project=$arg
    next=
  elif [ "$arg" = --project-dir ]; then
    next=project
  fi
done
printf '%s\n' "$*" > "$project/gradle-arguments"
variant=debug
case " $* " in
  *" :app:assembleRelease "*) variant=release ;;
esac
outputs="$project/app/build/outputs/apk/$variant"
mkdir -p "$outputs"
if [ -n "${TOKAMAK_ANDROID_TEST:-}" ]; then
  printf '%s' "$TOKAMAK_ANDROID_TEST" > "$project/platform-pack-set-value"
fi
cp "$project/app/src/main/AndroidManifest.xml" "$outputs/AndroidManifest.xml"
touch "$outputs/app-$variant.apk"
"#,
    )?;
    let mut path = OsString::from(bin);
    path.push(":");
    path.push(std::env::var_os("PATH").ok_or("PATH unavailable")?);
    command.env("PATH", path);
    Ok(())
}

#[cfg(unix)]
fn write_fake_apple_tools(bin: &Path) -> TestResult {
    let xcrun = bin.join("xcrun");
    write_executable(
        &xcrun,
        r#"#!/bin/sh
set -eu

mode=
next=
compile=
partial=
output=
for arg in "$@"; do
  if [ "$arg" = actool ] || [ "$arg" = swiftc ] || [ "$arg" = strip ] || [ "$arg" = simctl ] || [ "$arg" = devicectl ]; then
    mode=$arg
  elif [ "$arg" = --compile ]; then
    next=compile
  elif [ "$arg" = --output-partial-info-plist ]; then
    next=partial
  elif [ "$arg" = -o ]; then
    next=output
  elif [ -n "$next" ]; then
    case "$next" in
      compile) compile=$arg ;;
      partial) partial=$arg ;;
      output) output=$arg ;;
    esac
    next=
  fi
done

printf '%s\n' "$*" >> "$TOKAMAK_TEST_TOOL_LOG"
case "$mode" in
  actool)
    mkdir -p "$compile"
    printf '%s\n' fake-assets > "$compile/Assets.car"
    printf '%s\n' '<?xml version="1.0" encoding="UTF-8"?>' '<plist version="1.0"><dict><key>CFBundleIconName</key><string>AppIcon</string></dict></plist>' > "$partial"
    ;;
  swiftc)
    mkdir -p "$(dirname "$output")"
    printf '%s\n' '#!/bin/sh' > "$output"
    ;;
  strip)
    ;;
  simctl)
    printf '%s\n' '{"devices":{"com.apple.CoreSimulator.SimRuntime.iOS-17-0":[]}}'
    ;;
  devicectl)
    printf '%s\n' '{"devices":[{"identifier":"DEVICE","platform":"iOS","name":"Test iPhone"}]}'
    ;;
  *)
    exit 1
    ;;
esac
"#,
    )?;
    write_executable(
        &bin.join("codesign"),
        r#"#!/bin/sh
for bundle in "$@"; do :; done
manifest="$bundle/app/worker-environment.json"
if [ -f "$bundle/Contents/Resources/app/worker-environment.json" ]; then
  manifest="$bundle/Contents/Resources/app/worker-environment.json"
fi
printf 'codesign-env %s\n' "$(cat "$manifest")" >> "$TOKAMAK_TEST_TOOL_LOG"
"#,
    )?;
    Ok(())
}

#[cfg(unix)]
fn write_executable(path: &Path, contents: &str) -> TestResult {
    use std::os::unix::fs::PermissionsExt;

    fs::create_dir_all(path.parent().ok_or("executable path has no parent")?)?;
    fs::write(path, contents)?;
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[test]
fn links_storage_into_the_macos_executable_while_the_app_declares_it() -> TestResult {
    let (_temporary, project, platform_pack) = create_inputs("macos-arm64")?;
    let executable = project.join("build/macos/demo-app.app/Contents/MacOS/demo-app");
    let exports_storage = || -> TestResult<bool> {
        build_command("macos", &project, &platform_pack)?
            .assert()
            .success();
        let symbols = ProcessCommand::new("nm")
            .arg("-gU")
            .arg(&executable)
            .output()?;
        Ok(String::from_utf8(symbols.stdout)?.contains("_tokamak_storage"))
    };

    assert!(!exports_storage()?);
    declare_storage(&project)?;
    assert!(exports_storage()?);
    Ok(())
}

#[test]
fn builds_macos_app_with_quickjs_bundle_and_assets() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("macos-arm64")?;
    build_command("macos", &project, &manifest)?
        .assert()
        .success()
        .stdout(contains("Built macOS bundle"));

    let bundle = project.join("build/macos/demo-app.app");
    let app = bundle.join("Contents/Resources/app");
    assert!(bundle.join("Contents/MacOS/demo-app").is_file());
    assert!(
        !bundle
            .join("Contents/Frameworks/TokamakRuntime.framework")
            .exists()
    );
    let manifest = read_worker_manifest(&PackageLayout::new(&app))?;
    assert_eq!(manifest.entry, "index.js");
    assert_eq!(
        read_worker_module(&PackageLayout::new(&app), "index.js")?,
        compile_module("index.js", &fs::read(project.join("dist/app/index.js"))?)?
    );
    assert!(app.join("assets/index.html").is_file());
    let plist = fs::read_to_string(bundle.join("Contents/Info.plist"))?;
    assert!(plist.contains("NSAllowsLocalNetworking"));
    assert!(!bundle.join("Contents/Resources/Assets.car").exists());
    assert!(!plist.contains("CFBundleIconName"));
    assert!(!plist.contains("CFBundleIconFile"));
    let manifest = read_asset_manifest(&PackageLayout::new(&app))?;
    assert_eq!(manifest.files["styles/app.css"], "text/css");
    assert!(!app.join("config.capnp").exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn builds_configured_identifier_and_version() -> TestResult {
    let (temporary, project, manifest) = create_inputs("macos-arm64")?;
    configure(
        &project,
        r#"{
  "identifier": "com.example.app",
  "macos": { "identifier": "com.example.desktop" },
  "version": "2.3.4"
}"#,
    )?;

    let mut command = build_command("macos", &project, &manifest)?;
    command.env_remove("TOKAMAK_VERSION");
    configure_fake_apple_tools(&mut command, temporary.path())?;
    command.assert().success();

    let plist = fs::read_to_string(project.join("build/macos/demo-app.app/Contents/Info.plist"))?;
    assert!(plist.contains("<key>CFBundleIdentifier</key><string>com.example.desktop</string>"));
    assert!(plist.contains("<key>CFBundleVersion</key><string>2.3.4</string>"));
    assert!(plist.contains("<key>CFBundleShortVersionString</key><string>2.3.4</string>"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn passes_the_configuration_file_to_the_build() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("macos-arm64")?;
    project_build_command("macos", &project, &manifest)?
        .current_dir(&project)
        .args(["--config", "test.ts", "--build"])
        .arg(r#"node build.cjs && printf %s "$TOKAMAK_CONFIG" > config-path"#)
        .assert()
        .success();

    assert_eq!(
        PathBuf::from(fs::read_to_string(project.join("config-path"))?),
        fs::canonicalize(&project)?.join("test.ts")
    );
    Ok(())
}

#[test]
fn requires_the_tokamak_vite_plugin() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("macos-arm64")?;
    project_build_command("macos", &project, &manifest)?
        .args(["--build", "node --version"])
        .assert()
        .failure()
        .stderr(contains(
            "the build did not report its tokamak configuration; the Vite config must include tokamak() from @tokamakdev/tok/vite",
        ));
    Ok(())
}

#[cfg(unix)]
#[test]
fn preserves_configured_display_name_in_apple_bundle() -> TestResult {
    let (temporary, project, manifest) = create_inputs("ios-simulator-arm64")?;
    configure(&project, r#"{"name":"Vigilus"}"#)?;

    let mut command = build_command("ios-simulator", &project, &manifest)?;
    configure_fake_apple_tools(&mut command, temporary.path())?;
    command.assert().success();

    let bundle = project.join("build/ios-simulator/vigilus.app");
    assert!(bundle.join("vigilus").is_file());
    let plist = fs::read_to_string(bundle.join("Info.plist"))?;
    assert!(plist.contains("<key>CFBundleName</key><string>Vigilus</string>"));
    assert!(plist.contains("<key>CFBundleDisplayName</key><string>Vigilus</string>"));
    assert!(plist.contains("<key>CFBundleExecutable</key><string>vigilus</string>"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn preserves_configured_display_name_in_macos_bundle() -> TestResult {
    let (temporary, project, manifest) = create_inputs("macos-arm64")?;
    configure(&project, r#"{"name":"Vigilus"}"#)?;

    let mut command = build_command("macos", &project, &manifest)?;
    configure_fake_apple_tools(&mut command, temporary.path())?;
    command.assert().success();

    let bundle = project.join("build/macos/vigilus.app");
    assert!(bundle.join("Contents/MacOS/vigilus").is_file());
    let plist = fs::read_to_string(bundle.join("Contents/Info.plist"))?;
    assert!(plist.contains("<key>CFBundleName</key><string>Vigilus</string>"));
    assert!(plist.contains("<key>CFBundleDisplayName</key><string>Vigilus</string>"));
    assert!(plist.contains("<key>CFBundleExecutable</key><string>vigilus</string>"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn preserves_configured_display_name_in_android_manifest() -> TestResult {
    let (temporary, project, platform_pack) = create_android_inputs()?;
    configure(&project, r#"{"name":"Vigilus & <Co> \"Pro\" 'X'"}"#)?;

    let mut command = build_command("android", &project, &platform_pack)?;
    configure_fake_android_tools(&mut command, temporary.path())?;
    command.assert().success();

    let manifest = fs::read_to_string(
        project.join("build/android/.tokamak/app/src/main/AndroidManifest.xml"),
    )?;
    assert!(
        manifest
            .contains("android:label=\"Vigilus &amp; &lt;Co&gt; &quot;Pro&quot; &apos;X&apos;\"")
    );
    assert!(!manifest.contains("android:label=\"vigilus-co-pro-x\""));
    assert!(project.join("build/android/vigilus-co-pro-x.apk").is_file());
    Ok(())
}

#[cfg(unix)]
#[test]
fn links_storage_into_the_android_runtime_while_the_app_declares_it() -> TestResult {
    let (temporary, project, platform_pack) = create_android_inputs()?;
    let exports = project.join("build/android/.tokamak/tokamak-exports.map");
    let build = || -> TestResult<String> {
        let mut command = build_command("android", &project, &platform_pack)?;
        configure_fake_android_tools(&mut command, temporary.path())?;
        command.assert().success();
        Ok(fs::read_to_string(&exports)?)
    };

    let without_storage = build()?;
    assert!(without_storage.contains("    Java_*;\n"));
    assert!(!without_storage.contains("tokamak_storage"));
    declare_storage(&project)?;
    let with_storage = build()?;
    assert!(with_storage.contains("    Java_*;\n"));
    assert!(with_storage.contains("    tokamak_storage;\n"));
    assert!(
        project
            .join("build/android/.tokamak/app/src/main/jniLibs/arm64-v8a/libtokamak.so")
            .is_file()
    );
    Ok(())
}

#[cfg(unix)]
#[test]
#[ignore = "needs an Android platform pack at TOKAMAK_TEST_ANDROID_PACK and an NDK at ANDROID_NDK_HOME"]
fn links_storage_from_the_android_platform_pack_while_the_app_declares_it() -> TestResult {
    let platform_pack = PathBuf::from(std::env::var("TOKAMAK_TEST_ANDROID_PACK")?);
    let ndk = PathBuf::from(std::env::var("ANDROID_NDK_HOME")?);
    let toolchain = fs::read_dir(ndk.join("toolchains/llvm/prebuilt"))?
        .next()
        .ok_or("the NDK has no prebuilt toolchain")??
        .path();
    let temporary = tempfile::tempdir()?;
    let project = temporary.path().join("project");
    fs::create_dir_all(&project)?;
    create_project(&project)?;
    let library =
        project.join("build/android/.tokamak/app/src/main/jniLibs/arm64-v8a/libtokamak.so");
    let links_storage = || -> TestResult<(bool, bool)> {
        let mut command = build_command("android", &project, &platform_pack)?;
        configure_fake_gradle(&mut command, temporary.path())?;
        command.assert().success();
        let symbols = ProcessCommand::new(toolchain.join("bin/llvm-nm"))
            .args(["--dynamic", "--defined-only"])
            .arg(&library)
            .output()?;
        if !symbols.status.success() {
            return Err(format!("llvm-nm failed with {}", symbols.status).into());
        }
        let symbols = String::from_utf8(symbols.stdout)?;
        assert!(symbols.contains(" Java_"), "{symbols}");
        let contents = fs::read(&library)?;
        println!("{}: {} bytes", library.display(), contents.len());
        Ok((
            symbols.contains(tokamak::STORAGE_ENTRY_POINT),
            contains_sqlite(&contents),
        ))
    };

    assert_eq!(links_storage()?, (false, false));
    declare_storage(&project)?;
    assert_eq!(links_storage()?, (true, true));
    Ok(())
}

#[cfg(unix)]
#[test]
fn passes_platform_pack_variables_to_the_android_entrypoint() -> TestResult {
    let (temporary, project, platform_pack) = create_android_inputs()?;

    let mut command = build_command("android", &project, &platform_pack)?;
    command.args(["--android-test", "passed"]);
    configure_fake_android_tools(&mut command, temporary.path())?;
    command.assert().success();

    assert_eq!(
        fs::read_to_string(project.join("build/android/.tokamak/platform-pack-set-value"))?,
        "passed"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn passes_options_then_environment_then_configured_values_to_the_entrypoint() -> TestResult {
    let (temporary, project, platform_pack) = create_android_inputs()?;
    configure(&project, r#"{ "android": { "test": "configured" } }"#)?;
    let set_value = project.join("build/android/.tokamak/platform-pack-set-value");
    let build = |configure: &dyn Fn(&mut Command)| -> TestResult<String> {
        let mut command = build_command("android", &project, &platform_pack)?;
        command.env_remove("TOKAMAK_ANDROID_TEST");
        configure(&mut command);
        configure_fake_android_tools(&mut command, temporary.path())?;
        command.assert().success();
        Ok(fs::read_to_string(&set_value)?)
    };

    assert_eq!(build(&|_| {})?, "configured");
    assert_eq!(
        build(&|command| {
            command.env("TOKAMAK_ANDROID_TEST", "environment");
        })?,
        "environment"
    );
    assert_eq!(
        build(&|command| {
            command
                .env("TOKAMAK_ANDROID_TEST", "environment")
                .args(["--android-test", "option"]);
        })?,
        "option"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn rejects_keys_the_platform_pack_does_not_declare() -> TestResult {
    let (temporary, project, platform_pack) = create_android_inputs()?;
    configure(
        &project,
        r#"{ "android": { "tset": "x" }, "ios": { "tset": "ignored" } }"#,
    )?;

    let mut command = build_command("android", &project, &platform_pack)?;
    configure_fake_android_tools(&mut command, temporary.path())?;
    command
        .assert()
        .failure()
        .stderr(contains("unknown key android.tset in"));

    let mut command = build_command("android", &project, &platform_pack)?;
    command.args(["--android-manifset", "AndroidManifest.xml"]);
    configure_fake_android_tools(&mut command, temporary.path())?;
    command.assert().failure().stderr(contains(
        "unknown option --android-manifset; Android accepts",
    ));
    Ok(())
}

#[cfg(unix)]
#[test]
fn lists_the_platform_pack_options_in_build_help() -> TestResult {
    let (_temporary, _project, platform_pack) = create_android_inputs()?;
    Command::cargo_bin("tok")?
        .args(["build", "android", "--platform-pack"])
        .arg(&platform_pack)
        .arg("--help")
        .assert()
        .success()
        .stdout(
            contains(
                "Android options (also TOKAMAK_ANDROID_<KEY>, or android.<key> in the configuration file):",
            )
            .and(contains("--android-identifier <VALUE>"))
            .and(contains("--android-manifest <PATH>"))
            .and(contains("--android-test <VALUE>")),
        );
    Command::cargo_bin("tok")?
        .args(["dev", "android", "--platform-pack"])
        .arg(&platform_pack)
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("--android-manifest <PATH>"));
    Command::cargo_bin("tok")?
        .args(["build", "--help"])
        .assert()
        .success()
        .stdout(contains(
            "run `tok build <platforms> --help` to list the options",
        ));
    Ok(())
}

#[cfg(unix)]
#[test]
fn checks_settings_before_building_the_project() -> TestResult {
    let (_temporary, project, platform_pack) = create_android_inputs()?;
    Command::cargo_bin("tok")?
        .args(["build", "android", "--project"])
        .arg(&project)
        .arg("--platform-pack")
        .arg(&platform_pack)
        .args(["--version", "1.0.0", "--build", "touch project-built"])
        .args(["--android-manifset", "AndroidManifest.xml"])
        .assert()
        .failure()
        .stderr(contains("unknown option --android-manifset"));
    assert!(!project.join("project-built").exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn checks_android_api_levels_with_lint() -> TestResult {
    let (temporary, project, platform_pack) = create_android_inputs()?;

    let mut command = build_command("android", &project, &platform_pack)?;
    configure_fake_android_tools(&mut command, temporary.path())?;
    command.assert().success();

    let gradle = project.join("build/android/.tokamak");
    let build_script = fs::read_to_string(gradle.join("app/build.gradle"))?;
    assert!(build_script.contains("checkOnly 'NewApi'"));
    assert!(build_script.contains("abortOnError true"));
    let arguments = fs::read_to_string(gradle.join("gradle-arguments"))?;
    assert!(arguments.contains(":app:lintRelease :app:assembleRelease"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn shrinks_release_builds_and_signs_them_with_the_debug_key_by_default() -> TestResult {
    let (temporary, project, platform_pack) = create_android_inputs()?;

    let mut command = build_command("android", &project, &platform_pack)?;
    configure_fake_android_tools(&mut command, temporary.path())?;
    command.assert().success();

    let app = project.join("build/android/.tokamak/app");
    let build_script = fs::read_to_string(app.join("build.gradle"))?;
    assert!(build_script.contains("minifyEnabled true"));
    assert!(build_script.contains("signingConfig signingConfigs.debug"));
    assert!(!build_script.contains("signingConfigs {"));
    assert_eq!(
        fs::read_to_string(app.join("tokamak-rules.pro"))?,
        "-dontobfuscate\n"
    );
    assert!(project.join("build/android/demo-app.apk").is_file());
    Ok(())
}

#[cfg(unix)]
#[test]
fn signs_release_builds_with_the_configured_keystore() -> TestResult {
    let (temporary, project, platform_pack) = create_android_inputs()?;
    let keystore = temporary.path().join("release.keystore");
    fs::write(&keystore, "keystore")?;
    let build = |alias: Option<&str>| -> TestResult<Command> {
        let mut command = build_command("android", &project, &platform_pack)?;
        command
            .env("TOKAMAK_ANDROID_KEYSTORE", &keystore)
            .env("TOKAMAK_ANDROID_KEYSTORE_PASSWORD", "secret")
            .env_remove("TOKAMAK_ANDROID_KEY_ALIAS");
        if let Some(alias) = alias {
            command.env("TOKAMAK_ANDROID_KEY_ALIAS", alias);
        }
        configure_fake_android_tools(&mut command, temporary.path())?;
        Ok(command)
    };

    let missing = "an Android keystore needs key-alias and keystore-password";
    build(None)?.assert().failure().stderr(contains(missing));
    build(Some("release"))?
        .env_remove("TOKAMAK_ANDROID_KEYSTORE_PASSWORD")
        .assert()
        .failure()
        .stderr(contains(missing));
    build(Some("release"))?
        .env(
            "TOKAMAK_ANDROID_KEYSTORE",
            temporary.path().join("missing.keystore"),
        )
        .assert()
        .failure()
        .stderr(contains("Android keystore is missing"));
    build(Some("release"))?.assert().success();

    let build_script = fs::read_to_string(project.join("build/android/.tokamak/app/build.gradle"))?;
    assert!(build_script.contains("signingConfig signingConfigs.release"));
    assert!(build_script.contains("storeFile file(System.getenv('TOKAMAK_ANDROID_KEYSTORE'))"));
    assert!(build_script.contains(
        "keyPassword System.getenv('TOKAMAK_ANDROID_KEY_PASSWORD') ?: System.getenv('TOKAMAK_ANDROID_KEYSTORE_PASSWORD')"
    ));
    assert!(!build_script.contains("secret"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn rejects_android_signing_settings_without_a_keystore() -> TestResult {
    let (temporary, project, platform_pack) = create_android_inputs()?;
    let mut command = build_command("android", &project, &platform_pack)?;
    command.env("TOKAMAK_ANDROID_KEY_ALIAS", "release");
    configure_fake_android_tools(&mut command, temporary.path())?;

    command
        .assert()
        .failure()
        .stderr(contains("Android key-alias and passwords need a keystore"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn builds_development_android_apps_as_debug_builds() -> TestResult {
    let (temporary, project, platform_pack) = create_android_inputs()?;
    let mut command = build_command("android", &project, &platform_pack)?;
    configure_fake_android_tools(&mut command, temporary.path())?;
    command.assert().success();
    let input = project.join("build/.tokamak/android/input");
    fs::write(input.join("metadata/dev-endpoint"), "http://127.0.0.1:9")?;
    fs::write(input.join("metadata/dev-session-token"), "token")?;
    let output = temporary.path().join("development/app.apk");

    // Development builds ignore release signing settings.
    let mut entrypoint = Command::new("bash");
    entrypoint
        .arg(platform_pack.join(Target::AndroidArm64.build_entrypoint_path()))
        .arg("build")
        .arg(&input)
        .arg(&output)
        .env(
            "TOKAMAK_ANDROID_KEYSTORE",
            temporary.path().join("missing.keystore"),
        );
    configure_fake_android_tools(&mut entrypoint, temporary.path())?;
    entrypoint.assert().success();

    let gradle = temporary.path().join("development/.tokamak");
    let arguments = fs::read_to_string(gradle.join("gradle-arguments"))?;
    assert!(arguments.contains(":app:lintDebug :app:assembleDebug"));
    assert!(fs::read_to_string(gradle.join("app/build.gradle"))?.contains("signingConfigs.debug"));
    assert!(output.is_file());
    Ok(())
}

#[cfg(unix)]
#[test]
fn merges_the_app_android_manifest_while_it_is_set() -> TestResult {
    let (temporary, project, platform_pack) = create_android_inputs()?;
    let user_manifest = r#"<manifest xmlns:android="http://schemas.android.com/apk/res/android"><uses-permission android:name="android.permission.CAMERA" /></manifest>"#;
    fs::create_dir_all(project.join("native"))?;
    fs::write(project.join("native/AndroidManifest.xml"), user_manifest)?;
    let app = project.join("build/android/.tokamak/app");

    let mut command = build_command("android", &project, &platform_pack)?;
    command
        .current_dir(&project)
        .args(["--android-manifest", "native/AndroidManifest.xml"]);
    configure_fake_android_tools(&mut command, temporary.path())?;
    command.assert().success();

    assert_eq!(
        fs::read_to_string(app.join("user/AndroidManifest.xml"))?,
        user_manifest
    );
    assert!(
        fs::read_to_string(app.join("build.gradle"))?
            .contains("addStaticManifestFile(file('user/AndroidManifest.xml').path)")
    );

    let mut command = build_command("android", &project, &platform_pack)?;
    configure_fake_android_tools(&mut command, temporary.path())?;
    command.assert().success();

    assert!(!app.join("user").exists());
    assert!(!fs::read_to_string(app.join("build.gradle"))?.contains("addStaticManifestFile"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn rejects_a_missing_app_android_manifest() -> TestResult {
    let (temporary, project, platform_pack) = create_android_inputs()?;

    let mut command = build_command("android", &project, &platform_pack)?;
    command
        .current_dir(&project)
        .args(["--android-manifest", "native/AndroidManifest.xml"]);
    configure_fake_android_tools(&mut command, temporary.path())?;
    command
        .assert()
        .failure()
        .stderr(contains("Android manifest file is missing"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn builds_each_android_plugin_as_a_library_module() -> TestResult {
    let (temporary, project, platform_pack) = create_android_inputs()?;
    install_android_plugins(&project, &[("alerts", &[])])?;
    let plugin = project.join("node_modules/alerts");
    let plugin_manifest = r#"<manifest><application><service android:name="test.alerts.Service" /></application></manifest>"#;
    fs::write(plugin.join("android/AndroidManifest.xml"), plugin_manifest)?;
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(plugin.join("tokamak-plugin.json"))?)?;
    manifest["platforms"]["android"]["manifest"] = "android/AndroidManifest.xml".into();
    manifest["platforms"]["android"]["dependencies"] =
        serde_json::json!(["com.example:messaging:1.2.3"]);
    fs::write(plugin.join("tokamak-plugin.json"), manifest.to_string())?;

    let mut command = build_command("android", &project, &platform_pack)?;
    configure_fake_android_tools(&mut command, temporary.path())?;
    command.assert().success();

    let gradle = project.join("build/android/.tokamak");
    let module = gradle.join("plugins/alerts");
    assert_eq!(
        fs::read_to_string(module.join("src/main/AndroidManifest.xml"))?,
        plugin_manifest
    );
    assert!(module.join("src/main/kotlin/0-Plugin.kt").is_file());
    let module_script = fs::read_to_string(module.join("build.gradle"))?;
    assert!(module_script.contains("implementation project(':tokamak-plugin')"));
    assert!(module_script.contains("implementation 'com.example:messaging:1.2.3'"));
    assert!(
        fs::read_to_string(gradle.join("settings.gradle"))?.contains("include ':plugins:alerts'")
    );
    let app_script = fs::read_to_string(gradle.join("app/build.gradle"))?;
    assert!(app_script.contains("implementation project(':plugins:alerts')"));
    assert!(app_script.contains("checkDependencies true"));
    assert!(
        fs::read_to_string(
            gradle.join("app/src/main/kotlin/com/tokamak/runtime/TokamakPluginRegistry.kt")
        )?
        .contains("test.alerts.Plugin(host),")
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn writes_each_android_permission_once() -> TestResult {
    let (temporary, project, platform_pack) = create_android_inputs()?;
    install_android_plugins(
        &project,
        &[
            ("storage", &["android.permission.USE_BIOMETRIC"]),
            (
                "authentication",
                &[
                    "android.permission.USE_BIOMETRIC",
                    "android.permission.INTERNET",
                ],
            ),
        ],
    )?;

    let mut command = build_command("android", &project, &platform_pack)?;
    configure_fake_android_tools(&mut command, temporary.path())?;
    command.assert().success();

    let manifest = fs::read_to_string(
        project.join("build/android/.tokamak/app/src/main/AndroidManifest.xml"),
    )?;
    assert_eq!(
        manifest.matches("android.permission.USE_BIOMETRIC").count(),
        1
    );
    assert_eq!(manifest.matches("android.permission.INTERNET").count(), 1);
    Ok(())
}

#[cfg(unix)]
#[test]
fn environment_overrides_configured_identifier_and_version() -> TestResult {
    let (temporary, project, manifest) = create_inputs("macos-arm64")?;
    configure(
        &project,
        r#"{
  "identifier": "com.example.config",
  "macos": { "identifier": "com.example.config-macos" },
  "version": "2.3.4"
}"#,
    )?;

    let mut command = build_command("macos", &project, &manifest)?;
    command
        .env("TOKAMAK_MACOS_IDENTIFIER", "com.example.environment")
        .env("TOKAMAK_VERSION", "3.4.5");
    configure_fake_apple_tools(&mut command, temporary.path())?;
    command.assert().success();

    let plist = fs::read_to_string(project.join("build/macos/demo-app.app/Contents/Info.plist"))?;
    assert!(
        plist.contains("<key>CFBundleIdentifier</key><string>com.example.environment</string>")
    );
    assert!(plist.contains("<key>CFBundleVersion</key><string>3.4.5</string>"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn builds_configured_apple_icon_packages() -> TestResult {
    for (platform, target, bundle_path, assets_path, sdk) in [
        (
            "macos",
            "macos-arm64",
            "build/macos/demo-app.app",
            "Contents/Resources/Assets.car",
            "macosx",
        ),
        (
            "ios-simulator",
            "ios-simulator-arm64",
            "build/ios-simulator/demo-app.app",
            "Assets.car",
            "iphonesimulator",
        ),
    ] {
        let (temporary, project, manifest) = create_inputs(target)?;
        let icon = project.join("src/assets/AppIcon.icon");
        fs::create_dir_all(&icon)?;
        fs::write(icon.join("icon.json"), "{}")?;
        configure(&project, r#"{ "icon": "assets/AppIcon.icon" }"#)?;

        let mut command = build_command(platform, &project, &manifest)?;
        let tool_log = configure_fake_apple_tools(&mut command, temporary.path())?;
        command.assert().success();

        let bundle = project.join(bundle_path);
        assert!(bundle.join(assets_path).is_file());
        let plist = fs::read_to_string(bundle.join(if platform == "macos" {
            "Contents/Info.plist"
        } else {
            "Info.plist"
        }))?;
        assert!(plist.contains("CFBundleIconName"));
        assert!(plist.contains("AppIcon"));
        assert!(!plist.contains("CFBundleIconFile"));
        let log = fs::read_to_string(tool_log)?;
        assert!(log.contains("--app-icon AppIcon"));
        assert!(log.contains(&format!("--platform {sdk}")));
        assert!(log.contains("AppIcon.icon"));
    }
    Ok(())
}

#[test]
fn compiles_es_modules_and_copies_text_and_data_modules() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("macos-arm64")?;
    let worker = project.join("dist/app");
    fs::create_dir_all(worker.join("assets"))?;
    fs::write(
        worker.join("index.js"),
        "import page from './assets/page.html'; export default { async fetch() { const { value } = await import('./assets/lazy.js'); return new Response(page + value); } };",
    )?;
    fs::write(worker.join("assets/lazy.js"), "export const value = 1;")?;
    fs::write(worker.join("assets/page.html"), "<p>page</p>")?;
    fs::write(worker.join("assets/data.bin"), [0, 1, 2])?;
    fs::write(worker.join("index.js.map"), "{}")?;

    build_command("macos", &project, &manifest)?
        .assert()
        .success();

    let app = project.join("build/macos/demo-app.app/Contents/Resources/app");
    let manifest = read_worker_manifest(&PackageLayout::new(&app))?;
    assert_eq!(manifest.entry, "index.js");
    assert_eq!(
        manifest.modules,
        [
            ("assets/data.bin", ModuleType::Data),
            ("assets/lazy.js", ModuleType::EsModule),
            ("assets/page.html", ModuleType::Text),
            ("index.js", ModuleType::EsModule),
        ]
        .map(|(name, module_type)| (name.to_owned(), module_type))
        .into()
    );
    assert_eq!(
        read_worker_module(&PackageLayout::new(&app), "assets/lazy.js")?,
        compile_module("assets/lazy.js", &fs::read(worker.join("assets/lazy.js"))?)?
    );
    assert_eq!(
        fs::read(app.join("bundle/assets/page.html"))?,
        b"<p>page</p>"
    );
    assert_eq!(fs::read(app.join("bundle/assets/data.bin"))?, [0, 1, 2]);
    Ok(())
}

#[test]
fn rejects_a_webassembly_module() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("macos-arm64")?;
    fs::write(project.join("dist/app/add.wasm"), [0, 0x61, 0x73, 0x6d])?;

    build_command("macos", &project, &manifest)?
        .assert()
        .failure()
        .stderr(contains(
            "Worker module add.wasm is a CompiledWasm module; tokamak supports ESModule, Text and Data modules",
        ));
    Ok(())
}

#[test]
fn warns_about_bindings_the_packaged_app_lacks() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("macos-arm64")?;
    let build_warnings = || -> TestResult<String> {
        let output = build_command("macos", &project, &manifest)?
            .assert()
            .success();
        Ok(String::from_utf8(output.get_output().stderr.clone())?)
    };
    assert!(!build_warnings()?.contains("WARNING"));

    write_worker_config(
        &project,
        r#"{
            "kv_namespaces": [{ "binding": "SESSION", "id": "session" }],
            "durable_objects": { "bindings": [{ "name": "ROOMS", "class_name": "Room" }] }
        }"#,
    )?;
    let warnings = build_warnings()?;
    assert!(
        warnings.contains(
            "WARNING: this app declares bindings that the packaged app does not provide."
        ) && warnings.contains(
            "  - ROOMS (durable_objects): the packaged app does not provide Durable Objects"
        ) && !warnings.contains("SESSION"),
        "{warnings}"
    );
    Ok(())
}

#[test]
fn packages_worker_vars_as_a_normalized_manifest() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("macos-arm64")?;
    write_worker_config(
        &project,
        r#"{ "vars": { "TEXT": "value", "JSON": { "enabled": true } } }"#,
    )?;

    build_command("macos", &project, &manifest)?
        .assert()
        .success();

    let manifest: serde_json::Value = serde_json::from_slice(&fs::read(
        project.join("build/macos/demo-app.app/Contents/Resources/app/worker-environment.json"),
    )?)?;
    assert_eq!(manifest["vars"]["TEXT"], "value");
    assert_eq!(
        manifest["vars"]["JSON"],
        serde_json::json!({ "enabled": true })
    );
    Ok(())
}

#[test]
fn packages_each_generated_configuration_without_recompiling_unchanged_modules() -> TestResult {
    let (_temporary, project, pack) = create_windows_inputs()?;
    let build_dir = project.join(".cache/tokamak");
    let app = build_dir.join("windows/demo-app/app");
    let packaged_vars = |vars: &str| -> TestResult<serde_json::Value> {
        write_worker_config(&project, &format!(r#"{{ "vars": {vars} }}"#))?;
        project_build_command("windows", &project, &pack)?
            .args(["--build-dir", ".cache/tokamak"])
            .assert()
            .success();
        let environment: serde_json::Value =
            serde_json::from_slice(&fs::read(app.join("worker-environment.json"))?)?;
        Ok(environment["vars"].clone())
    };
    assert_eq!(
        packaged_vars(r#"{ "STAGE": "test" }"#)?,
        serde_json::json!({ "STAGE": "test" })
    );
    let compiled = worker_compiled_at(&build_dir)?;

    assert_eq!(
        packaged_vars(r#"{ "STAGE": "production" }"#)?,
        serde_json::json!({ "STAGE": "production" })
    );
    assert_eq!(worker_compiled_at(&build_dir)?, compiled);

    let entry = project.join("dist/app/index.js");
    fs::write(&entry, "export default { value: 2 };")?;
    packaged_vars(r#"{ "STAGE": "production" }"#)?;
    assert_ne!(worker_compiled_at(&build_dir)?, compiled);
    assert_eq!(
        read_worker_module(&PackageLayout::new(&app), "index.js")?,
        compile_module("index.js", &fs::read(&entry)?)?
    );
    Ok(())
}

/// When the Worker of the build in `build_dir` was last compiled.
fn worker_compiled_at(build_dir: &Path) -> TestResult<std::time::SystemTime> {
    let compiled = PackageLayout::new(build_dir.join(".tokamak/worker/compiled"));
    Ok(fs::metadata(compiled.worker_manifest())?.modified()?)
}

#[test]
fn updates_changed_assets_without_recompiling_the_worker() -> TestResult {
    let (_temporary, project, pack) = create_windows_inputs()?;
    let build = || -> TestResult {
        build_command("windows", &project, &pack)?
            .assert()
            .success();
        Ok(())
    };

    build()?;
    let compiled = worker_compiled_at(&project.join("build"))?;
    fs::write(
        project.join("dist/client/index.html"),
        "<html>updated</html>",
    )?;
    build()?;

    assert_eq!(
        fs::read_to_string(project.join("build/windows/demo-app/app/assets/index.html"))?,
        "<html>updated</html>"
    );
    assert_eq!(worker_compiled_at(&project.join("build"))?, compiled);
    Ok(())
}

#[test]
fn ignores_unrelated_files_when_reusing_worker_modules() -> TestResult {
    let (_temporary, project, pack) = create_windows_inputs()?;
    let worker = project.join("dist/app");
    fs::write(worker.join("settings.json"), r#"{"feature":true}"#)?;
    write_worker_config(
        &project,
        r#"{ "rules": [{ "type": "ESModule", "globs": ["**/*.js"] }, { "type": "Data", "globs": ["**/*.json"] }] }"#,
    )?;
    let build = || -> TestResult {
        build_command("windows", &project, &pack)?
            .assert()
            .success();
        Ok(())
    };

    build()?;
    let compiled = worker_compiled_at(&project.join("build"))?;
    fs::write(worker.join("unrelated.md"), "changed")?;
    build()?;
    assert_eq!(worker_compiled_at(&project.join("build"))?, compiled);

    fs::write(worker.join("settings.json"), r#"{"feature":false}"#)?;
    build()?;
    assert_ne!(worker_compiled_at(&project.join("build"))?, compiled);
    assert_eq!(
        fs::read(project.join("build/windows/demo-app/app/bundle/settings.json"))?,
        br#"{"feature":false}"#
    );
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn apple_env_only_build_reuses_the_native_bundle() -> TestResult {
    for (platform, target, environment_path) in [
        (
            "macos",
            "macos-arm64",
            "build/macos/demo-app.app/Contents/Resources/app/worker-environment.json",
        ),
        (
            "ios",
            "ios-arm64",
            "build/ios/demo-app.app/app/worker-environment.json",
        ),
    ] {
        let (temporary, project, pack) = create_inputs(target)?;
        write_worker_config(&project, r#"{ "vars": { "API": "test" } }"#)?;
        let mut test = build_command(platform, &project, &pack)?;
        let log = configure_fake_apple_tools(&mut test, temporary.path())?;
        test.assert().success();

        write_worker_config(&project, r#"{ "vars": { "API": "production" } }"#)?;
        let mut production = build_command(platform, &project, &pack)?;
        configure_fake_apple_tools(&mut production, temporary.path())?;
        if platform == "ios" {
            let profile = project.join("release.mobileprovision");
            fs::write(&profile, "release-profile")?;
            production
                .env_remove("TOKAMAK_IOS_TEAM_ID")
                .env("TOKAMAK_IOS_SIGNING_IDENTITY", "Apple Distribution: Test")
                .env("TOKAMAK_IOS_PROVISIONING_PROFILE", profile);
        }
        production.assert().success();

        let commands = fs::read_to_string(log)?;
        assert_eq!(
            commands
                .lines()
                .filter(|line| line.contains(" swiftc "))
                .count(),
            1
        );
        let signatures = commands
            .lines()
            .filter_map(|line| line.strip_prefix("codesign-env "))
            .collect::<Vec<_>>();
        assert_eq!(signatures.len(), 2);
        let final_environment = fs::read_to_string(project.join(environment_path))?;
        assert_eq!(signatures[1], final_environment.trim_end());
        let environment: serde_json::Value = serde_json::from_str(&final_environment)?;
        assert_eq!(environment["vars"]["API"], "production");
        if platform == "ios" {
            assert_eq!(
                fs::read_to_string(
                    project.join("build/ios/demo-app.app/embedded.mobileprovision")
                )?,
                "release-profile"
            );
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn macos_team_signing_changes_without_rebuilding_the_bundle() -> TestResult {
    let (temporary, project, pack) = create_inputs("macos-arm64")?;
    let profile = project.join("build/macos/demo-app.app/Contents/embedded.provisionprofile");

    let mut team_signed = build_command("macos", &project, &pack)?;
    let log = configure_fake_apple_tools(&mut team_signed, temporary.path())?;
    team_signed
        .env("TOKAMAK_MACOS_TEAM_ID", "TEAM")
        .assert()
        .success();
    assert_eq!(fs::read_to_string(&profile)?, "TEAM");

    let mut ad_hoc = build_command("macos", &project, &pack)?;
    configure_fake_apple_tools(&mut ad_hoc, temporary.path())?;
    ad_hoc.assert().success();
    assert!(!profile.exists());

    let commands = fs::read_to_string(log)?;
    assert_eq!(
        commands
            .lines()
            .filter(|line| line.contains(" swiftc "))
            .count(),
        1
    );
    assert_eq!(
        commands
            .lines()
            .filter(|line| line.starts_with("codesign-env "))
            .count(),
        2
    );
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn apple_build_number_change_reuses_the_native_bundle() -> TestResult {
    for (platform, target) in [
        ("macos", "macos-arm64"),
        ("ios", "ios-arm64"),
        ("ios-simulator", "ios-simulator-arm64"),
    ] {
        let (temporary, project, pack) = create_inputs(target)?;
        let icon = project.join("src/assets/AppIcon.icon");
        fs::create_dir_all(&icon)?;
        fs::write(icon.join("icon.json"), "{}")?;
        configure(&project, r#"{"icon":"assets/AppIcon.icon"}"#)?;
        let build_number = if platform == "macos" {
            "TOKAMAK_MACOS_BUILD_NUMBER"
        } else {
            "TOKAMAK_IOS_BUILD_NUMBER"
        };

        let build = |number, environment| -> TestResult {
            let vars = format!(r#"{{ "vars": {{ "API": "{environment}" }} }}"#);
            write_worker_config(&project, &vars)?;
            let mut command = build_command(platform, &project, &pack)?;
            configure_fake_apple_tools(&mut command, temporary.path())?;
            command.env(build_number, number).assert().success();
            Ok(())
        };
        build("1", "test")?;
        build("2", "production")?;
        build("3", "production")?;

        let commands = fs::read_to_string(temporary.path().join("apple-tool.log"))?;
        assert_eq!(
            commands
                .lines()
                .filter(|line| line.contains(" swiftc "))
                .count(),
            1
        );
        assert_eq!(
            commands
                .lines()
                .filter(|line| line.contains(" actool "))
                .count(),
            1
        );
        let signatures = commands
            .lines()
            .filter_map(|line| line.strip_prefix("codesign-env "))
            .collect::<Vec<_>>();
        assert_eq!(signatures.len(), 3);
        let bundle = project.join(format!("build/{platform}/demo-app.app"));
        let plist = fs::read_to_string(bundle.join(if platform == "macos" {
            "Contents/Info.plist"
        } else {
            "Info.plist"
        }))?;
        assert!(plist.contains("<key>CFBundleVersion</key><string>3</string>"));
        assert!(plist.contains("<key>CFBundleIconName</key><string>AppIcon</string>"));
        let app_dir = bundle.join(if platform == "macos" {
            "Contents/Resources/app"
        } else {
            "app"
        });
        let final_environment = fs::read_to_string(app_dir.join("worker-environment.json"))?;
        assert_eq!(signatures[2], final_environment.trim_end());
        let environment: serde_json::Value = serde_json::from_str(&final_environment)?;
        assert_eq!(environment["vars"]["API"], "production");
    }
    Ok(())
}

#[test]
fn builds_declared_plugins_into_the_macos_shell() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("macos-arm64")?;
    install_location_plugin(&project)?;

    build_command("macos", &project, &manifest)?
        .assert()
        .success();

    let plist = fs::read_to_string(project.join("build/macos/demo-app.app/Contents/Info.plist"))?;
    assert!(plist.contains("<key>NSLocationUsageDescription</key><string>Location test</string>"));
    Ok(())
}

#[test]
fn builds_plugins_installed_above_a_relative_project_directory() -> TestResult {
    let (temporary, project, manifest) = create_inputs("macos-arm64")?;
    install_location_plugin(&project)?;
    fs::rename(
        project.join("node_modules"),
        temporary.path().join("node_modules"),
    )?;

    build_command("macos", Path::new("."), &manifest)?
        .current_dir(&project)
        .assert()
        .success();

    let plist = fs::read_to_string(project.join("build/macos/demo-app.app/Contents/Info.plist"))?;
    assert!(plist.contains("<key>NSLocationUsageDescription</key><string>Location test</string>"));
    Ok(())
}

#[test]
fn builds_intel_macos_app() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("macos-x64")?;
    build_command("macos", &project, &manifest)?
        .assert()
        .success();

    assert!(project.join("build/macos/demo-app.app").is_dir());
    Ok(())
}

#[test]
fn builds_windows_app_with_its_runtime_files() -> TestResult {
    let (_temporary, project, manifest) = create_windows_inputs()?;
    build_command("windows", &project, &manifest)?
        .assert()
        .success()
        .stdout(contains("Built Windows bundle"));

    let bundle = project.join("build/windows/demo-app");
    assert!(bundle.join("demo-app.exe").is_file());
    assert!(!bundle.join("tokamak-runtime.dll").exists());
    assert!(bundle.join("app/worker-manifest.json").is_file());
    let config: serde_json::Value =
        serde_json::from_slice(&fs::read(bundle.join("tokamak.json"))?)?;
    assert_eq!(config["host"], "demo-app.tokamak.local");
    Ok(())
}

#[cfg(windows)]
#[test]
fn links_storage_into_the_windows_executable_while_the_app_declares_it() -> TestResult {
    let (_temporary, project, platform_pack) = create_windows_inputs()?;
    let run = || -> TestResult<Option<i32>> {
        // Outside a developer shell, the entrypoint finds the linker itself.
        build_command("windows", &project, &platform_pack)?
            .env_remove("VSCMD_ARG_TGT_ARCH")
            .assert()
            .success();
        let executable = project.join("build/windows/demo-app/demo-app.exe");
        Ok(ProcessCommand::new(executable).status()?.code())
    };

    assert_eq!(run()?, Some(1));
    declare_storage(&project)?;
    assert_eq!(run()?, Some(0));
    Ok(())
}

#[cfg(windows)]
#[test]
#[ignore = "needs a Windows platform pack at TOKAMAK_TEST_WINDOWS_PACK"]
fn links_storage_from_the_windows_platform_pack_while_the_app_declares_it() -> TestResult {
    let platform_pack = PathBuf::from(std::env::var("TOKAMAK_TEST_WINDOWS_PACK")?);
    let temporary = tempfile::tempdir()?;
    let project = temporary.path().join("project");
    fs::create_dir_all(&project)?;
    create_project(&project)?;
    let executable = project.join("build/windows/demo-app/demo-app.exe");
    let links_storage = || -> TestResult<(bool, bool)> {
        build_command("windows", &project, &platform_pack)?
            .assert()
            .success();
        let exports = ProcessCommand::new("dumpbin")
            .args(["/NOLOGO", "/EXPORTS"])
            .arg(&executable)
            .output()?;
        if !exports.status.success() {
            return Err(format!("dumpbin failed with {}", exports.status).into());
        }
        let contents = fs::read(&executable)?;
        println!("{}: {} bytes", executable.display(), contents.len());
        Ok((
            String::from_utf8_lossy(&exports.stdout).contains(tokamak::STORAGE_ENTRY_POINT),
            contains_sqlite(&contents),
        ))
    };

    assert_eq!(links_storage()?, (false, false));
    declare_storage(&project)?;
    assert_eq!(links_storage()?, (true, true));
    Ok(())
}

#[cfg(unix)]
#[test]
fn passes_platform_pack_variables_to_the_windows_entrypoint() -> TestResult {
    let (_temporary, project, manifest) = create_windows_inputs()?;
    build_command("windows", &project, &manifest)?
        .args(["--windows-test", "passed"])
        .assert()
        .success();

    assert_eq!(
        fs::read_to_string(project.join("build/windows/demo-app/set-value"))?,
        "passed"
    );
    Ok(())
}

#[test]
fn builds_with_a_configured_display_name() -> TestResult {
    let (_temporary, project, manifest) = create_windows_inputs()?;
    configure(
        &project,
        r#"{"name":"My App","windows":{"name":"My App Pro"}}"#,
    )?;

    let mut command = build_command("windows", &project, &manifest)?;
    command.assert().success();

    let bundle = project.join("build/windows/my-app-pro");
    assert!(bundle.join("my-app-pro.exe").is_file());
    let config: serde_json::Value =
        serde_json::from_slice(&fs::read(bundle.join("tokamak.json"))?)?;
    assert_eq!(config["name"], "My App Pro");
    assert_eq!(config["slug"], "my-app-pro");
    assert_eq!(config["host"], "my-app-pro.tokamak.local");
    Ok(())
}

#[test]
fn builds_a_configured_windows_icon() -> TestResult {
    let (_temporary, project, manifest) = create_windows_inputs()?;
    configure(&project, r#"{"windows":{"icon":"AppIcon.ico"}}"#)?;
    fs::create_dir_all(project.join("src"))?;
    fs::write(project.join("src/AppIcon.ico"), "ico")?;

    let mut command = build_command("windows", &project, &manifest)?;
    command.assert().success();

    assert_eq!(
        fs::read(project.join("build/windows/demo-app/AppIcon.ico"))?,
        b"ico"
    );
    Ok(())
}

#[test]
fn requires_a_version_for_app_builds() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("macos-arm64")?;
    let mut command = build_command("macos", &project, &manifest)?;
    command
        .env_remove("TOKAMAK_VERSION")
        .assert()
        .failure()
        .stderr(contains("tokamak version is required for `tok build`"));
    Ok(())
}

#[test]
fn requires_a_platform_pack_for_app_builds() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let project = temporary.path().join("project");
    fs::create_dir_all(&project)?;
    create_project(&project)?;

    let mut command = Command::cargo_bin("tok")?;
    command
        .args(["build", "macos", "--project"])
        .arg(&project)
        .arg("--skip-project-build")
        .env("TOKAMAK_VERSION", "1.0.0")
        // Installed platform packs under the real home directory must not satisfy the build.
        .env("HOME", temporary.path())
        .env("USERPROFILE", temporary.path())
        .env_remove("TOKAMAK_PLATFORM_PACK_PATH")
        .assert()
        .failure()
        .stderr(contains("no platform pack found"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn finds_platform_packs_installed_in_the_home_directory() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let project = temporary.path().join("project");
    let home = temporary.path().join("home");
    let pack = home.join(".local/share/tokamak/platform-packs/macos-arm64");
    fs::create_dir_all(&project)?;
    fs::create_dir_all(&pack)?;
    create_project(&project)?;
    create_platform_pack(&pack, "macos-arm64")?;

    let mut command = Command::cargo_bin("tok")?;
    command
        .args(["build", "macos", "--project"])
        .arg(&project)
        .arg("--skip-project-build")
        .env("TOKAMAK_VERSION", "1.0.0")
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        // An empty path is ignored.
        .env("TOKAMAK_PLATFORM_PACK_PATH", "")
        .assert()
        .success();

    assert!(project.join("build/macos/demo-app.app").is_dir());
    Ok(())
}

#[test]
fn reads_platform_pack_path_from_tokamak_environment() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let project = temporary.path().join("project");
    let platform_packs = temporary.path().join("platform-packs");
    fs::create_dir_all(&project)?;
    fs::create_dir_all(&platform_packs)?;
    create_project(&project)?;

    let mut command = Command::cargo_bin("tok")?;
    command
        .args(["build", "macos", "--project"])
        .arg(&project)
        .arg("--skip-project-build")
        .env("TOKAMAK_VERSION", "1.0.0")
        .env("TOKAMAK_PLATFORM_PACK_PATH", platform_packs)
        .assert()
        .failure()
        .stderr(contains(
            "TOKAMAK_PLATFORM_PACK_PATH does not contain a platform pack",
        ));
    Ok(())
}

#[cfg(unix)]
#[test]
fn searches_every_directory_in_the_platform_pack_path() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let project = temporary.path().join("project");
    let empty = temporary.path().join("empty");
    let platform_packs = temporary.path().join("platform-packs");
    let pack = platform_packs.join("macos-arm64");
    fs::create_dir_all(&project)?;
    fs::create_dir_all(&empty)?;
    fs::create_dir_all(&pack)?;
    create_project(&project)?;
    create_platform_pack(&pack, "macos-arm64")?;

    let mut command = Command::cargo_bin("tok")?;
    command
        .args(["build", "macos", "--project"])
        .arg(&project)
        .arg("--skip-project-build")
        .env("TOKAMAK_VERSION", "1.0.0")
        .env(
            "TOKAMAK_PLATFORM_PACK_PATH",
            std::env::join_paths([&empty, &platform_packs])?,
        )
        .assert()
        .success();

    assert!(project.join("build/macos/demo-app.app").is_dir());
    Ok(())
}

#[test]
fn builds_the_project_before_reading_what_it_generates() -> TestResult {
    let (_temporary, project, pack) = create_inputs("macos-arm64")?;
    configure(&project, r#"{ "name": "Built App" }"#)?;
    fs::remove_dir_all(project.join("build"))?;

    project_build_command("macos", &project, &pack)?
        .assert()
        .success();

    assert!(project.join("build/macos/built-app.app").is_dir());
    Ok(())
}

#[test]
fn runs_the_configured_build_command_in_the_project_directory() -> TestResult {
    let (temporary, project, pack) = create_windows_inputs()?;
    fs::remove_file(project.join("package.json"))?;
    fs::write(
        project.join("client.cjs"),
        "const fs = require('node:fs');\nfs.mkdirSync('dist/client', { recursive: true });\nfs.writeFileSync('dist/client/index.html', '<html>built</html>');\n",
    )?;

    Command::cargo_bin("tok")?
        .current_dir(temporary.path())
        .args(["build", "windows", "--project"])
        .arg(&project)
        .arg("--platform-pack")
        .arg(&pack)
        .args(["--version", "1.0.0"])
        .env("TOKAMAK_BUILD", "node build.cjs && node client.cjs")
        .assert()
        .success();

    assert_eq!(
        fs::read_to_string(project.join("build/windows/demo-app/app/assets/index.html"))?,
        "<html>built</html>"
    );
    Ok(())
}

#[test]
fn stops_when_the_configured_build_command_fails() -> TestResult {
    let (_temporary, project, pack) = create_windows_inputs()?;

    project_build_command("windows", &project, &pack)?
        .args(["--build", "exit 3"])
        .env("TOKAMAK_BUILD", "node build.cjs")
        .assert()
        .failure()
        .stderr(contains("project build failed"));
    assert!(!project.join("build/windows/demo-app").exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn builds_physical_ios_app() -> TestResult {
    let (temporary, project, manifest) = create_inputs("ios-arm64")?;
    install_location_plugin(&project)?;
    let profile = project.join("development.mobileprovision");
    fs::write(&profile, "profile")?;
    let mut command = build_command("ios", &project, &manifest)?;
    configure_fake_apple_tools(&mut command, temporary.path())?;
    command
        .env_remove("TOKAMAK_IOS_TEAM_ID")
        .args([
            "--ios-build-number",
            "5",
            "--ios-signing-identity",
            "Apple Development: Test",
            "--ios-provisioning-profile",
        ])
        .arg(&profile)
        .assert()
        .success()
        .stdout(contains("Built iOS bundle"));

    let bundle = project.join("build/ios/demo-app.app");
    assert!(bundle.join("demo-app").is_file());
    assert!(bundle.join("app/worker-manifest.json").is_file());
    assert!(!bundle.join("Frameworks/TokamakRuntime.framework").exists());
    let plist = fs::read_to_string(bundle.join("Info.plist"))?;
    assert!(plist.contains("LSRequiresIPhoneOS"));
    assert!(plist.contains("UIDeviceFamily"));
    assert!(plist.contains("UILaunchScreen"));
    assert!(plist.contains("NSAllowsLocalNetworking"));
    assert!(plist.contains("<key>CFBundleVersion</key><string>5</string>"));
    assert!(plist.contains("<key>CFBundleShortVersionString</key><string>1.0.0</string>"));
    assert!(plist.contains("NSLocationWhenInUseUsageDescription"));
    assert!(bundle.join("embedded.mobileprovision").is_file());
    Ok(())
}

#[cfg(unix)]
#[test]
fn builds_macos_app_with_environment_build_number() -> TestResult {
    let (temporary, project, manifest) = create_inputs("macos-arm64")?;
    let mut command = build_command("macos", &project, &manifest)?;
    command.env("TOKAMAK_MACOS_BUILD_NUMBER", "7");
    configure_fake_apple_tools(&mut command, temporary.path())?;
    command.assert().success();

    let plist = fs::read_to_string(project.join("build/macos/demo-app.app/Contents/Info.plist"))?;
    assert!(plist.contains("<key>CFBundleVersion</key><string>7</string>"));
    assert!(plist.contains("<key>CFBundleShortVersionString</key><string>1.0.0</string>"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn rejects_invalid_apple_build_number() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("ios-simulator-arm64")?;
    let mut command = build_command("ios-simulator", &project, &manifest)?;
    command.env("TOKAMAK_IOS_BUILD_NUMBER", "0.1.2-5");
    command.assert().failure().stderr(contains(
        "Apple build number must contain one to three period-separated integers: 0.1.2-5",
    ));
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn build_explains_conflicting_automatic_and_manual_signing() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("ios-arm64")?;
    for cli_variable in [false, true] {
        let mut command = build_command("ios", &project, &manifest)?;
        command
            .env_remove("TOKAMAK_IOS_TEAM_ID")
            .env("TOKAMAK_IOS_SIGNING_IDENTITY", "IDENTITY_SHA1")
            .env(
                "TOKAMAK_IOS_PROVISIONING_PROFILE",
                project.join("manual.mobileprovision"),
            );
        if cli_variable {
            command.args(["--ios-team-id", "TEAM"]);
        } else {
            command.env("TOKAMAK_IOS_TEAM_ID", "TEAM");
        }
        command.assert().failure().stderr(
            contains("automatic and manual iOS signing cannot be combined.")
                .and(contains(
                    "Automatic: set TOKAMAK_IOS_TEAM_ID or use --ios-team-id TEAM_ID",
                ))
                .and(contains(
                    "Manual:    set BOTH TOKAMAK_IOS_SIGNING_IDENTITY and",
                )),
        );
    }
    Ok(())
}

/// `tok dev` for an iOS device with both automatic and manual signing, which
/// the platform-pack entrypoint rejects.
#[cfg(all(unix, target_os = "macos"))]
fn conflicting_signing_dev_command(
    temporary: &Path,
    project: &Path,
    platform_pack: &Path,
    project_arg: &Path,
) -> TestResult<Command> {
    let profile = project.join("manual.mobileprovision");
    fs::write(&profile, "profile")?;

    let mut command = Command::cargo_bin("tok")?;
    configure_fake_apple_tools(&mut command, temporary)?;
    command
        .args(["dev", "DEVICE", "--project"])
        .arg(project_arg)
        .args(["--platform-pack"])
        .arg(platform_pack)
        .args(["--host-address", "127.0.0.1"])
        .args([
            "--ios-signing-identity",
            "IDENTITY_SHA1",
            "--ios-provisioning-profile",
        ])
        .arg(&profile)
        .env("TOKAMAK_IOS_TEAM_ID", "TEAM")
        .args(["--", "node", "-e", DEV_SERVER]);
    Ok(command)
}

/// A development server that reports the stand-in build's configuration, and
/// itself and its Worker as the tokamak Vite plugin does.
#[cfg(all(unix, target_os = "macos"))]
const DEV_SERVER: &str = r#"
require("./build.cjs");
const server = require("node:http").createServer((_, response) => response.end());
server.listen(0, "127.0.0.1", () => {
  const url = `http://127.0.0.1:${server.address().port}/`;
  const report = require("node:path").join(process.env.TOKAMAK_VITE_OUTPUT, "server.json");
  require("node:fs").writeFileSync(report, JSON.stringify({ url, workerName: "demo-app" }));
});
"#;

#[cfg(all(unix, target_os = "macos"))]
#[test]
fn dev_runs_the_entrypoint_for_a_relative_project_directory() -> TestResult {
    let (temporary, project, platform_pack) = create_inputs("ios-arm64")?;
    conflicting_signing_dev_command(temporary.path(), &project, &platform_pack, Path::new("."))?
        .current_dir(&project)
        .assert()
        .failure()
        .stderr(contains(
            "automatic and manual iOS signing cannot be combined.",
        ));
    Ok(())
}

#[cfg(all(unix, target_os = "macos"))]
#[test]
fn dev_selects_an_ios_device_when_android_devices_cannot_be_queried() -> TestResult {
    let (temporary, project, platform_pack) = create_inputs("ios-arm64")?;
    conflicting_signing_dev_command(temporary.path(), &project, &platform_pack, &project)?
        .env("ANDROID_ADB_SERVER_PORT", "0")
        .assert()
        .failure()
        .stderr(contains(
            "automatic and manual iOS signing cannot be combined.",
        ));
    Ok(())
}

#[cfg(all(unix, target_os = "macos"))]
#[test]
fn dev_explains_conflicting_automatic_and_manual_signing() -> TestResult {
    let (temporary, project, platform_pack) = create_inputs("ios-arm64")?;
    conflicting_signing_dev_command(temporary.path(), &project, &platform_pack, &project)?
        .assert()
        .failure()
        .stderr(
            contains("automatic and manual iOS signing cannot be combined.")
                .and(contains(
                    "Automatic: set TOKAMAK_IOS_TEAM_ID or use --ios-team-id TEAM_ID",
                ))
                .and(contains(
                    "Manual:    set BOTH TOKAMAK_IOS_SIGNING_IDENTITY and",
                )),
        );
    Ok(())
}

#[test]
fn builds_ios_simulator_app() -> TestResult {
    for target in ["ios-simulator-arm64", "ios-simulator-x64"] {
        let (_temporary, project, manifest) = create_inputs(target)?;
        install_location_plugin(&project)?;
        build_command("ios-simulator", &project, &manifest)?
            .assert()
            .success()
            .stdout(contains("Built iOS Simulator bundle"));

        let bundle = project.join("build/ios-simulator/demo-app.app");
        assert!(bundle.join("demo-app").is_file());
        assert!(bundle.join("app/worker-manifest.json").is_file());
        assert!(!bundle.join("Frameworks/TokamakRuntime.framework").exists());
        assert!(!bundle.join("embedded.mobileprovision").exists());
        let plist = fs::read_to_string(bundle.join("Info.plist"))?;
        assert!(plist.contains("iPhoneSimulator"));
        assert!(plist.contains("NSLocationWhenInUseUsageDescription"));
        assert_eq!(
            simulator_entitlements(&bundle.join("demo-app"))?,
            serde_json::json!({
                "application-identifier": "com.tokamak.demo-app",
                "keychain-access-groups": ["com.tokamak.demo-app"],
            })
        );
    }
    Ok(())
}

#[test]
fn embeds_declared_entitlements_in_simulator_builds() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("ios-simulator-arm64")?;
    fs::write(
        project.join("App.entitlements"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>aps-environment</key><string>development</string></dict></plist>"#,
    )?;

    build_command("ios-simulator", &project, &manifest)?
        .current_dir(&project)
        .args(["--ios-entitlements", "App.entitlements"])
        .assert()
        .success();

    assert_eq!(
        simulator_entitlements(&project.join("build/ios-simulator/demo-app.app/demo-app"))?,
        serde_json::json!({
            "application-identifier": "com.tokamak.demo-app",
            "keychain-access-groups": ["com.tokamak.demo-app"],
            "aps-environment": "development",
        })
    );
    Ok(())
}

/// The entitlements the simulator reads from the executable's `__TEXT,__entitlements` section.
fn simulator_entitlements(executable: &Path) -> TestResult<serde_json::Value> {
    let section = executable.with_extension("entitlements");
    let status = ProcessCommand::new("xcrun")
        .args(["segedit"])
        .arg(executable)
        .args(["-extract", "__TEXT", "__entitlements"])
        .arg(&section)
        .status()?;
    if !status.success() {
        return Err(format!("entitlements extraction failed with {status}").into());
    }
    let output = ProcessCommand::new("plutil")
        .args(["-convert", "json", "-o", "-"])
        .arg(&section)
        .output()?;
    Ok(serde_json::from_slice(&output.stdout)?)
}

#[test]
fn builds_secure_storage_and_local_authentication_into_apple_shells() -> TestResult {
    for (platform, target, bundle, plist) in [
        (
            "ios-simulator",
            "ios-simulator-arm64",
            "build/ios-simulator/demo-app.app",
            "Info.plist",
        ),
        (
            "macos",
            "macos-arm64",
            "build/macos/demo-app.app",
            "Contents/Info.plist",
        ),
    ] {
        let (_temporary, project, manifest) = create_inputs(target)?;
        install_key_flow_plugins(&project)?;
        build_command(platform, &project, &manifest)?
            .assert()
            .success();

        let plist = fs::read_to_string(project.join(bundle).join(plist))?;
        assert_eq!(
            plist.contains("<key>NSFaceIDUsageDescription</key>"),
            platform == "ios-simulator"
        );
    }
    Ok(())
}

#[test]
fn writes_configured_asset_routing_modes() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("macos-arm64")?;
    write_worker_config(
        &project,
        r#"{
  "assets": {
    "directory": "../client",
    "html_handling": "drop-trailing-slash",
    "not_found_handling": "single-page-application"
  }
}"#,
    )?;
    build_command("macos", &project, &manifest)?
        .assert()
        .success();

    let app = PackageLayout::new(project.join("build/macos/demo-app.app/Contents/Resources/app"));
    let manifest = read_asset_manifest(&app)?;
    assert_eq!(manifest.html_handling, HtmlHandling::DropTrailingSlash);
    assert_eq!(
        manifest.not_found_handling,
        NotFoundHandling::SinglePageApplication
    );
    Ok(())
}

#[test]
fn packages_modules_the_configured_rules_match_under_bundle() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("macos-arm64")?;
    fs::create_dir_all(project.join("dist/app/config"))?;
    fs::write(
        project.join("dist/app/config/runtime.json"),
        br#"{"feature":true}"#,
    )?;
    fs::write(project.join("dist/app/not-included.md"), "private")?;
    write_worker_config(
        &project,
        r#"{ "rules": [{ "type": "ESModule", "globs": ["**/*.js"] }, { "type": "Data", "globs": ["**/*.json"] }] }"#,
    )?;

    build_command("macos", &project, &manifest)?
        .assert()
        .success();

    let app = project.join("build/macos/demo-app.app/Contents/Resources/app");
    assert_eq!(
        fs::read(app.join("bundle/config/runtime.json"))?,
        br#"{"feature":true}"#
    );
    assert!(!app.join("bundle/not-included.md").exists());
    Ok(())
}

#[test]
fn packages_webassembly_assets_as_static_files() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("macos-arm64")?;
    fs::write(project.join("dist/client/module.wasm"), b"wasm")?;

    build_command("macos", &project, &manifest)?
        .assert()
        .success();
    let app = project.join("build/macos/demo-app.app/Contents/Resources/app");
    assert!(app.join("assets/module.wasm").is_file());
    Ok(())
}

#[test]
fn requires_a_name_while_the_worker_name_is_not_an_app_name() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("macos-arm64")?;
    write_worker_config(&project, r#"{"name":"../demo"}"#)?;

    build_command("macos", &project, &manifest)?
        .assert()
        .failure()
        .stderr(contains(
            r#"macOS has no name, and the Worker name "../demo" is not a valid app name"#,
        ));
    assert!(!project.join("build/.tokamak/worker").exists());
    Ok(())
}

#[test]
fn names_the_app_from_the_name_setting_whatever_the_worker_name() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("macos-arm64")?;
    write_worker_config(&project, r#"{"name":"my_worker"}"#)?;

    build_command("macos", &project, &manifest)?
        .args(["--name", "My Worker"])
        .assert()
        .success();
    assert!(project.join("build/macos/my-worker.app").is_dir());
    Ok(())
}

#[test]
fn requires_a_wrangler_name() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("macos-arm64")?;
    write_worker_config(&project, r#"{"name":null}"#)?;

    build_command("macos", &project, &manifest)?
        .assert()
        .failure()
        .stderr(contains("missing required field name"));
    Ok(())
}

#[test]
fn rejects_platform_pack_for_another_platform() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("ios-arm64")?;
    build_command("macos", &project, &manifest)?
        .assert()
        .failure()
        .stderr(contains("platform pack ios-arm64 cannot build macOS"));
    Ok(())
}

#[test]
fn rejects_platform_pack_for_another_cli_version() -> TestResult {
    let (_temporary, project, platform_pack) = create_inputs("macos-arm64")?;
    let manifest_path = platform_pack.join(MANIFEST_FILE);
    write_manifest(&manifest_path, &test_manifest(Target::MacosArm64, "9.9.9")?)?;

    build_command("macos", &project, &platform_pack)?
        .assert()
        .failure()
        .stderr(contains("platform pack was built for tokamak 9.9.9"));
    Ok(())
}
