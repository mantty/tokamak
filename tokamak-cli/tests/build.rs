use std::fs;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::Command as ProcessCommand;

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;
#[cfg(unix)]
use tokamak::{
    HtmlHandling, ModuleType, NotFoundHandling, read_asset_manifest, read_worker_manifest,
};
use tokamak::{PackageLayout, compile_module, read_worker_module};
use tokamak_cli::{
    MANIFEST_FILE, PackVariable, Platform, PlatformPackManifest, Target, VariableKind,
    write_manifest,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// A project whose earlier build left Worker output as Cloudflare's Vite
/// plugin writes it, and no `tokamak()` options as the tokamak Vite plugin
/// reports them. Its build command stands in for that build.
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
    configure(root, "{}")
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

/// Report `config` as the `tokamak()` options, with the Vite root `root`,
/// where the earlier build left the report and to `vite-report.json`, which
/// the stand-in build and development commands report.
fn configure(root: &Path, config: &str) -> TestResult {
    let config = serde_json::from_str::<serde_json::Value>(config)?;
    let report = serde_json::json!({ "root": root, "config": config }).to_string();
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

/// Install this repository's location plugin.
#[cfg(unix)]
fn install_location_plugin(root: &Path) -> TestResult {
    fs::write(
        root.join("package.json"),
        r#"{"name":"demo-app","scripts":{"build":"echo already-built"},"dependencies":{"@tokamakdev/plugin-location":"1.0.0"}}"#,
    )?;
    let plugin = root.join("node_modules/@tokamakdev/plugin-location");
    fs::create_dir_all(&plugin)?;
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins/location");
    tokamak_cli::copy_dir_contents(&source, &plugin)?;
    Ok(())
}

/// An entrypoint that records how tok runs it: its output holds the input tok
/// staged, its arguments, the directory it ran from, and the tokamak settings
/// it received. It is a Bash script recording Unix paths, so tests that run it
/// are Unix-only.
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
#[cfg(unix)]
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
            "android": { "class": "test.alerts.Plugin" },
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

/// Assert that each file holds its text.
#[track_caller]
fn assert_texts(files: &[(PathBuf, &str)]) -> TestResult {
    for (path, text) in files {
        assert_eq!(fs::read_to_string(path)?, *text, "{}", path.display());
    }
    Ok(())
}

#[cfg(unix)]
#[track_caller]
fn assert_file(path: &Path) {
    assert!(path.is_file(), "{} is not a file", path.display());
}

#[cfg(unix)]
#[track_caller]
fn assert_missing(path: &Path) {
    assert!(!path.exists(), "{} exists", path.display());
}

/// Assert that the packaged ES module `module` is `source` compiled.
#[track_caller]
fn assert_compiled(app: &PackageLayout, module: &str, source: &Path) -> TestResult {
    assert_eq!(
        read_worker_module(app, module)?,
        compile_module(module, &fs::read(source)?)?
    );
    Ok(())
}

/// Assert that `app` packages the Worker and assets of the demo `project`.
#[cfg(unix)]
#[track_caller]
fn assert_packages_demo_app(app: &Path, project: &Path) -> TestResult {
    let layout = PackageLayout::new(app);
    assert_eq!(read_worker_manifest(&layout)?.entry, "index.js");
    assert_compiled(&layout, "index.js", &project.join("dist/app/index.js"))?;
    assert_file(&app.join("assets/index.html"));
    assert_eq!(
        read_asset_manifest(&layout)?.files["styles/app.css"],
        "text/css"
    );
    assert_missing(&app.join("config.capnp"));
    Ok(())
}

#[cfg(unix)]
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

#[cfg(unix)]
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
    assert_texts(&[(
        output.join("arguments"),
        &format!("build\n{}\n{}\n", input.display(), output.display()),
    )])?;
    assert_eq!(
        PathBuf::from(fs::read_to_string(output.join("directory"))?.trim_end()),
        fs::canonicalize(&platform_pack)?
    );
    let project_dir = project.display().to_string();
    let metadata = output.join("metadata");
    assert_texts(&[
        (metadata.join("app-name"), "demo-app"),
        (metadata.join("app-slug"), "demo-app"),
        (metadata.join("identifier"), "com.tokamak.demo-app"),
        (metadata.join("host"), "demo-app.tokamak.local"),
        (metadata.join("platform"), "macos"),
        (metadata.join("target"), "macos-arm64"),
        (metadata.join("project-dir"), &project_dir),
        (metadata.join("version"), "1.0.0"),
    ])?;

    assert_packages_demo_app(&output.join("app"), &project)?;
    Ok(())
}

#[cfg(unix)]
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

    build_command("macos", &project, &platform_pack)?
        .env_remove("TOKAMAK_VERSION")
        .assert()
        .success();
    assert_texts(&[
        (metadata.join("identifier"), "com.example.desktop"),
        (metadata.join("version"), "2.3.4"),
    ])?;

    build_command("macos", &project, &platform_pack)?
        .env("TOKAMAK_MACOS_IDENTIFIER", "com.example.environment")
        .env("TOKAMAK_VERSION", "1.0'beta\"")
        .assert()
        .success();
    assert_texts(&[
        (metadata.join("identifier"), "com.example.environment"),
        (metadata.join("version"), "1.0'beta\""),
    ])?;
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

#[cfg(unix)]
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

#[cfg(unix)]
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
        path(project.join("assets/AppIcon.icon"))
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
        .stderr(contains(
            "unknown key android.tset in the tokamak() options",
        ));

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
                "Android options (also TOKAMAK_ANDROID_<KEY>, or android.<key> in the tokamak() options):",
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

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
fn configure_worker_graph(project: &Path) -> TestResult {
    write_worker_config(
        project,
        r#"{
        "name": "demo-app-production", "topLevelName": "demo-app",
        "vars": { "ROLE": "main" },
        "services": [
            { "binding": "TOKAMAK", "service": "device-production" },
            { "binding": "LOCATION", "service": "device-production", "entrypoint": "Location" }
        ]
    }"#,
    )?;
    let native = project.join("dist/device");
    fs::create_dir_all(native.join("migrations"))?;
    fs::write(
        native.join("index.js"),
        "export default {}; export class Location {};",
    )?;
    fs::write(
        native.join("migrations/001.sql"),
        "CREATE TABLE locations (latitude REAL);",
    )?;
    fs::write(
        native.join("wrangler.json"),
        r#"{
        "name": "device-production", "topLevelName": "device", "main": "index.js",
        "vars": { "ROLE": "native" },
        "services": [{ "binding": "MAIN", "service": "demo-app-production" }],
        "d1_databases": [{ "binding": "DB", "database_id": "locations" }]
    }"#,
    )?;
    fs::write(
        project.join(".wrangler/deploy/config.json"),
        r#"{
        "configPath": "../../dist/app/wrangler.json",
        "auxiliaryWorkers": [{ "configPath": "../../dist/device/wrangler.json" }]
    }"#,
    )?;

    Ok(())
}

#[cfg(unix)]
#[test]
fn packages_both_workers_with_their_own_bindings_and_migrations() -> TestResult {
    let (_temporary, project, platform_pack) = create_inputs("macos-arm64")?;
    configure_worker_graph(&project)?;
    build_command("macos", &project, &platform_pack)?
        .assert()
        .success();

    let output = project.join("build/macos/demo-app.app");
    let main = PackageLayout::new(output.join("app"));
    let native = main.tokamak_worker();
    assert_eq!(read_worker_manifest(&native)?.entry, "index.js");
    let assert_environment = |layout: &PackageLayout,
                              vars: serde_json::Value,
                              services: serde_json::Value|
     -> TestResult {
        let environment: serde_json::Value =
            serde_json::from_slice(&fs::read(layout.worker_environment())?)?;
        let worker = layout.root().display();
        assert_eq!(environment["vars"], vars, "vars of {worker}");
        assert_eq!(environment["services"], services, "services of {worker}");
        Ok(())
    };
    assert_environment(
        &main,
        serde_json::json!({ "ROLE": "main" }),
        serde_json::json!({
            "TOKAMAK": { "service": "tokamak", "entrypoint": "default" },
            "LOCATION": { "service": "tokamak", "entrypoint": "Location" }
        }),
    )?;
    assert_environment(
        &native,
        serde_json::json!({ "ROLE": "native" }),
        serde_json::json!({
            "MAIN": { "service": "main", "entrypoint": "default" }
        }),
    )?;
    assert_texts(&[
        (
            native.d1_migrations("DB").join("001.sql"),
            "CREATE TABLE locations (latitude REAL);",
        ),
        (
            output.join("metadata/exported-symbols"),
            "tokamak_storage\n",
        ),
    ])?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn caches_worker_bundles_independently_and_refreshes_native_environment() -> TestResult {
    let (_temporary, project, platform_pack) = create_inputs("macos-arm64")?;
    configure_worker_graph(&project)?;
    let build = || -> TestResult {
        build_command("macos", &project, &platform_pack)?
            .assert()
            .success();
        Ok(())
    };
    let build_dir = project.join("build");
    build()?;
    let main_time = worker_compiled_at(&build_dir, "main")?;
    let native_time = worker_compiled_at(&build_dir, "tokamak")?;
    let config_path = project.join("dist/device/wrangler.json");
    let mut config: serde_json::Value = serde_json::from_slice(&fs::read(&config_path)?)?;
    config["vars"]["ROLE"] = "updated".into();
    fs::write(&config_path, serde_json::to_vec(&config)?)?;
    build()?;
    assert_worker_reused(&build_dir, "main", main_time)?;
    assert_worker_reused(&build_dir, "tokamak", native_time)?;
    let app = PackageLayout::new(project.join("build/macos/demo-app.app/app"));
    let env: serde_json::Value =
        serde_json::from_slice(&fs::read(app.tokamak_worker().worker_environment())?)?;
    assert_eq!(env["vars"]["ROLE"], "updated");

    let source = "export default { value: 2 }; export class Location {};";
    fs::write(project.join("dist/device/index.js"), source)?;
    build()?;
    assert_worker_reused(&build_dir, "main", main_time)?;
    assert_worker_recompiled(&build_dir, "tokamak", native_time)?;
    assert_eq!(
        read_worker_module(&app.tokamak_worker(), "index.js")?,
        compile_module("index.js", source.as_bytes())?
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
    let compiled = worker_compiled_at(&build_dir, "main")?;

    assert_eq!(
        packaged_vars(r#"{ "STAGE": "production" }"#)?,
        serde_json::json!({ "STAGE": "production" })
    );
    assert_worker_reused(&build_dir, "main", compiled)?;

    let entry = project.join("dist/app/index.js");
    fs::write(&entry, "export default { value: 2 };")?;
    packaged_vars(r#"{ "STAGE": "production" }"#)?;
    assert_worker_recompiled(&build_dir, "main", compiled)?;
    assert_compiled(&PackageLayout::new(&app), "index.js", &entry)?;
    Ok(())
}

/// When the Worker `worker` of the build in `build_dir` was last compiled.
fn worker_compiled_at(build_dir: &Path, worker: &str) -> TestResult<std::time::SystemTime> {
    let compiled = PackageLayout::new(build_dir.join(format!(".tokamak/worker/{worker}/compiled")));
    Ok(fs::metadata(compiled.worker_manifest())?.modified()?)
}

/// Assert that the build in `build_dir` reused the Worker `worker` it compiled at `compiled`.
#[track_caller]
fn assert_worker_reused(
    build_dir: &Path,
    worker: &str,
    compiled: std::time::SystemTime,
) -> TestResult {
    assert_eq!(worker_compiled_at(build_dir, worker)?, compiled);
    Ok(())
}

/// Assert that the build in `build_dir` compiled the Worker `worker` again since `compiled`.
#[track_caller]
fn assert_worker_recompiled(
    build_dir: &Path,
    worker: &str,
    compiled: std::time::SystemTime,
) -> TestResult {
    assert_ne!(worker_compiled_at(build_dir, worker)?, compiled);
    Ok(())
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
    let compiled = worker_compiled_at(&project.join("build"), "main")?;
    fs::write(
        project.join("dist/client/index.html"),
        "<html>updated</html>",
    )?;
    build()?;

    assert_eq!(
        fs::read_to_string(project.join("build/windows/demo-app/app/assets/index.html"))?,
        "<html>updated</html>"
    );
    assert_eq!(
        worker_compiled_at(&project.join("build"), "main")?,
        compiled
    );
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
    let compiled = worker_compiled_at(&project.join("build"), "main")?;
    fs::write(worker.join("unrelated.md"), "changed")?;
    build()?;
    assert_eq!(
        worker_compiled_at(&project.join("build"), "main")?,
        compiled
    );

    fs::write(worker.join("settings.json"), r#"{"feature":false}"#)?;
    build()?;
    assert_ne!(
        worker_compiled_at(&project.join("build"), "main")?,
        compiled
    );
    assert_texts(&[(
        project.join("build/windows/demo-app/app/bundle/settings.json"),
        r#"{"feature":false}"#,
    )])?;
    Ok(())
}

#[cfg(unix)]
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
    assert_texts(&[(plugin.join("class"), "TokamakLocationPlugin")])?;
    assert!(
        fs::read_to_string(plugin.join("plist"))?.contains("<key>NSLocationUsageDescription</key>")
    );
    assert_texts(&[(plugin.join("compilation-conditions/0"), "LOCATION_ALWAYS")])?;
    assert_file(&plugin.join("native/LocationPlugin.swift"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn builds_the_location_plugin_s_declarations_from_its_settings() -> TestResult {
    let (_temporary, project, platform_pack) = create_inputs("ios-arm64")?;
    install_location_plugin(&project)?;
    let plugin = project.join("build/ios/demo-app.app/plugins/location");

    build_command("ios", &project, &platform_pack)?
        .assert()
        .success();
    assert_file(&plugin.join("plist"));
    assert_missing(&plugin.join("compilation-conditions"));

    build_command("ios", &project, &platform_pack)?
        .args([
            "--location-background-always",
            "--location-precision",
            "approximate",
        ])
        .args([
            "--location-accuracy-purposes",
            "Directions=Shows directions",
        ])
        .assert()
        .success();

    let plist = fs::read_to_string(plugin.join("plist"))?;
    for (key, declared) in [
        ("NSLocationWhenInUseUsageDescription", true),
        ("NSLocationAlwaysAndWhenInUseUsageDescription", true),
        ("NSLocationDefaultAccuracyReduced", true),
        ("<key>Directions</key>", true),
        ("UIBackgroundModes", false),
    ] {
        assert_eq!(plist.contains(key), declared, "{key} in {plist}");
    }
    assert_texts(&[(plugin.join("compilation-conditions/0"), "LOCATION_ALWAYS")])?;
    build_command("ios", &project, &platform_pack)?
        .args(["--location-always-description", "Records runs"])
        .assert()
        .failure()
        .stderr(contains("always-description needs background-always"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn stages_the_location_plugin_s_generated_android_manifest() -> TestResult {
    let (_temporary, project, platform_pack) = create_inputs("android-arm64")?;
    install_location_plugin(&project)?;

    build_command("android", &project, &platform_pack)?
        .arg("--location-background-updates")
        .assert()
        .success();

    let plugin = project.join("build/android/demo-app.apk/plugins/location");
    let manifest = fs::read_to_string(plugin.join("manifest"))?;
    for (declaration, declared) in [
        ("android.permission.FOREGROUND_SERVICE_LOCATION", true),
        ("com.tokamak.plugins.location.TokamakLocationService", true),
        (
            "com.tokamak.plugins.location.TokamakLocationBootReceiver",
            true,
        ),
        ("ACCESS_BACKGROUND_LOCATION", false),
    ] {
        assert_eq!(
            manifest.contains(declaration),
            declared,
            "{declaration} in {manifest}"
        );
    }
    Ok(())
}

/// Install the `alerts` plugin, whose build script adds the files in its
/// `sounds` setting's directory to its macOS resources.
#[cfg(unix)]
fn install_plugin_with_settings(root: &Path) -> TestResult {
    fs::write(
        root.join("package.json"),
        r#"{"name":"demo-app","scripts":{"build":"echo already-built"},"dependencies":{"alerts":"1.0.0"}}"#,
    )?;
    let plugin = root.join("node_modules/alerts");
    fs::create_dir_all(&plugin)?;
    fs::write(
        plugin.join("tokamak-plugin.json"),
        serde_json::json!({
            "schemaVersion": 1,
            "id": "alerts",
            "settings": { "sounds": { "kind": "path", "description": "Sound files" } },
            "build": "build.mjs",
            "platforms": { "macos": { "class": "Alerts" } },
        })
        .to_string(),
    )?;
    fs::write(
        plugin.join("build.mjs"),
        r#"
        import { readdirSync, readFileSync, writeFileSync } from "node:fs";
        const { settings, output } = JSON.parse(readFileSync(0, "utf8"));
        const resources = readdirSync(settings.sounds).map((name) => `${settings.sounds}/${name}`);
        writeFileSync(`${output}/Generated.caf`, "generated");
        resources.push(`${output}/Generated.caf`);
        console.error(`staged ${resources.length} resources`);
        console.log(JSON.stringify({ resources }));
        "#,
    )?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn stages_what_a_plugin_s_build_script_generates_and_prints_its_notices() -> TestResult {
    let (_temporary, project, platform_pack) = create_inputs("macos-arm64")?;
    install_plugin_with_settings(&project)?;
    fs::create_dir_all(project.join("sounds"))?;
    fs::write(project.join("sounds/Chime.caf"), "chime")?;

    build_command("macos", &project, &platform_pack)?
        .arg("--alerts-macos-sounds")
        .arg(project.join("sounds"))
        .assert()
        .success()
        .stderr(contains("warning: plugin 'alerts': staged 2 resources\n"));

    let plugin = project.join("build/macos/demo-app.app/plugins/alerts");
    assert_eq!(
        fs::read_to_string(plugin.join("resources/0-Chime.caf"))?,
        "chime"
    );
    assert_eq!(
        fs::read_to_string(plugin.join("resources/1-Generated.caf"))?,
        "generated"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn lists_plugin_settings_in_help_and_rejects_undeclared_ones() -> TestResult {
    let (_temporary, project, platform_pack) = create_inputs("macos-arm64")?;
    install_plugin_with_settings(&project)?;

    Command::cargo_bin("tok")?
        .args(["build", "macos", "--help", "--project"])
        .arg(&project)
        .assert()
        .success()
        .stdout(contains("--alerts-sounds <PATH>").and(contains("Sound files")));
    build_command("macos", &project, &platform_pack)?
        .args(["--alerts-volume", "3"])
        .assert()
        .failure()
        .stderr(contains("unexpected argument '--alerts-volume' found"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn passes_typed_settings_with_their_defaults_to_build_scripts() -> TestResult {
    let (_temporary, project, platform_pack) = create_inputs("macos-arm64")?;
    install_plugin_with_settings(&project)?;
    let plugin = project.join("node_modules/alerts");
    fs::write(
        plugin.join("tokamak-plugin.json"),
        serde_json::json!({
            "schemaVersion": 1,
            "id": "alerts",
            "settings": {
                "loud": { "kind": "boolean", "description": "Loud", "default": false },
                "names": { "kind": "list", "of": "string", "description": "Names" },
                "tone": {
                    "kind": "enum",
                    "values": ["soft", "sharp"],
                    "description": "Tone",
                    "default": "soft",
                },
            },
            "build": "build.mjs",
            "platforms": { "macos": { "class": "Alerts" } },
        })
        .to_string(),
    )?;
    fs::write(
        plugin.join("build.mjs"),
        r#"
        import { readFileSync } from "node:fs";
        console.error(JSON.stringify(JSON.parse(readFileSync(0, "utf8")).settings));
        process.exit(1);
        "#,
    )?;

    build_command("macos", &project, &platform_pack)?
        .args(["--alerts-loud", "--alerts-names", "a,b"])
        .env("TOKAMAK_ALERTS_MACOS_NAMES", "c")
        .assert()
        .failure()
        .stderr(contains(r#"{"loud":true,"names":["a","b"],"tone":"soft"}"#));
    Ok(())
}

#[cfg(windows)]
#[test]
fn builds_windows_app_with_its_runtime_files() -> TestResult {
    let (_temporary, project, manifest) = create_windows_inputs()?;
    build_command("windows", &project, &manifest)?
        .assert()
        .success()
        .stdout(contains("Built Windows bundle").and(contains(r"\\?\").not()));

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
fn links_only_the_runtime_parts_the_windows_app_declares() -> TestResult {
    let platform_pack = PathBuf::from(std::env::var("TOKAMAK_TEST_WINDOWS_PACK")?);
    let temporary = tempfile::tempdir()?;
    let project = temporary.path().join("project");
    fs::create_dir_all(&project)?;
    create_project(&project)?;
    let executable = project.join("build/windows/demo-app/demo-app.exe");
    let build = || -> TestResult {
        build_command("windows", &project, &platform_pack)?
            .assert()
            .success()
            .stdout(contains("warning LNK").not())
            .stderr(contains("warning LNK").not());
        println!(
            "{}: {} bytes",
            executable.display(),
            fs::metadata(&executable)?.len()
        );
        Ok(())
    };
    // Whether the executable exports `entry_point` and contains `code`, a
    // string only that part's code holds.
    let linked = |entry_point: &str, code: &str| -> TestResult<(bool, bool)> {
        let exports = ProcessCommand::new("dumpbin")
            .args(["/NOLOGO", "/EXPORTS"])
            .arg(&executable)
            .output()?;
        if !exports.status.success() {
            return Err(format!("dumpbin failed with {}", exports.status).into());
        }
        let contents = fs::read(&executable)?;
        Ok((
            String::from_utf8_lossy(&exports.stdout).contains(entry_point),
            contents
                .windows(code.len())
                .any(|window| window == code.as_bytes()),
        ))
    };
    // SQLite writes this header into every database.
    let storage = || linked(tokamak::STORAGE_ENTRY_POINT, "SQLite format 3");
    let development = || linked(tokamak::DEVELOPMENT_ENTRY_POINT, tokamak::DEV_SOCKET_HEADER);

    build()?;
    assert_eq!(storage()?, (false, false));
    assert_eq!(development()?, (false, false));
    declare_storage(&project)?;
    build()?;
    assert_eq!(storage()?, (true, true));
    assert_eq!(development()?, (false, false));
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
    fs::write(project.join("AppIcon.ico"), "ico")?;

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
    let target = Platform::Macos.default_target()?.to_string();
    let pack = home
        .join(".local/share/tokamak/platform-packs")
        .join(&target);
    fs::create_dir_all(&project)?;
    fs::create_dir_all(&pack)?;
    create_project(&project)?;
    create_platform_pack(&pack, &target)?;

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
    let target = Platform::Macos.default_target()?.to_string();
    let pack = platform_packs.join(&target);
    fs::create_dir_all(&project)?;
    fs::create_dir_all(&empty)?;
    fs::create_dir_all(&pack)?;
    create_project(&project)?;
    create_platform_pack(&pack, &target)?;

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

#[cfg(unix)]
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
/// reports the device and project it builds for and the entry points the app
/// exports, then fails.
#[cfg(target_os = "macos")]
fn dev_command(temporary: &Path, platform_pack: &Path, project: &Path) -> TestResult<Command> {
    use std::ffi::OsString;

    fs::write(
        platform_pack.join("build/entrypoint"),
        "echo \"building for $(cat \"$2/metadata/device-id\") from $(cat \"$2/metadata/project-dir\"), exporting [$(cat \"$2/metadata/exported-symbols\")]\" >&2\nexit 1\n",
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
  require("node:fs").writeFileSync(`${report}.tmp`, JSON.stringify({ url, workerName: "demo-app", socketPort: 1 }));
  require("node:fs").renameSync(`${report}.tmp`, report);
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
fn dev_links_the_development_part() -> TestResult {
    let (temporary, project, platform_pack) = create_inputs("ios-arm64")?;
    dev_command(temporary.path(), &platform_pack, &project)?
        .assert()
        .failure()
        .stderr(contains(format!(
            "exporting [{}]",
            tokamak::DEVELOPMENT_ENTRY_POINT
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

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
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
