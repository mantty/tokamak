use std::fs;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::Command as ProcessCommand;

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;
use tokamak::compile_module;
use tokamak::{
    HtmlHandling, ModuleType, NotFoundHandling, PackageLayout, read_asset_manifest,
    read_worker_manifest, read_worker_module,
};
use tokamak_cli::{
    MANIFEST_FILE, PackVariable, Platform, PlatformPackManifest, Target, VariableKind,
    write_manifest,
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
    fs::create_dir_all(plugin.join("apple/ios"))?;
    fs::create_dir_all(plugin.join("apple/macos"))?;
    for (path, contents) in [
        (
            "tokamak-plugin.json",
            include_str!("../../plugins/location/tokamak-plugin.json"),
        ),
        (
            "apple/LocationPlugin.swift",
            include_str!("../../plugins/location/apple/LocationPlugin.swift"),
        ),
        (
            "apple/ios/Info.plist",
            include_str!("../../plugins/location/apple/ios/Info.plist"),
        ),
        (
            "apple/macos/Info.plist",
            include_str!("../../plugins/location/apple/macos/Info.plist"),
        ),
    ] {
        fs::write(plugin.join(path), contents)?;
    }
    Ok(())
}

/// An entrypoint that records how tok runs it: its output holds the input tok
/// staged, its arguments, the directory it ran from, and the tokamak settings
/// it received.
const RECORDING_ENTRYPOINT: &str = r#"#!/usr/bin/env bash
set -euo pipefail
output=$3
rm -rf "$output"
mkdir -p "$(dirname "$output")"
cp -R "$2" "$output"
printf '%s\n' "$@" > "$output/arguments"
pwd > "$output/directory"
env | grep '^TOKAMAK_' | LC_ALL=C sort > "$output/environment" || true
"#;

fn create_platform_pack(root: &Path, target: &str) -> TestResult<PathBuf> {
    let target = target.parse::<Target>()?;
    let entrypoint = root.join(target.build_entrypoint_path());
    fs::create_dir_all(entrypoint.parent().ok_or("entrypoint path has no parent")?)?;
    fs::write(entrypoint, RECORDING_ENTRYPOINT)?;
    write_test_manifest(root, target)?;
    Ok(root.to_path_buf())
}

/// The value of the tokamak setting `name` that the recording entrypoint with
/// `output` received.
fn received(output: &Path, name: &str) -> TestResult<Option<String>> {
    let prefix = format!("{name}=");
    Ok(fs::read_to_string(output.join("environment"))?
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .map(str::to_owned))
}

/// Install the `alerts` plugin, whose Android section is `test.alerts.Plugin`
/// built from an empty Kotlin source.
fn install_android_plugin(root: &Path) -> TestResult<PathBuf> {
    fs::write(
        root.join("package.json"),
        r#"{"name":"demo-app","dependencies":{"alerts":"1.0.0"}}"#,
    )?;
    let plugin = root.join("node_modules/alerts");
    fs::create_dir_all(plugin.join("android"))?;
    fs::write(plugin.join("android/Plugin.kt"), "")?;
    let manifest = serde_json::json!({
        "schemaVersion": 1,
        "id": "alerts",
        "platforms": {
            "android": { "class": "test.alerts.Plugin", "sources": ["android/Plugin.kt"] },
        },
    });
    fs::write(plugin.join("tokamak-plugin.json"), manifest.to_string())?;
    Ok(plugin)
}

fn write_test_manifest(root: &Path, target: Target) -> TestResult {
    write_manifest(
        root.join(MANIFEST_FILE),
        &test_manifest(target, env!("CARGO_PKG_VERSION"))?,
    )?;
    Ok(())
}

/// A manifest declaring the pack's real variables and plugin keys, and the
/// variable `test`.
fn test_manifest(target: Target, tokamak_version: &str) -> TestResult<PlatformPackManifest> {
    let (declarations, plugin_keys) = match target.platform() {
        Platform::Android => (
            include_str!("../../platforms/android/build/variables.json"),
            include_str!("../../platforms/android/build/plugin-keys.json"),
        ),
        Platform::Windows => (
            include_str!("../../platforms/windows/build/variables.json"),
            include_str!("../../platforms/windows/build/plugin-keys.json"),
        ),
        Platform::Ios | Platform::IosSimulator | Platform::Macos => (
            include_str!("../../platforms/apple/build/variables.json"),
            include_str!("../../platforms/apple/build/plugin-keys.json"),
        ),
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
            description: "A test setting".to_owned(),
        },
    );
    Ok(PlatformPackManifest {
        tokamak_version: tokamak_version.to_owned(),
        target,
        variables,
        plugin_keys: serde_json::from_str(plugin_keys)?,
    })
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

/// A Windows platform pack with a test runtime and the real entrypoint, which
/// runs on Windows; elsewhere, the recording entrypoint.
#[cfg(windows)]
fn create_windows_inputs() -> TestResult<(tempfile::TempDir, PathBuf, PathBuf)> {
    let temporary = tempfile::tempdir()?;
    let project = temporary.path().join("project");
    let pack = temporary.path().join("pack");
    fs::create_dir_all(&project)?;
    create_project(&project)?;
    create_windows_runtime(&pack.join("lib/TokamakRuntime"))?;
    fs::create_dir_all(pack.join("build"))?;
    fs::write(
        pack.join("build/entrypoint.ps1"),
        include_str!("../../platforms/windows/build/entrypoint.ps1"),
    )?;
    write_test_manifest(&pack, Target::WindowsX64)?;
    Ok((temporary, project, pack))
}

#[cfg(not(windows))]
fn create_windows_inputs() -> TestResult<(tempfile::TempDir, PathBuf, PathBuf)> {
    create_inputs("windows-x64")
}

/// A runtime library for the Windows entrypoint to link, whose app exits
/// with 0 while it exports the storage part and 1 otherwise.
#[cfg(windows)]
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

#[cfg(windows)]
fn create_windows_runtime(runtime: &Path) -> TestResult {
    fs::create_dir_all(runtime)?;
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
#[cfg(windows)]
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

/// A `tok build` that builds the project, without the developer's tokamak
/// settings.
fn project_build_command(
    platform: &str,
    project: &Path,
    platform_pack: &Path,
) -> TestResult<Command> {
    let mut command = Command::cargo_bin("tok")?;
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("TOKAMAK_") {
            command.env_remove(name);
        }
    }
    command
        .args(["build", platform, "--project"])
        .arg(project)
        .arg("--platform-pack")
        .arg(platform_pack)
        .env("TOKAMAK_VERSION", "1.0.0");
    Ok(command)
}

#[cfg(target_os = "macos")]
fn write_executable(path: &Path, contents: &str) -> TestResult {
    use std::os::unix::fs::PermissionsExt;

    fs::create_dir_all(path.parent().ok_or("executable path has no parent")?)?;
    fs::write(path, contents)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

#[test]
fn stages_the_entry_points_of_the_runtime_parts_the_app_links() -> TestResult {
    let (_temporary, project, platform_pack) = create_inputs("android-arm64")?;
    let exported_symbols = || -> TestResult<String> {
        build_command("android", &project, &platform_pack)?
            .assert()
            .success();
        Ok(fs::read_to_string(project.join(
            "build/android/demo-app.apk/metadata/exported-symbols",
        ))?)
    };

    assert_eq!(exported_symbols()?, "");
    declare_storage(&project)?;
    assert_eq!(exported_symbols()?, "tokamak_storage\n");
    Ok(())
}

#[test]
fn stages_the_app_and_its_metadata_and_runs_the_entrypoint_from_the_pack_root() -> TestResult {
    let (_temporary, project, platform_pack) = create_inputs("macos-arm64")?;
    build_command("macos", &project, &platform_pack)?
        .assert()
        .success()
        .stdout(contains("Built macOS bundle"));

    let project = fs::canonicalize(&project)?;
    let output = project.join("build/macos/demo-app.app");
    let mut staged = fs::read_dir(&output)?
        .map(|entry| Ok(entry?.file_name().into_string().map_err(|_| "UTF-8 name")?))
        .collect::<TestResult<Vec<_>>>()?;
    staged.sort();
    assert_eq!(
        staged,
        ["app", "arguments", "directory", "environment", "metadata"]
    );
    let input = project.join("build/.tokamak/macos/input");
    assert_eq!(
        fs::read_to_string(output.join("arguments"))?,
        format!("build\n{}\n{}\n", input.display(), output.display())
    );
    assert_eq!(
        PathBuf::from(fs::read_to_string(output.join("directory"))?.trim_end()),
        fs::canonicalize(&platform_pack)?
    );
    let project_dir = project.display().to_string();
    for (name, value) in [
        ("app-name", "demo-app"),
        ("app-slug", "demo-app"),
        ("identifier", "com.tokamak.demo-app"),
        ("host", "demo-app.tokamak.local"),
        ("platform", "macos"),
        ("target", "macos-arm64"),
        ("project-dir", &project_dir),
        ("version", "1.0.0"),
    ] {
        assert_eq!(
            fs::read_to_string(output.join("metadata").join(name))?,
            value,
            "{name}"
        );
    }

    let app = PackageLayout::new(output.join("app"));
    let manifest = read_worker_manifest(&app)?;
    assert_eq!(manifest.entry, "index.js");
    assert_eq!(
        read_worker_module(&app, "index.js")?,
        compile_module("index.js", &fs::read(project.join("dist/app/index.js"))?)?
    );
    assert!(output.join("app/assets/index.html").is_file());
    let manifest = read_asset_manifest(&app)?;
    assert_eq!(manifest.files["styles/app.css"], "text/css");
    assert!(!output.join("app/config.capnp").exists());
    Ok(())
}

#[test]
fn stages_the_identifier_and_version_from_each_source() -> TestResult {
    let (_temporary, project, platform_pack) = create_inputs("macos-arm64")?;
    configure(
        &project,
        r#"{
  "identifier": "com.example.app",
  "macos": { "identifier": "com.example.desktop" },
  "version": "2.3.4"
}"#,
    )?;
    let metadata = project.join("build/macos/demo-app.app/metadata");
    let staged = |name: &str| fs::read_to_string(metadata.join(name));

    build_command("macos", &project, &platform_pack)?
        .env_remove("TOKAMAK_VERSION")
        .assert()
        .success();
    assert_eq!(staged("identifier")?, "com.example.desktop");
    assert_eq!(staged("version")?, "2.3.4");

    build_command("macos", &project, &platform_pack)?
        .env("TOKAMAK_MACOS_IDENTIFIER", "com.example.environment")
        .env("TOKAMAK_VERSION", "1.0'beta\"")
        .assert()
        .success();
    assert_eq!(staged("identifier")?, "com.example.environment");
    assert_eq!(staged("version")?, "1.0'beta\"");
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

#[test]
fn stages_the_display_name_and_names_the_output_after_its_slug() -> TestResult {
    for (platform, target, output) in [
        ("macos", "macos-arm64", "build/macos/vigilus-co-pro-x.app"),
        (
            "android",
            "android-arm64",
            "build/android/vigilus-co-pro-x.apk",
        ),
        ("windows", "windows-x64", "build/windows/vigilus-co-pro-x"),
    ] {
        let (_temporary, project, platform_pack) = create_inputs(target)?;
        configure(&project, r#"{"name":"Vigilus & <Co> \"Pro\" 'X'"}"#)?;

        build_command(platform, &project, &platform_pack)?
            .assert()
            .success();

        let metadata = project.join(output).join("metadata");
        assert_eq!(
            fs::read_to_string(metadata.join("app-name"))?,
            "Vigilus & <Co> \"Pro\" 'X'"
        );
        assert_eq!(
            fs::read_to_string(metadata.join("app-slug"))?,
            "vigilus-co-pro-x"
        );
        assert_eq!(
            fs::read_to_string(metadata.join("host"))?,
            "vigilus-co-pro-x.tokamak.local"
        );
    }
    Ok(())
}

#[test]
fn passes_options_then_environment_then_configured_values_to_the_entrypoint() -> TestResult {
    let (_temporary, project, platform_pack) = create_inputs("android-arm64")?;
    configure(&project, r#"{ "android": { "test": "configured" } }"#)?;
    let output = project.join("build/android/demo-app.apk");
    let build = |configure: &dyn Fn(&mut Command)| -> TestResult<Option<String>> {
        let mut command = build_command("android", &project, &platform_pack)?;
        configure(&mut command);
        command.assert().success();
        received(&output, "TOKAMAK_ANDROID_TEST")
    };

    assert_eq!(build(&|_| {})?.as_deref(), Some("configured"));
    assert_eq!(
        build(&|command| {
            command.env("TOKAMAK_ANDROID_TEST", "environment");
        })?
        .as_deref(),
        Some("environment")
    );
    assert_eq!(
        build(&|command| {
            command
                .env("TOKAMAK_ANDROID_TEST", "environment")
                .args(["--android-test", "option"]);
        })?
        .as_deref(),
        Some("option")
    );
    Ok(())
}

#[test]
fn passes_path_settings_and_the_icon_as_absolute_paths() -> TestResult {
    let (_temporary, project, platform_pack) = create_inputs("macos-arm64")?;
    configure(&project, r#"{ "icon": "assets/AppIcon.icon" }"#)?;

    build_command("macos", &project, &platform_pack)?
        .current_dir(&project)
        .args(["--macos-plist", "native/Info.plist"])
        .assert()
        .success();

    let output = project.join("build/macos/demo-app.app");
    let path = |path: PathBuf| Some(path.display().to_string());
    assert_eq!(
        received(&output, "TOKAMAK_MACOS_ICON")?,
        path(project.join("src/assets/AppIcon.icon"))
    );
    assert_eq!(
        received(&output, "TOKAMAK_MACOS_PLIST")?,
        path(fs::canonicalize(&project)?.join("native/Info.plist"))
    );
    Ok(())
}

#[test]
fn rejects_keys_the_platform_pack_does_not_declare() -> TestResult {
    let (_temporary, project, platform_pack) = create_inputs("android-arm64")?;
    configure(
        &project,
        r#"{ "android": { "tset": "x" }, "ios": { "tset": "ignored" } }"#,
    )?;

    build_command("android", &project, &platform_pack)?
        .assert()
        .failure()
        .stderr(contains("unknown key android.tset in"));

    build_command("android", &project, &platform_pack)?
        .args(["--android-manifset", "AndroidManifest.xml"])
        .assert()
        .failure()
        .stderr(contains(
            "unknown option --android-manifset; Android accepts",
        ));
    Ok(())
}

#[test]
fn lists_the_platform_pack_options_in_build_help() -> TestResult {
    let (_temporary, _project, platform_pack) = create_inputs("android-arm64")?;
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

#[test]
fn checks_settings_before_building_the_project() -> TestResult {
    let (_temporary, project, platform_pack) = create_inputs("android-arm64")?;
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

#[test]
fn checks_every_platform_s_plugin_sections_before_any_platform_builds() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let project = temporary.path().join("project");
    let packs = temporary.path().join("platform-packs");
    fs::create_dir_all(&project)?;
    create_project(&project)?;
    create_platform_pack(&packs.join("android-arm64"), "android-arm64")?;
    create_platform_pack(&packs.join("ios-arm64"), "ios-arm64")?;
    let plugin_manifest = install_android_plugin(&project)?.join("tokamak-plugin.json");
    let mut plugin: serde_json::Value = serde_json::from_slice(&fs::read(&plugin_manifest)?)?;
    plugin["platforms"]["ios"] = serde_json::json!({ "class": "Alerts", "frameworks": ["UIKit"] });
    fs::write(&plugin_manifest, plugin.to_string())?;

    Command::cargo_bin("tok")?
        .args(["build", "android,ios", "--skip-project-build", "--project"])
        .arg(&project)
        .env("TOKAMAK_VERSION", "1.0.0")
        .env("TOKAMAK_PLATFORM_PACK_PATH", &packs)
        .assert()
        .failure()
        .stderr(contains("plugin 'alerts' has unknown ios key 'frameworks'"));
    assert!(!project.join("build/.tokamak/android").exists());
    assert!(!project.join("build/.tokamak/worker").exists());
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

    let app = project.join("build/macos/demo-app.app/app");
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
        project.join("build/macos/demo-app.app/app/worker-environment.json"),
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

#[test]
fn stages_the_plugins_installed_above_a_relative_project_directory() -> TestResult {
    let (temporary, project, platform_pack) = create_inputs("macos-arm64")?;
    install_location_plugin(&project)?;
    fs::rename(
        project.join("node_modules"),
        temporary.path().join("node_modules"),
    )?;

    build_command("macos", Path::new("."), &platform_pack)?
        .current_dir(&project)
        .assert()
        .success();

    let plugin = project.join("build/macos/demo-app.app/plugins/location");
    assert_eq!(
        fs::read_to_string(plugin.join("class"))?,
        "TokamakLocationPlugin"
    );
    assert_eq!(
        fs::read_to_string(plugin.join("plist"))?,
        include_str!("../../plugins/location/apple/macos/Info.plist")
    );
    assert!(plugin.join("sources/0-LocationPlugin.swift").is_file());
    Ok(())
}

#[cfg(windows)]
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
    assert_eq!(
        config,
        serde_json::json!({
            "name": "demo-app",
            "slug": "demo-app",
            "host": "demo-app.tokamak.local",
        })
    );
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

#[cfg(windows)]
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

#[cfg(windows)]
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

/// `tok dev` for the iOS device `DEVICE`, whose platform pack's entrypoint
/// reports the device and project it builds for, then fails.
#[cfg(target_os = "macos")]
fn dev_command(temporary: &Path, platform_pack: &Path, project: &Path) -> TestResult<Command> {
    use std::ffi::OsString;

    fs::write(
        platform_pack.join("build/entrypoint"),
        "echo \"building for $(cat \"$2/metadata/device-id\") from $(cat \"$2/metadata/project-dir\")\" >&2\nexit 1\n",
    )?;
    let bin = temporary.join("fake-devices");
    write_executable(
        &bin.join("xcrun"),
        r#"#!/bin/sh
case " $* " in
  *" simctl "*) printf '%s\n' '{"devices":{"com.apple.CoreSimulator.SimRuntime.iOS-17-0":[]}}' ;;
  *" devicectl "*) printf '%s\n' '{"devices":[{"identifier":"DEVICE","platform":"iOS","name":"Test iPhone"}]}' ;;
  *) exit 1 ;;
esac
"#,
    )?;
    let mut path = OsString::from(bin);
    path.push(":");
    path.push(std::env::var_os("PATH").ok_or("PATH unavailable")?);

    let mut command = Command::cargo_bin("tok")?;
    command
        .args(["dev", "DEVICE", "--project"])
        .arg(project)
        .arg("--platform-pack")
        .arg(platform_pack)
        .args(["--host-address", "127.0.0.1"])
        .env("PATH", path)
        .args(["--", "node", "-e", DEV_SERVER]);
    Ok(command)
}

/// A development server that reports the stand-in build's configuration, and
/// itself and its Worker as the tokamak Vite plugin does.
#[cfg(target_os = "macos")]
const DEV_SERVER: &str = r#"
require("./build.cjs");
const server = require("node:http").createServer((_, response) => response.end());
server.listen(0, "127.0.0.1", () => {
  const url = `http://127.0.0.1:${server.address().port}/`;
  const report = require("node:path").join(process.env.TOKAMAK_VITE_OUTPUT, "server.json");
  require("node:fs").writeFileSync(report, JSON.stringify({ url, workerName: "demo-app" }));
});
"#;

#[cfg(target_os = "macos")]
#[test]
fn dev_runs_the_entrypoint_for_a_relative_project_directory() -> TestResult {
    let (temporary, project, platform_pack) = create_inputs("ios-arm64")?;
    dev_command(temporary.path(), &platform_pack, Path::new("."))?
        .current_dir(&project)
        .assert()
        .failure()
        .stderr(contains(format!(
            "building for DEVICE from {}",
            fs::canonicalize(&project)?.display()
        )));
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn dev_selects_an_ios_device_when_android_devices_cannot_be_queried() -> TestResult {
    let (temporary, project, platform_pack) = create_inputs("ios-arm64")?;
    dev_command(temporary.path(), &platform_pack, &project)?
        .env("ANDROID_ADB_SERVER_PORT", "0")
        .assert()
        .failure()
        .stderr(contains("building for DEVICE from"));
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

    let app = PackageLayout::new(project.join("build/macos/demo-app.app/app"));
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

    let app = project.join("build/macos/demo-app.app/app");
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
    let app = project.join("build/macos/demo-app.app/app");
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
