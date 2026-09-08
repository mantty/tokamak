use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use assert_cmd::Command;
use predicates::str::contains;
use tokamak::compile_module;
use tokamak::{PackageLayout, decompress_worker_module, read_worker_manifest};
use tokamak_cli::{
    ESBUILD_EXECUTABLE, MANIFEST_FILE, RUNTIME_JAVASCRIPT_DIRECTORY, Target, TargetPackManifest,
    write_manifest,
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

fn install_geolocation_plugin(root: &Path) -> TestResult {
    fs::write(
        root.join("package.json"),
        r#"{"name":"demo-app","scripts":{"build":"echo already-built"},"dependencies":{"@tokamak/geolocation":"1.0.0"}}"#,
    )?;
    let plugin = root.join("node_modules/@tokamak/geolocation");
    fs::create_dir_all(plugin.join("apple"))?;
    fs::write(
        plugin.join("apple/GeolocationPlugin.swift"),
        include_str!("../../plugins/geolocation/apple/GeolocationPlugin.swift"),
    )?;
    fs::write(
        plugin.join("tokamak-plugin.json"),
        r#"{
  "schemaVersion": 1,
  "id": "geolocation",
  "kind": "frontend",
  "platforms": {
    "macos": {
      "class": "TokamakGeolocationPlugin",
      "sources": ["apple/GeolocationPlugin.swift"],
      "frameworks": ["CoreLocation"],
      "plist": {"NSLocationUsageDescription": "Location test"}
    },
    "ios": {
      "class": "TokamakGeolocationPlugin",
      "sources": ["apple/GeolocationPlugin.swift"],
      "frameworks": ["CoreLocation"],
      "plist": {"NSLocationWhenInUseUsageDescription": "Location test"}
    },
    "ios-simulator": {
      "class": "TokamakGeolocationPlugin",
      "sources": ["apple/GeolocationPlugin.swift"],
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
  assets: { directory: "../client", binding: "ASSETS" }
}));
"#,
    )?;
    Ok(())
}

fn write_test_esbuild(root: &Path) -> TestResult {
    let path = root.join(ESBUILD_EXECUTABLE);
    fs::create_dir_all(path.parent().ok_or("esbuild path has no parent")?)?;
    fs::write(
        &path,
        "#!/usr/bin/env node\nconst fs = require('node:fs');\nconst path = require('node:path');\nconst args = process.argv.slice(2);\nconst output = args.find((arg) => arg.startsWith('--outdir=')).slice('--outdir='.length);\nconst metafile = args.find((arg) => arg.startsWith('--metafile='));\nconst input = args.at(-1);\nfs.mkdirSync(output, { recursive: true });\nfs.copyFileSync(input, path.join(output, 'entry.js'));\nif (metafile) fs.writeFileSync(metafile.slice('--metafile='.length), JSON.stringify({ inputs: { [input]: {} } }));\n",
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

fn create_target_pack(root: &Path, target: &str) -> TestResult<PathBuf> {
    let target_name = target;
    let target = target_name.parse::<Target>()?;
    let framework = root.join(target.runtime_artifact_path());
    let shell = root.join("native-shell");
    fs::create_dir_all(&framework)?;
    fs::create_dir_all(&shell)?;
    fs::create_dir_all(root.join(RUNTIME_JAVASCRIPT_DIRECTORY))?;
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
    write_manifest(
        root.join(MANIFEST_FILE),
        &TargetPackManifest {
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
    Ok(root.to_path_buf())
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
    let target_pack = create_target_pack(&pack, target)?;
    Ok((temporary, project, target_pack))
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
    fs::create_dir_all(pack.join(RUNTIME_JAVASCRIPT_DIRECTORY))?;
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
host=$(cat "$input/metadata/host")
rm -rf "$output"
mkdir -p "$output/app"
cp -R "$input/app/." "$output/app/"
cp "$input/runtime/tokamak-shell-windows.exe" "$output/$app_name.exe"
if [ -f "$input/icons/windows/AppIcon.ico" ]; then
  cp "$input/icons/windows/AppIcon.ico" "$output/AppIcon.ico"
fi
printf '{"name":"%s","host":"%s"}\n' "$app_name" "$host" > "$output/tokamak.json"
"#
    };
    fs::write(pack.join(entrypoint_path), entrypoint)?;
    write_test_esbuild(&pack)?;
    write_manifest(
        pack.join(MANIFEST_FILE),
        &TargetPackManifest {
            tokamak_version: env!("CARGO_PKG_VERSION").to_owned(),
            target,
            artifacts: target.artifacts(),
            required_tools: Vec::new(),
        },
    )?;
    Ok((temporary, project, pack))
}

fn build_command(platform: &str, project: &Path, target_pack: &Path) -> TestResult<Command> {
    let mut command = Command::cargo_bin("tok")?;
    command
        .args(["build", platform, "--project"])
        .arg(project)
        .arg("--target-pack")
        .arg(target_pack)
        .arg("--skip-web-build");
    Ok(command)
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
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(app.join("asset-manifest.json"))?)?;
    assert_eq!(manifest["files"]["styles/app.css"], "text/css");
    assert!(!app.join("config.capnp").exists());
    Ok(())
}

#[test]
fn builds_a_configured_macos_icon() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("macos-arm64")?;
    fs::create_dir_all(project.join("assets"))?;
    fs::write(project.join("assets/AppIcon.icns"), "icns")?;
    fs::write(
        project.join("tokamak.jsonc"),
        r#"{
  "icons": {
    "macos": "assets/AppIcon.icns"
  }
}"#,
    )?;

    let mut command = build_command("macos", &project, &manifest)?;
    command.arg("--config").arg(project.join("tokamak.jsonc"));
    command.assert().success();

    let bundle = project.join("build/macos/demo-app.app");
    assert_eq!(
        fs::read(bundle.join("Contents/Resources/AppIcon.icns"))?,
        b"icns"
    );
    let plist = fs::read_to_string(bundle.join("Contents/Info.plist"))?;
    assert!(plist.contains("<key>CFBundleIconFile</key><string>AppIcon.icns</string>"));
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
fn builds_declared_plugins_into_the_macos_shell() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("macos-arm64")?;
    install_geolocation_plugin(&project)?;

    build_command("macos", &project, &manifest)?
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

#[test]
fn builds_a_configured_windows_icon() -> TestResult {
    let (_temporary, project, manifest) = create_windows_inputs()?;
    fs::write(project.join("AppIcon.ico"), "ico")?;
    fs::write(
        project.join("tokamak.jsonc"),
        r#"{"icons":{"windows":"AppIcon.ico"}}"#,
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
fn requires_a_target_pack_for_app_builds() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let project = temporary.path().join("project");
    fs::create_dir_all(&project)?;
    create_project(&project)?;

    let mut command = Command::cargo_bin("tok")?;
    command
        .args(["build", "macos", "--project"])
        .arg(&project)
        .arg("--skip-web-build")
        .assert()
        .failure()
        .stderr(contains("no target pack found"));
    Ok(())
}

#[test]
fn reads_target_pack_directory_from_tokamak_environment() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let project = temporary.path().join("project");
    let target_packs = temporary.path().join("target-packs");
    fs::create_dir_all(&project)?;
    fs::create_dir_all(&target_packs)?;
    create_project(&project)?;

    let mut command = Command::cargo_bin("tok")?;
    command
        .args(["build", "macos", "--project"])
        .arg(&project)
        .arg("--skip-web-build")
        .env("TOKAMAK_TARGET_PACK_DIR", target_packs)
        .assert()
        .failure()
        .stderr(contains(
            "TOKAMAK_TARGET_PACK_DIR does not contain a target pack",
        ));
    Ok(())
}

#[test]
fn builds_web_project_before_loading_generated_config() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let project = temporary.path().join("project");
    let pack = temporary.path().join("pack");
    fs::create_dir_all(&project)?;
    fs::create_dir_all(&pack)?;
    create_unbuilt_project(&project)?;
    let target_pack = create_target_pack(&pack, "macos-arm64")?;
    let config = project.join("dist/server/wrangler.json");

    let mut command = Command::cargo_bin("tok")?;
    command
        .args(["build", "macos", "--project"])
        .arg(&project)
        .arg("--target-pack")
        .arg(target_pack)
        .arg("--wrangler")
        .arg(config);
    command.assert().success();

    assert!(project.join("build/macos/built-app.app").is_dir());
    Ok(())
}

#[test]
fn builds_physical_ios_app() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("ios-arm64")?;
    install_geolocation_plugin(&project)?;
    build_command("ios", &project, &manifest)?
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
    assert!(plist.contains("NSLocationWhenInUseUsageDescription"));
    Ok(())
}

#[test]
fn builds_ios_simulator_app() -> TestResult {
    for target in ["ios-simulator-arm64", "ios-simulator-x64"] {
        let (_temporary, project, manifest) = create_inputs(target)?;
        install_geolocation_plugin(&project)?;
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
fn rejects_target_pack_for_another_platform() -> TestResult {
    let (_temporary, project, manifest) = create_inputs("ios-arm64")?;
    build_command("macos", &project, &manifest)?
        .assert()
        .failure()
        .stderr(contains("target pack ios-arm64 cannot build macOS"));
    Ok(())
}

#[test]
fn rejects_target_pack_for_another_cli_version() -> TestResult {
    let (_temporary, project, target_pack) = create_inputs("macos-arm64")?;
    let manifest_path = target_pack.join(MANIFEST_FILE);
    let target = Target::MacosArm64;
    write_manifest(
        &manifest_path,
        &TargetPackManifest {
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

    build_command("macos", &project, &target_pack)?
        .assert()
        .failure()
        .stderr(contains("target pack was built for tokamak 9.9.9"));
    Ok(())
}
