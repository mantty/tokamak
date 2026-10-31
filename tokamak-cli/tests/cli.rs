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
        Command::cargo_bin("tok")?
            .args(["plugin", "new"])
            .arg(&path)
            .args(
                platforms
                    .into_iter()
                    .flat_map(|platforms| ["--platforms", platforms]),
            )
            .assert()
            .success();
        assert_publishable_plugin(&path, platforms, families)?;
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

/// Asserts that the plugin scaffolded at `path` is a publishable package declaring `platforms`,
/// with the native packages of `families`.
fn assert_publishable_plugin(
    path: &std::path::Path,
    platforms: Option<&str>,
    families: &[&str],
) -> TestResult {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.join("tokamak-plugin.json"))?)?;
    let package: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.join("package.json"))?)?;
    assert_eq!(package["type"], "module");
    assert_eq!(package["scripts"]["prepack"], "npm run check");
    assert_eq!(manifest["schemaVersion"], 2);
    assert_plugin_modules_exist(path, &manifest)?;
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
        assert_native_package(path, &package, &manifest, family)?;
    }
    Ok(())
}

/// Asserts that the manifest's API and web modules exist in the plugin at `path`.
fn assert_plugin_modules_exist(path: &std::path::Path, manifest: &serde_json::Value) -> TestResult {
    for module in [&manifest["api"]["module"], &manifest["web"]["module"]] {
        assert!(
            path.join(module.as_str().ok_or("missing module")?)
                .is_file()
        );
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
        assert_native_package(&path, &package, &manifest, family)?;
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
/// and holds the family's native package, and only that, in the family's directory, whose
/// module the manifest's classes name.
fn assert_native_package(
    path: &std::path::Path,
    package: &serde_json::Value,
    manifest: &serde_json::Value,
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
    let words = path
        .file_name()
        .ok_or("no plugin name")?
        .to_string_lossy()
        .split('-')
        .map(|word| word[..1].to_uppercase() + &word[1..])
        .collect::<String>();
    let module = format!("Tokamak{words}");
    let (expected, platforms, class) = if family == "apple" {
        (
            vec![
                "Package.swift".to_owned(),
                format!("Sources/{module}/Exports.generated.swift"),
                format!("Sources/{module}/Exports.swift"),
                format!("Sources/{module}/Plugin.swift"),
            ],
            &["ios", "macos"][..],
            format!("{module}.{module}Plugin"),
        )
    } else {
        (
            vec![
                "build.gradle.kts".to_owned(),
                "gradle.properties".to_owned(),
                "settings.gradle.kts".to_owned(),
                "src/main/kotlin/Exports.generated.kt".to_owned(),
                "src/main/kotlin/Exports.kt".to_owned(),
                "src/main/kotlin/Plugin.kt".to_owned(),
            ],
            &["android"][..],
            format!("tokamak.plugins.{}.{module}Plugin", words.to_lowercase()),
        )
    };
    let directory = path.join(family);
    let mut files = walkdir::WalkDir::new(&directory)
        .into_iter()
        .filter(|entry| {
            entry
                .as_ref()
                .is_ok_and(|entry| entry.file_type().is_file())
        })
        .map(|entry| {
            let path = entry?.into_path();
            let relative = path.strip_prefix(&directory)?;
            Ok(relative.to_string_lossy().replace('\\', "/"))
        })
        .collect::<TestResult<Vec<_>>>()?;
    files.sort();
    assert_eq!(files, expected);
    for platform in platforms {
        if let Some(section) = manifest["platforms"].get(platform) {
            assert_eq!(section["class"], class.as_str(), "{platform}");
        }
    }
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
    use tokamak_cli::Platform;

    let packs = tempfile::tempdir()?;
    stand_in_pack(
        packs.path(),
        Platform::Macos.default_target()?,
        &format!(
            "[ \"$1 $5 $6\" = 'describe ios macos' ] || exit 2\n[ -f \"$3\" ] || exit 3\n[ {status} = 0 ] || exit {status}\nprintf '%s' '{description}' > \"$4/description.json\"\nprintf '%s' '{dispatch}' > \"$4/Exports.generated.swift\"\n"
        ),
    )?;
    Ok(packs)
}

/// A stand-in pack for `target` in `packs`, declaring no variables, whose
/// build entrypoint is the shell script `entrypoint`.
#[cfg(unix)]
fn stand_in_pack(
    packs: &std::path::Path,
    target: tokamak_cli::Target,
    entrypoint: &str,
) -> TestResult<std::path::PathBuf> {
    use std::collections::BTreeMap;

    use tokamak_cli::{MANIFEST_FILE, PlatformPackManifest, write_manifest};

    let pack = packs.join(target.to_string());
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
    std::fs::write(pack.join("build/entrypoint"), entrypoint)?;
    Ok(pack)
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
        contains(
            "`tok plugin generate apple` changes apple/Sources/TokamakDeviceTools/Exports.generated.swift\n",
        )
            .and(contains("apple.generated.ts").not()),
    );
    tok(&["generate", "apple"])?
        .assert()
        .success()
        .stdout(contains(
            "Generated apple/Sources/TokamakDeviceTools/Exports.generated.swift, src/apple.generated.ts",
        ));
    assert_eq!(
        std::fs::read_to_string(plugin.join("src/apple.generated.ts"))?,
        scaffolded
    );
    assert_eq!(
        std::fs::read_to_string(
            plugin.join("apple/Sources/TokamakDeviceTools/Exports.generated.swift")
        )?,
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
            "apple/Sources/TokamakDeviceTools/Exports.swift cannot be called from TypeScript",
        ));

    // The family's directory holds one registration file outside its build outputs.
    let packs = describing_pack(ECHO, "dispatch", 0)?;
    for copy in [".build/checkouts/other", "build/other", "Sources/Other"] {
        std::fs::create_dir_all(plugin.join("apple").join(copy))?;
        std::fs::write(plugin.join("apple").join(copy).join("Exports.swift"), "")?;
    }
    tok_plugin(&plugin, packs.path(), &["check"])?
        .assert()
        .failure()
        .stderr(contains(
            "apple has more than one Exports.swift: apple/Sources/Other/Exports.swift, apple/Sources/TokamakDeviceTools/Exports.swift\n",
        ));
    std::fs::remove_dir_all(plugin.join("apple/Sources"))?;
    tok_plugin(&plugin, packs.path(), &["generate", "apple"])?
        .assert()
        .failure()
        .stderr(contains("apple has no Exports.swift"));

    // A registration file's family is one the plugin builds for.
    std::fs::create_dir_all(plugin.join("android/src/main/kotlin"))?;
    std::fs::write(plugin.join("android/src/main/kotlin/Exports.kt"), "")?;
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
    use std::fs;

    let packs = tempfile::tempdir()?;
    let pack = stand_in_pack(
        packs.path(),
        tokamak_cli::Target::IosArm64,
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
