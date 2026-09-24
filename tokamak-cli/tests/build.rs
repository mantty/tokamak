use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use assert_cmd::Command;
#[cfg(target_os = "macos")]
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;
use tokamak::compile_module;
use tokamak::{PackageLayout, decompress_worker_module, read_worker_manifest};
use tokamak_cli::{
    ESBUILD_EXECUTABLE, MANIFEST_FILE, PlatformPackManifest, Target, write_manifest,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn create_project(root: &Path) -> TestResult {
    fs::write(
        root.join("package.json"),
        r#"{"name":"demo-app","scripts":{"build":"echo already-built"}}"#,
    )?;
    fs::create_dir_all(root.join("dist/server"))?;
    fs::create_dir_all(root.join("dist/client/styles"))?;
    fs::write(root.join("dist/server/entry.mjs"), "export default {};")?;
    fs::write(root.join("dist/client/index.html"), "<html></html>")?;
    fs::write(root.join("dist/client/styles/app.css"), "body{}")?;
    fs::write(
        root.join("wrangler.jsonc"),
        r#"{
  "name": "demo-app",
  "main": "dist/server/entry.mjs",
  "assets": { "directory": "dist/client", "binding": "ASSETS" }
}"#,
    )?;
    Ok(())
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
    fs::write(
        plugin.join("tokamak-plugin.json"),
        r#"{
  "schemaVersion": 1,
  "id": "location",
  "kind": "frontend",
  "platforms": {
    "macos": {
      "class": "TokamakLocationPlugin",
      "sources": ["apple/LocationPlugin.swift"],
      "frameworks": ["CoreLocation"],
      "plist": {"NSLocationUsageDescription": "Location test"}
    },
    "ios": {
      "class": "TokamakLocationPlugin",
      "sources": ["apple/LocationPlugin.swift"],
      "frameworks": ["CoreLocation"],
      "plist": {"NSLocationWhenInUseUsageDescription": "Location test"}
    },
    "ios-simulator": {
      "class": "TokamakLocationPlugin",
      "sources": ["apple/LocationPlugin.swift"],
      "frameworks": ["CoreLocation"],
      "plist": {"NSLocationWhenInUseUsageDescription": "Location test"}
    }
  }
}"#,
    )?;
    Ok(())
}

fn create_unbuilt_project(root: &Path) -> TestResult {
    fs::write(
        root.join("package.json"),
        r#"{"name":"built-app","scripts":{"build":"node build.cjs"}}"#,
    )?;
    fs::write(
        root.join("build.cjs"),
        r#"const fs = require("node:fs");
fs.mkdirSync("dist/server", { recursive: true });
fs.mkdirSync("dist/client", { recursive: true });
fs.writeFileSync("dist/server/entry.mjs", "export default {};");
fs.writeFileSync("dist/client/index.html", "<html></html>");
fs.writeFileSync("dist/server/wrangler.json", JSON.stringify({
  name: "built-app",
  main: "entry.mjs",
  userConfigPath: "../../wrangler.jsonc",
  assets: { directory: "../client", binding: "ASSETS" }
}));
"#,
    )?;
    fs::write(
        root.join("wrangler.jsonc"),
        r#"{
  "name": "built-app",
  "vars": { "API": "default" },
  "env": { "production": { "vars": { "API": "production" } } }
}"#,
    )?;
    Ok(())
}

fn write_test_esbuild(root: &Path) -> TestResult {
    let path = root.join(ESBUILD_EXECUTABLE);
    fs::create_dir_all(path.parent().ok_or("esbuild path has no parent")?)?;
    fs::write(
        &path,
        "#!/usr/bin/env node\nconst fs = require('node:fs');\nconst path = require('node:path');\nconst args = process.argv.slice(2);\nconst output = args.find((arg) => arg.startsWith('--outdir=')).slice('--outdir='.length);\nconst metafile = args.find((arg) => arg.startsWith('--metafile='));\nconst input = args.at(-1);\nif (process.env.TOKAMAK_TEST_ESBUILD_LOG) fs.appendFileSync(process.env.TOKAMAK_TEST_ESBUILD_LOG, 'build\\n');\nfs.mkdirSync(output, { recursive: true });\nfs.copyFileSync(input, path.join(output, 'entry.js'));\nif (metafile) fs.writeFileSync(metafile.slice('--metafile='.length), JSON.stringify({ inputs: { [input]: {} } }));\n",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&path)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)?;
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
    write_test_esbuild(root)?;
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
    fs::create_dir_all(root.join("bin"))?;
    fs::write(root.join(target.runtime_artifact_path()), "runtime")?;
    fs::create_dir_all(root.join("native-shell"))?;
    fs::File::create(root.join("native-shell/TokamakActivity.kt"))?;
    let entrypoint = root.join(target.build_entrypoint_path());
    fs::create_dir_all(entrypoint.parent().ok_or("entrypoint path has no parent")?)?;
    fs::write(
        entrypoint,
        include_str!("../../platforms/android/build/entrypoint"),
    )?;
    write_test_esbuild(root)?;
    write_test_manifest(root, target)?;
    Ok(root.to_path_buf())
}

fn write_test_manifest(root: &Path, target: Target) -> TestResult {
    write_manifest(
        root.join(MANIFEST_FILE),
        &PlatformPackManifest {
            tokamak_version: env!("CARGO_PKG_VERSION").to_owned(),
            target,
            artifacts: target.artifacts(),
            required_tools: target
                .required_tools()
                .iter()
                .map(|tool| (*tool).to_owned())
                .collect(),
        },
    )?;
    Ok(())
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

protocol TokamakPlugin: AnyObject {
  var id: String { get }
  func call(method: String, arguments: Any, reply: @escaping TokamakPluginReply)
  func subscribe(
    method: String,
    arguments: Any,
    reply: @escaping TokamakPluginReply
  ) -> (() -> Void)
}

@main struct App { static func main() {} }
"#,
    )?;
    Ok(())
}

fn create_test_framework(root: &Path, target: &str) -> TestResult {
    let source = root.join("runtime.c");
    let object = root.join("runtime.o");
    fs::write(&source, "void tokamak_test(void) {}")?;

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
    fs::create_dir_all(pack.join("bin"))?;
    create_project(&project)?;
    let target = Target::WindowsX64;
    fs::create_dir_all(
        pack.join(target.runtime_artifact_path())
            .parent()
            .ok_or("runtime path has no parent")?,
    )?;
    fs::write(pack.join(target.runtime_artifact_path()), "shell")?;
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
cp "$input/runtime/tokamak-shell-windows.exe" "$output/$app_slug.exe"
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
    write_test_esbuild(&pack)?;
    write_test_manifest(&pack, target)?;
    Ok((temporary, project, pack))
}

fn build_command(platform: &str, project: &Path, platform_pack: &Path) -> TestResult<Command> {
    let mut command = Command::cargo_bin("tok")?;
    command
        .args(["build", platform, "--project"])
        .arg(project)
        .arg("--platform-pack")
        .arg(platform_pack)
        .arg("--skip-project-build")
        .env("TOKAMAK_VERSION", "1.0.0")
        .env_remove("TOKAMAK_IOS_BUILD_NUMBER")
        .env_remove("TOKAMAK_MACOS_BUILD_NUMBER");
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
mkdir -p "$project/app/build/outputs/apk/debug"
if [ -n "${TOKAMAK_ANDROID_TEST:-}" ]; then
  printf '%s' "$TOKAMAK_ANDROID_TEST" > "$project/platform-pack-set-value"
fi
cp "$project/app/src/main/AndroidManifest.xml" "$project/app/build/outputs/apk/debug/AndroidManifest.xml"
touch "$project/app/build/outputs/apk/debug/app-debug.apk"
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

    fs::write(path, contents)?;
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)?;
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
    assert_eq!(manifest.entry, "entry.js");
    assert_eq!(
        decompress_worker_module(&fs::read(app.join("worker-modules/entry.js.qjs"))?)?,
        compile_module(
            "entry.js",
            &fs::read(project.join("dist/server/entry.mjs"))?
        )?
    );
    assert!(app.join("assets/index.html").is_file());
    let plist = fs::read_to_string(bundle.join("Contents/Info.plist"))?;
    assert!(plist.contains("NSAllowsLocalNetworking"));
    assert!(!bundle.join("Contents/Resources/Assets.car").exists());
    assert!(!plist.contains("CFBundleIconName"));
    assert!(!plist.contains("CFBundleIconFile"));
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(app.join("asset-manifest.json"))?)?;
    assert_eq!(manifest["files"]["styles/app.css"], "text/css");
    assert!(!app.join("config.capnp").exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn builds_configured_identifier_and_version() -> TestResult {
    let (temporary, project, manifest) = create_inputs("macos-arm64")?;
    fs::write(
        project.join("tokamak.jsonc"),
        r#"{
  "identifier": "com.example.app",
  "macos": { "identifier": "com.example.desktop" },
  "version": "2.3.4"
}"#,
    )?;

    let mut command = build_command("macos", &project, &manifest)?;
    command.env_remove("TOKAMAK_VERSION");
    command.arg("--config").arg(project.join("tokamak.jsonc"));
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
fn warns_about_a_nested_include() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("macos-arm64")?;
    fs::write(project.join("base.jsonc"), r#"{ "version": "9.9.9" }"#)?;
    fs::write(
        project.join("parent.jsonc"),
        r#"{ "include": "base.jsonc", "identifier": "com.example.parent" }"#,
    )?;
    fs::write(
        project.join("tokamak.jsonc"),
        r#"{ "include": "parent.jsonc", "name": "Demo App" }"#,
    )?;

    let mut command = build_command("macos", &project, &manifest)?;
    command.arg("--config").arg(project.join("tokamak.jsonc"));
    command.assert().success().stderr(contains(format!(
        "warning: {}: nested include is ignored",
        project.join("parent.jsonc").display()
    )));
    Ok(())
}

#[cfg(unix)]
#[test]
fn preserves_configured_display_name_in_apple_bundle() -> TestResult {
    let (temporary, project, manifest) = create_inputs("ios-simulator-arm64")?;
    fs::write(project.join("tokamak.jsonc"), r#"{"name":"Vigilus"}"#)?;

    let mut command = build_command("ios-simulator", &project, &manifest)?;
    command.arg("--config").arg(project.join("tokamak.jsonc"));
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
    fs::write(project.join("tokamak.jsonc"), r#"{"name":"Vigilus"}"#)?;

    let mut command = build_command("macos", &project, &manifest)?;
    command.arg("--config").arg(project.join("tokamak.jsonc"));
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
    let temporary = tempfile::tempdir()?;
    let project = temporary.path().join("project");
    let pack = temporary.path().join("pack");
    fs::create_dir_all(&project)?;
    fs::create_dir_all(&pack)?;
    create_project(&project)?;
    let platform_pack = create_android_platform_pack(&pack)?;
    fs::write(
        project.join("tokamak.jsonc"),
        r#"{"name":"Vigilus & <Co> \"Pro\" 'X'"}"#,
    )?;

    let mut command = build_command("android", &project, &platform_pack)?;
    command.arg("--config").arg(project.join("tokamak.jsonc"));
    configure_fake_gradle(&mut command, temporary.path())?;
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
fn passes_platform_pack_variables_to_the_android_entrypoint() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let project = temporary.path().join("project");
    let pack = temporary.path().join("pack");
    fs::create_dir_all(&project)?;
    fs::create_dir_all(&pack)?;
    create_project(&project)?;
    let platform_pack = create_android_platform_pack(&pack)?;

    let mut command = build_command("android", &project, &platform_pack)?;
    command.args(["--set", "android-test=passed"]);
    configure_fake_gradle(&mut command, temporary.path())?;
    command.assert().success();

    assert_eq!(
        fs::read_to_string(project.join("build/android/.tokamak/platform-pack-set-value"))?,
        "passed"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn environment_overrides_configured_identifier_and_version() -> TestResult {
    let (temporary, project, manifest) = create_inputs("macos-arm64")?;
    fs::write(
        project.join("tokamak.jsonc"),
        r#"{
  "identifier": "com.example.config",
  "macos": { "identifier": "com.example.config-macos" },
  "version": "2.3.4"
}"#,
    )?;

    let mut command = build_command("macos", &project, &manifest)?;
    command
        .arg("--config")
        .arg(project.join("tokamak.jsonc"))
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
        let icon = project.join("assets/AppIcon.icon");
        fs::create_dir_all(&icon)?;
        fs::write(icon.join("icon.json"), "{}")?;
        fs::write(
            project.join("tokamak.jsonc"),
            r#"{ "icon": "assets/AppIcon.icon" }"#,
        )?;

        let mut command = build_command(platform, &project, &manifest)?;
        command.arg("--config").arg(project.join("tokamak.jsonc"));
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
fn compiles_a_self_contained_worker() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("macos-arm64")?;
    fs::write(
        project.join("dist/server/entry.mjs"),
        "export default { fetch() { return new Response('ok'); } };",
    )?;

    build_command("macos", &project, &manifest)?
        .assert()
        .success();

    let bundle = project.join("build/macos/demo-app.app/Contents/Resources/app");
    let manifest = read_worker_manifest(&PackageLayout::new(&bundle))?;
    assert_eq!(manifest.entry, "entry.js");
    assert_eq!(
        decompress_worker_module(&fs::read(bundle.join("worker-modules/entry.js.qjs"))?)?,
        compile_module(
            "entry.js",
            &fs::read(project.join("dist/server/entry.mjs"))?
        )?
    );
    Ok(())
}

#[test]
fn packages_worker_vars_as_a_normalized_manifest() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("macos-arm64")?;
    fs::write(
        project.join("wrangler.jsonc"),
        r#"{
  "name": "demo-app",
  "main": "dist/server/entry.mjs",
  "vars": { "TEXT": "value", "JSON": { "enabled": true } }
}"#,
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
fn reuses_unchanged_build_layers_across_wrangler_environments() -> TestResult {
    let (temporary, project, pack) = create_windows_inputs()?;
    let esbuild_log = temporary.path().join("esbuild.log");
    fs::write(
        project.join("wrangler.jsonc"),
        r#"{
  "name": "demo-app",
  "main": "dist/server/entry.mjs",
  "assets": { "directory": "dist/client" },
  "env": {
    "test": { "vars": { "API": "test", "OPTIONS": { "enabled": true } } },
    "production": { "vars": { "API": "production" } }
  }
}"#,
    )?;
    let build = |environment: &str| -> TestResult {
        build_command("windows", &project, &pack)?
            .args(["--build-dir", ".cache/tokamak", "--env", environment])
            .env("TOKAMAK_TEST_ESBUILD_LOG", &esbuild_log)
            .assert()
            .success();
        Ok(())
    };
    let app = project.join(".cache/tokamak/windows/demo-app/app");
    build("test")?;
    let test: serde_json::Value =
        serde_json::from_slice(&fs::read(app.join("worker-environment.json"))?)?;
    assert_eq!(test["vars"]["API"], "test");
    assert_eq!(
        test["vars"]["OPTIONS"],
        serde_json::json!({ "enabled": true })
    );

    build("production")?;
    let production: serde_json::Value =
        serde_json::from_slice(&fs::read(app.join("worker-environment.json"))?)?;
    assert_eq!(
        production["vars"],
        serde_json::json!({ "API": "production" })
    );
    assert_eq!(fs::read_to_string(&esbuild_log)?.lines().count(), 1);

    let entry = project.join("dist/server/entry.mjs");
    fs::write(&entry, "export default { value: 2 };")?;
    build("production")?;
    assert_eq!(fs::read_to_string(&esbuild_log)?.lines().count(), 2);
    assert_eq!(
        decompress_worker_module(&fs::read(app.join("worker-modules/entry.js.qjs"))?)?,
        compile_module("entry.js", &fs::read(&entry)?)?
    );
    Ok(())
}

#[test]
fn updates_changed_assets_without_recompiling_the_worker() -> TestResult {
    let (temporary, project, pack) = create_windows_inputs()?;
    let esbuild_log = temporary.path().join("esbuild.log");
    let build = || -> TestResult {
        build_command("windows", &project, &pack)?
            .env("TOKAMAK_TEST_ESBUILD_LOG", &esbuild_log)
            .assert()
            .success();
        Ok(())
    };

    build()?;
    fs::write(
        project.join("dist/client/index.html"),
        "<html>updated</html>",
    )?;
    build()?;

    assert_eq!(
        fs::read_to_string(project.join("build/windows/demo-app/app/assets/index.html"))?,
        "<html>updated</html>"
    );
    assert_eq!(fs::read_to_string(esbuild_log)?.lines().count(), 1);
    Ok(())
}

#[test]
fn ignores_unrelated_files_when_reusing_worker_modules() -> TestResult {
    let (temporary, project, pack) = create_windows_inputs()?;
    let esbuild_log = temporary.path().join("esbuild.log");
    let server = project.join("dist/server");
    fs::write(server.join("settings.json"), r#"{"feature":true}"#)?;
    fs::write(
        project.join("wrangler.jsonc"),
        r#"{
  "name": "demo-app",
  "main": "dist/server/entry.mjs",
  "base_dir": "dist/server",
  "find_additional_modules": true,
  "rules": [{ "type": "Data", "globs": ["**/*.json"] }]
}"#,
    )?;
    let build = || -> TestResult {
        build_command("windows", &project, &pack)?
            .env("TOKAMAK_TEST_ESBUILD_LOG", &esbuild_log)
            .assert()
            .success();
        Ok(())
    };

    build()?;
    fs::write(server.join("unrelated.txt"), "changed")?;
    build()?;
    assert_eq!(fs::read_to_string(&esbuild_log)?.lines().count(), 1);

    fs::write(server.join("settings.json"), r#"{"feature":false}"#)?;
    build()?;
    assert_eq!(fs::read_to_string(&esbuild_log)?.lines().count(), 2);
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
        fs::write(
            project.join("wrangler.jsonc"),
            r#"{
  "name": "demo-app",
  "main": "dist/server/entry.mjs",
  "env": {
    "test": { "vars": { "API": "test" } },
    "production": { "vars": { "API": "production" } }
  }
}"#,
        )?;
        let mut test = build_command(platform, &project, &pack)?;
        let log = configure_fake_apple_tools(&mut test, temporary.path())?;
        test.args(["--env", "test"]).assert().success();

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
        production.args(["--env", "production"]).assert().success();

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
fn apple_build_number_change_reuses_the_native_bundle() -> TestResult {
    for (platform, target) in [
        ("macos", "macos-arm64"),
        ("ios", "ios-arm64"),
        ("ios-simulator", "ios-simulator-arm64"),
    ] {
        let (temporary, project, pack) = create_inputs(target)?;
        fs::write(
            project.join("wrangler.jsonc"),
            r#"{"name":"demo-app","main":"dist/server/entry.mjs","env":{"test":{"vars":{"API":"test"}},"production":{"vars":{"API":"production"}}}}"#,
        )?;
        let icon = project.join("assets/AppIcon.icon");
        fs::create_dir_all(&icon)?;
        fs::write(icon.join("icon.json"), "{}")?;
        let config = project.join("tokamak.jsonc");
        fs::write(&config, r#"{"icon":"assets/AppIcon.icon"}"#)?;
        let build_number = if platform == "macos" {
            "TOKAMAK_MACOS_BUILD_NUMBER"
        } else {
            "TOKAMAK_IOS_BUILD_NUMBER"
        };

        let build = |number, environment| -> TestResult {
            let mut command = build_command(platform, &project, &pack)?;
            configure_fake_apple_tools(&mut command, temporary.path())?;
            command
                .arg("--config")
                .arg(&config)
                .env(build_number, number)
                .args(["--env", environment])
                .assert()
                .success();
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

#[cfg(unix)]
#[test]
fn passes_platform_pack_variables_to_the_windows_entrypoint() -> TestResult {
    let (_temporary, project, manifest) = create_windows_inputs()?;
    build_command("windows", &project, &manifest)?
        .args(["--set", "windows-test=passed"])
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
    fs::write(
        project.join("tokamak.jsonc"),
        r#"{"name":"My App","windows":{"name":"My App Pro"}}"#,
    )?;

    let mut command = build_command("windows", &project, &manifest)?;
    command.arg("--config").arg(project.join("tokamak.jsonc"));
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
    fs::write(project.join("AppIcon.ico"), "ico")?;
    fs::write(
        project.join("tokamak.jsonc"),
        r#"{"windows":{"icon":"AppIcon.ico"}}"#,
    )?;

    let mut command = build_command("windows", &project, &manifest)?;
    command.arg("--config").arg(project.join("tokamak.jsonc"));
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
fn builds_project_before_loading_generated_config() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let project = temporary.path().join("project");
    let pack = temporary.path().join("pack");
    fs::create_dir_all(&project)?;
    fs::create_dir_all(&pack)?;
    create_unbuilt_project(&project)?;
    let platform_pack = create_platform_pack(&pack, "macos-arm64")?;
    let config = project.join("dist/server/wrangler.json");

    let mut command = Command::cargo_bin("tok")?;
    command
        .args(["build", "macos", "--project"])
        .arg(&project)
        .arg("--platform-pack")
        .arg(platform_pack)
        .arg("--wrangler")
        .arg(config)
        .env("TOKAMAK_VERSION", "1.0.0");
    command.assert().success();

    assert!(project.join("build/macos/built-app.app").is_dir());
    Ok(())
}

#[test]
fn runs_the_configured_build_command_in_the_project_directory() -> TestResult {
    let (temporary, project, pack) = create_windows_inputs()?;
    fs::remove_dir_all(project.join("dist"))?;
    fs::remove_file(project.join("package.json"))?;
    fs::write(
        project.join("server.cjs"),
        "const fs = require('node:fs');\nfs.mkdirSync('dist/server', { recursive: true });\nfs.writeFileSync('dist/server/entry.mjs', 'export default {};');\n",
    )?;
    fs::write(
        project.join("client.cjs"),
        "const fs = require('node:fs');\nfs.mkdirSync('dist/client', { recursive: true });\nfs.writeFileSync('dist/client/index.html', '<html>built</html>');\n",
    )?;
    let config = temporary.path().join("tokamak.jsonc");
    fs::write(
        &config,
        r#"{ "version": "1.0.0", "build": "node server.cjs && node client.cjs" }"#,
    )?;

    Command::cargo_bin("tok")?
        .current_dir(temporary.path())
        .args(["build", "windows", "--project"])
        .arg(&project)
        .arg("--platform-pack")
        .arg(&pack)
        .arg("--config")
        .arg(&config)
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
    let (temporary, project, pack) = create_windows_inputs()?;
    let config = temporary.path().join("tokamak.jsonc");
    fs::write(&config, r#"{ "version": "1.0.0", "build": "exit 3" }"#)?;

    Command::cargo_bin("tok")?
        .args(["build", "windows", "--project"])
        .arg(&project)
        .arg("--platform-pack")
        .arg(&pack)
        .arg("--config")
        .arg(&config)
        .assert()
        .failure()
        .stderr(contains("project build failed"));
    assert!(!project.join("build/windows/demo-app").exists());
    Ok(())
}

#[test]
fn builds_named_environment_from_generated_wrangler_config() -> TestResult {
    let (temporary, project, pack) = create_windows_inputs()?;
    fs::remove_dir_all(project.join("dist"))?;
    create_unbuilt_project(&project)?;
    let esbuild_log = temporary.path().join("esbuild.log");
    let build = || -> TestResult {
        Command::cargo_bin("tok")?
            .args(["build", "windows", "--project"])
            .arg(&project)
            .arg("--platform-pack")
            .arg(&pack)
            .arg("--wrangler")
            .arg(project.join("dist/server/wrangler.json"))
            .args(["--env", "production"])
            .env("TOKAMAK_VERSION", "1.0.0")
            .env("TOKAMAK_TEST_ESBUILD_LOG", &esbuild_log)
            .assert()
            .success();
        Ok(())
    };
    build()?;
    fs::write(
        project.join("wrangler.jsonc"),
        r#"{
  "name": "built-app",
  "vars": { "API": "default" },
  "env": { "production": { "vars": { "API": "promoted" } } }
}"#,
    )?;
    build()?;

    let environment: serde_json::Value = serde_json::from_slice(&fs::read(
        project.join("build/windows/built-app/app/worker-environment.json"),
    )?)?;
    assert_eq!(environment["vars"]["API"], "promoted");
    assert_eq!(fs::read_to_string(esbuild_log)?.lines().count(), 1);
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
            "--set",
            "ios-build-number=5",
            "--set",
            "ios-signing-identity=Apple Development: Test",
            "--set",
        ])
        .arg(format!("ios-provisioning-profile={}", profile.display()))
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
            command.args(["--set", "ios-team-id=TEAM"]);
        } else {
            command.env("TOKAMAK_IOS_TEAM_ID", "TEAM");
        }
        command.assert().failure().stderr(
            contains("automatic and manual iOS signing cannot be combined.")
                .and(contains(
                    "Automatic: set TOKAMAK_IOS_TEAM_ID or use --set ios-team-id=TEAM_ID",
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
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    drop(listener);

    let mut command = Command::cargo_bin("tok")?;
    configure_fake_apple_tools(&mut command, temporary)?;
    let server = format!("http://127.0.0.1:{port}");
    let framework = format!(
        "require('http').createServer((_, response) => response.end()).listen({port}, '127.0.0.1')"
    );
    command
        .args(["dev", "DEVICE", "--project"])
        .arg(project_arg)
        .args(["--platform-pack"])
        .arg(platform_pack)
        .args(["--config"])
        .arg(project)
        .args(["--server", &server, "--host-address", "127.0.0.1"])
        .args(["--set", "ios-signing-identity=IDENTITY_SHA1", "--set"])
        .arg(format!("ios-provisioning-profile={}", profile.display()))
        .env("TOKAMAK_IOS_TEAM_ID", "TEAM")
        .args(["--", "node", "-e"])
        .arg(framework);
    Ok(command)
}

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
                    "Automatic: set TOKAMAK_IOS_TEAM_ID or use --set ios-team-id=TEAM_ID",
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
    }
    Ok(())
}

#[test]
fn writes_configured_asset_routing_modes() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("macos-arm64")?;
    fs::write(
        project.join("wrangler.jsonc"),
        r#"{
  "name": "demo-app",
  "main": "dist/server/entry.mjs",
  "assets": {
    "directory": "dist/client",
    "html_handling": "drop-trailing-slash",
    "not_found_handling": "single-page-application"
  }
}"#,
    )?;
    build_command("macos", &project, &manifest)?
        .assert()
        .success();

    let path = project.join("build/macos/demo-app.app/Contents/Resources/app/asset-manifest.json");
    let value: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
    assert_eq!(value["htmlHandling"], "drop-trailing-slash");
    assert_eq!(value["notFoundHandling"], "single-page-application");
    Ok(())
}

#[test]
fn packages_declared_non_code_modules_under_bundle() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("macos-arm64")?;
    fs::create_dir_all(project.join("dist/server/config"))?;
    fs::write(
        project.join("dist/server/config/runtime.json"),
        br#"{"feature":true}"#,
    )?;
    fs::write(project.join("dist/server/not-included.txt"), "private")?;
    fs::write(
        project.join("wrangler.jsonc"),
        r#"{
  "name": "demo-app",
  "main": "dist/server/entry.mjs",
  "base_dir": "dist/server",
  "find_additional_modules": true,
  "rules": [{ "type": "Data", "globs": ["**/*.json"] }]
}"#,
    )?;

    build_command("macos", &project, &manifest)?
        .assert()
        .success();

    let app = project.join("build/macos/demo-app.app/Contents/Resources/app");
    assert_eq!(
        fs::read(app.join("bundle/config/runtime.json"))?,
        br#"{"feature":true}"#
    );
    assert!(!app.join("bundle/not-included.txt").exists());
    Ok(())
}

#[test]
fn ignores_unreferenced_webassembly_modules() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("macos-arm64")?;
    fs::write(project.join("dist/server/module.wasm"), b"wasm")?;

    build_command("macos", &project, &manifest)?
        .assert()
        .success();
    let app = project.join("build/macos/demo-app.app/Contents/Resources/app");
    assert!(!app.join("bundle/module.wasm").exists());
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
fn rejects_unsafe_wrangler_names() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("macos-arm64")?;
    fs::write(
        project.join("wrangler.jsonc"),
        r#"{"name":"../demo","main":"dist/server/entry.mjs"}"#,
    )?;

    build_command("macos", &project, &manifest)?
        .assert()
        .failure()
        .stderr(contains("wrangler config name is not a safe app name"));
    Ok(())
}

#[test]
fn rejects_wrangler_names_that_are_not_dns_labels() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("macos-arm64")?;
    fs::write(
        project.join("wrangler.jsonc"),
        r#"{"name":"Demo_App","main":"dist/server/entry.mjs"}"#,
    )?;

    build_command("macos", &project, &manifest)?
        .assert()
        .failure()
        .stderr(contains("wrangler config name is not a safe app name"));
    Ok(())
}

#[test]
fn requires_a_wrangler_name() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("macos-arm64")?;
    fs::write(
        project.join("wrangler.jsonc"),
        r#"{"main":"dist/server/entry.mjs"}"#,
    )?;

    build_command("macos", &project, &manifest)?
        .assert()
        .failure()
        .stderr(contains("missing required field name"));
    Ok(())
}

#[test]
fn rejects_wrangler_names_outside_dns_label_bounds() -> TestResult {
    let too_long = "a".repeat(64);
    for name in ["-demo", "demo-", &too_long] {
        let (_temporary, project, manifest) = create_inputs("macos-arm64")?;
        fs::write(
            project.join("wrangler.jsonc"),
            format!(r#"{{"name":"{name}","main":"dist/server/entry.mjs"}}"#),
        )?;

        build_command("macos", &project, &manifest)?
            .assert()
            .failure()
            .stderr(contains("wrangler config name is not a safe app name"));
    }
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
    let target = Target::MacosArm64;
    write_manifest(
        &manifest_path,
        &PlatformPackManifest {
            tokamak_version: "9.9.9".to_owned(),
            target,
            artifacts: target.artifacts(),
            required_tools: target
                .required_tools()
                .iter()
                .map(|tool| (*tool).to_owned())
                .collect(),
        },
    )?;

    build_command("macos", &project, &platform_pack)?
        .assert()
        .failure()
        .stderr(contains("platform pack was built for tokamak 9.9.9"));
    Ok(())
}
