use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[test]
fn scaffolds_plugin_names_that_are_javascript_keywords() -> TestResult {
    let root = tempfile::tempdir()?;
    let path = root.path().join("class");
    Command::cargo_bin("tok")?
        .args(["plugin", "new"])
        .arg(&path)
        .assert()
        .success();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.join("tokamak-plugin.json"))?)?;
    assert_eq!(manifest["api"]["export"], "createClass");
    assert!(
        std::fs::read_to_string(path.join("src/index.ts"))?
            .contains("export function createClass(")
    );
    Ok(())
}

#[test]
fn scaffolds_publishable_plugins_without_overwriting_existing_projects() -> TestResult {
    let root = tempfile::tempdir()?;
    for (name, platforms, families) in [
        ("page-tools", None, &[][..]),
        ("apple-tools", Some("ios,macos"), &["apple"][..]),
        ("android-tools", Some("android"), &["android"][..]),
    ] {
        // Avoid reserved CLI option prefixes in the plugin's ID.
        let path = root.path().join(format!("example-{name}"));
        let mut command = Command::cargo_bin("tok")?;
        command.args(["plugin", "new"]).arg(&path);
        if let Some(platforms) = platforms {
            command.args(["--platforms", platforms]);
        }
        command.assert().success();
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path.join("tokamak-plugin.json"))?)?;
        let package: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path.join("package.json"))?)?;
        assert_eq!(package["type"], "module");
        assert_eq!(package["scripts"]["prepack"], "npm run check");
        assert_eq!(manifest["schemaVersion"], 2);
        for module in [&manifest["api"]["module"], &manifest["web"]["module"]] {
            assert!(
                path.join(module.as_str().ok_or("missing module")?)
                    .is_file()
            );
        }
        let native = !families.is_empty();
        assert_eq!(
            package["exports"]["./events"].as_str(),
            native.then_some("./src/events.ts"),
            "{package}"
        );
        assert_eq!(path.join("src/events.ts").is_file(), native);
        // Generated files keep LF, as `tok plugin check` compares them byte for byte.
        let attributes = std::fs::read_to_string(path.join(".gitattributes")).unwrap_or_default();
        assert_eq!(attributes.contains("*.generated.* text eol=lf"), native);
        assert_eq!(
            declared_platforms(&manifest)?,
            platforms.unwrap_or_default()
        );
        for family in families {
            assert_native_package(&path, &package, family)?;
        }
        let readme = path.join("README.md");
        std::fs::write(&readme, "author's work")?;
        Command::cargo_bin("tok")?
            .args(["plugin", "new"])
            .arg(&path)
            .assert()
            .failure();
        assert_eq!(std::fs::read_to_string(readme)?, "author's work");
    }
    Ok(())
}

#[test]
fn scaffolds_each_family_for_the_named_platforms() -> TestResult {
    let root = tempfile::tempdir()?;
    let path = root.path().join("phone-tools");
    Command::cargo_bin("tok")?
        .args(["plugin", "new", "--platforms", "ios,android"])
        .arg(&path)
        .assert()
        .success();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.join("tokamak-plugin.json"))?)?;
    let package: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.join("package.json"))?)?;
    assert_eq!(declared_platforms(&manifest)?, "android,ios");
    for family in ["apple", "android"] {
        assert_native_package(&path, &package, family)?;
    }
    // Each family's listeners register on the named platforms only.
    assert!(
        std::fs::read_to_string(path.join("src/apple.generated.ts"))?
            .contains(r#"defineEvent("phone-tools.apple.echoed", { platforms: ["ios"] })"#)
    );
    assert!(
        std::fs::read_to_string(path.join("src/index.ts"))?
            .contains("platformApi(ctx, { apple, android })")
    );
    let events = std::fs::read_to_string(path.join("src/events.ts"))?;
    assert!(events.contains("apple.onEchoed(listener);"), "{events}");
    assert!(events.contains("android.onEchoed(listener);"), "{events}");
    Ok(())
}

#[test]
fn rejects_platforms_without_plugin_native_code_before_creating_files() -> TestResult {
    let root = tempfile::tempdir()?;
    let path = root.path().join("desk-tools");
    for (platforms, reason) in [
        ("windows", "android, not windows"),
        ("ios-simulator", "android, not ios-simulator"),
        ("ios,linux", "unknown platform 'linux'"),
    ] {
        Command::cargo_bin("tok")?
            .args(["plugin", "new", "--platforms", platforms])
            .arg(&path)
            .assert()
            .failure()
            .stderr(contains(reason));
        assert!(!path.exists(), "{platforms}");
    }
    Ok(())
}

/// The platforms `manifest` declares, sorted and comma-separated.
fn declared_platforms(manifest: &serde_json::Value) -> TestResult<String> {
    let platforms = manifest["platforms"]
        .as_object()
        .ok_or("missing platforms")?;
    let mut names = platforms.keys().cloned().collect::<Vec<_>>();
    names.sort();
    Ok(names.join(","))
}

/// Asserts that the scaffolded package exports and generates `family`'s platform API, in `src/`,
/// and holds the family's native files, and only those, in the family's directory.
fn assert_native_package(
    path: &std::path::Path,
    package: &serde_json::Value,
    family: &str,
) -> TestResult {
    assert_eq!(
        package["exports"][format!("./{family}")],
        format!("./src/{family}.generated.ts")
    );
    assert!(path.join(format!("src/{family}.generated.ts")).is_file());
    assert_eq!(
        package["scripts"][format!("generate:{family}")],
        format!("tok plugin generate {family}")
    );
    let extension = if family == "apple" { "swift" } else { "kt" };
    let mut files = std::fs::read_dir(path.join(family))?
        .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
        .collect::<std::io::Result<Vec<_>>>()?;
    files.sort();
    assert_eq!(
        files,
        [
            format!("Exports.generated.{extension}"),
            format!("Exports.{extension}"),
            format!("Plugin.{extension}"),
        ]
    );
    Ok(())
}

/// What the native template's registration file describes.
#[cfg(unix)]
const ECHO: &str = include_str!("../templates/plugin/description.json");

/// A stand-in Apple pack whose `describe` writes `description`, with `dispatch` as the dispatch,
/// or exits with `status`.
#[cfg(unix)]
fn describing_pack(
    description: &str,
    dispatch: &str,
    status: i32,
) -> TestResult<tempfile::TempDir> {
    use std::collections::BTreeMap;

    use tokamak_cli::{MANIFEST_FILE, Platform, PlatformPackManifest, write_manifest};

    let packs = tempfile::tempdir()?;
    let target = Platform::Macos.default_target()?;
    let pack = packs.path().join(target.to_string());
    std::fs::create_dir_all(pack.join("build"))?;
    write_manifest(
        pack.join(MANIFEST_FILE),
        &PlatformPackManifest {
            tokamak_version: env!("CARGO_PKG_VERSION").to_owned(),
            target,
            variables: BTreeMap::new(),
            plugin_keys: BTreeMap::new(),
        },
    )?;
    std::fs::write(
        pack.join("build/entrypoint"),
        format!(
            "[ \"$1 $4 $5\" = 'describe ios macos' ] || exit 2\n[ {status} = 0 ] || exit {status}\nprintf '%s' '{description}' > \"$3/description.json\"\nprintf '%s' '{dispatch}' > \"$3/Exports.generated.swift\"\n"
        ),
    )?;
    Ok(packs)
}

/// A new Apple plugin, `device-tools`, in a temporary directory.
#[cfg(unix)]
fn scaffolded_plugin() -> TestResult<(tempfile::TempDir, std::path::PathBuf)> {
    let root = tempfile::tempdir()?;
    let plugin = root.path().join("device-tools");
    Command::cargo_bin("tok")?
        .args(["plugin", "new", "--platforms", "ios,macos"])
        .arg(&plugin)
        .assert()
        .success();
    Ok((root, plugin))
}

/// `tok plugin` with `arguments` in `plugin`, finding packs in `packs`.
#[cfg(unix)]
fn tok_plugin(
    plugin: &std::path::Path,
    packs: &std::path::Path,
    arguments: &[&str],
) -> TestResult<Command> {
    let mut command = Command::cargo_bin("tok")?;
    command
        .arg("plugin")
        .args(arguments)
        .current_dir(plugin)
        .env("TOKAMAK_PLATFORM_PACK_PATH", packs);
    Ok(command)
}

#[cfg(unix)]
#[test]
fn generates_and_checks_a_platform_api_with_the_family_pack() -> TestResult {
    let (_root, plugin) = scaffolded_plugin()?;
    let scaffolded = std::fs::read_to_string(plugin.join("src/apple.generated.ts"))?;
    let packs = describing_pack(ECHO, "dispatch", 0)?;
    let tok = |arguments: &[&str]| tok_plugin(&plugin, packs.path(), arguments);

    tok(&["check"])?.assert().failure().stderr(
        contains("`tok plugin generate apple` changes apple/Exports.generated.swift\n")
            .and(contains("apple.generated.ts").not()),
    );
    tok(&["generate", "apple"])?
        .assert()
        .success()
        .stdout(contains(
            "Generated apple/Exports.generated.swift, src/apple.generated.ts",
        ));
    assert_eq!(
        std::fs::read_to_string(plugin.join("src/apple.generated.ts"))?,
        scaffolded
    );
    assert_eq!(
        std::fs::read_to_string(plugin.join("apple/Exports.generated.swift"))?,
        "dispatch"
    );
    tok(&["check"])?.assert().success();
    std::fs::write(plugin.join("src/apple.generated.ts"), "edited")?;
    tok(&["check"])?.assert().failure().stderr(contains(
        "`tok plugin generate apple` changes src/apple.generated.ts\n",
    ));
    // The API's directory need not exist, as the manifest's API module may be elsewhere.
    std::fs::remove_dir_all(plugin.join("src"))?;
    tok(&["generate", "apple"])?.assert().success();
    assert_eq!(
        std::fs::read_to_string(plugin.join("src/apple.generated.ts"))?,
        scaffolded
    );
    tok(&["generate", "android"])?
        .assert()
        .failure()
        .stderr(contains("tokamak-plugin.json has no android platform"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn reports_why_the_family_pack_cannot_generate() -> TestResult {
    let (_root, plugin) = scaffolded_plugin()?;

    let unavailable = describing_pack(ECHO, "dispatch", 69)?;
    tok_plugin(&plugin, unavailable.path(), &["check"])?
        .assert()
        .success()
        .stderr(contains(
            "apple files are not checked: this host cannot describe apple plugins",
        ));
    tok_plugin(&plugin, unavailable.path(), &["generate", "apple"])?
        .assert()
        .failure()
        .stderr(contains("this host cannot describe apple plugins"));

    // An installed pack that cannot load fails the check rather than skipping it.
    let incompatible = describing_pack(ECHO, "dispatch", 0)?;
    let manifest = incompatible
        .path()
        .join(tokamak_cli::Platform::Macos.default_target()?.to_string())
        .join(tokamak_cli::MANIFEST_FILE);
    let content = std::fs::read_to_string(&manifest)?;
    std::fs::write(
        &manifest,
        content.replace(env!("CARGO_PKG_VERSION"), "0.0.0"),
    )?;
    tok_plugin(&plugin, incompatible.path(), &["check"])?
        .assert()
        .failure()
        .stderr(contains("incompatible platform pack"));

    let failing = describing_pack(ECHO, "dispatch", 1)?;
    tok_plugin(&plugin, failing.path(), &["check"])?
        .assert()
        .failure()
        .stderr(contains("platform-pack describe entrypoint failed"));

    let promise = describing_pack(
        r#"{"methods":[{"name":"then","kind":"call","returns":"void"}]}"#,
        "dispatch",
        0,
    )?;
    tok_plugin(&plugin, promise.path(), &["check"])?
        .assert()
        .failure()
        .stderr(contains(
            "apple/Exports.swift cannot be called from TypeScript",
        ));

    // A registration file's family is one the plugin builds for.
    let packs = describing_pack(ECHO, "dispatch", 0)?;
    std::fs::create_dir_all(plugin.join("android"))?;
    std::fs::write(plugin.join("android/Exports.kt"), "")?;
    tok_plugin(&plugin, packs.path(), &["check"])?
        .assert()
        .failure()
        .stderr(contains("tokamak-plugin.json has no android platform"));
    Ok(())
}

#[test]
fn rejects_invalid_plugin_names_before_creating_files() -> TestResult {
    let root = tempfile::tempdir()?;
    for name in [
        "BadName",
        "double--hyphen",
        "tokamak",
        "tokamak-device",
        "ios-helper",
    ] {
        let path = root.path().join(name);
        Command::cargo_bin("tok")?
            .args(["plugin", "new"])
            .arg(&path)
            .assert()
            .failure();
        assert!(!path.exists());
    }
    Ok(())
}

#[test]
fn explains_the_cert_inventory_command() -> TestResult {
    Command::cargo_bin("tok")?
        .args(["certs", "--help"])
        .assert()
        .success()
        .stdout(
            contains("signing identities and provisioning profiles")
                .and(contains("Usage: tok certs")),
        );
    Ok(())
}

#[cfg(unix)]
#[test]
fn lists_signing_assets_with_the_ios_platform_pack() -> TestResult {
    use std::collections::BTreeMap;
    use std::fs;

    use tokamak_cli::{MANIFEST_FILE, PlatformPackManifest, Target, write_manifest};

    let packs = tempfile::tempdir()?;
    let pack = packs.path().join("ios-arm64");
    fs::create_dir_all(pack.join("build"))?;
    write_manifest(
        pack.join(MANIFEST_FILE),
        &PlatformPackManifest {
            tokamak_version: env!("CARGO_PKG_VERSION").to_owned(),
            target: Target::IosArm64,
            variables: BTreeMap::new(),
            plugin_keys: BTreeMap::new(),
        },
    )?;
    fs::write(
        pack.join("build/entrypoint"),
        "printf '%s from %s\\n' \"$*\" \"$PWD\"\n",
    )?;

    Command::cargo_bin("tok")?
        .arg("certs")
        .env("TOKAMAK_PLATFORM_PACK_PATH", packs.path())
        .assert()
        .success()
        .stdout(format!(
            "certs from {}\n",
            fs::canonicalize(&pack)?.display()
        ));
    Ok(())
}

#[test]
fn prints_its_version() -> TestResult {
    Command::cargo_bin("tok")?
        .arg("version")
        .assert()
        .success()
        .stdout(format!("tok {}\n", env!("CARGO_PKG_VERSION")));
    Ok(())
}

#[test]
fn lists_supported_targets() -> TestResult {
    let mut cmd = Command::cargo_bin("tok")?;

    cmd.arg("targets").assert().success().stdout(
        contains("android-arm64")
            .and(contains("ios-arm64"))
            .and(contains("ios-simulator-arm64"))
            .and(contains("ios-simulator-x64"))
            .and(contains("macos-arm64"))
            .and(contains("macos-x64"))
            .and(contains("windows-x64")),
    );

    Ok(())
}

#[test]
fn accepts_platform_pack_variables_for_dev_and_build() -> TestResult {
    let project = tempfile::tempdir()?;
    let missing_project = project.path().join("missing");
    for (command, target) in [("dev", "DEVICE"), ("build", "ios")] {
        let mut cmd = Command::cargo_bin("tok")?;
        cmd.args([command, target, "--ios-team-id", "TEAM", "--project"])
            .arg(&missing_project);
        if command == "dev" {
            cmd.args(["--", "pnpm", "dev"]);
        }
        cmd.assert()
            .failure()
            .stderr(contains("project directory does not exist"));
    }
    Ok(())
}
