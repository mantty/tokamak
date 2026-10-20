use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[test]
fn scaffolds_plugin_names_that_are_javascript_keywords() -> TestResult {
    let root = tempfile::tempdir()?;
    let path = root.path().join("class");
    Command::cargo_bin("tok")?
        .arg("plugin")
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
    for (name, native, platforms) in [
        ("page-tools", None, vec![]),
        ("apple-tools", Some("apple"), vec!["ios", "macos"]),
        ("android-tools", Some("android"), vec!["android"]),
    ] {
        // Avoid reserved CLI option prefixes in the plugin's ID.
        let path = root.path().join(format!("example-{name}"));
        let mut command = Command::cargo_bin("tok")?;
        command.arg("plugin").arg(&path);
        if let Some(native) = native {
            command.args(["--native", native]);
        }
        command.assert().success();
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path.join("tokamak-plugin.json"))?)?;
        let package: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path.join("package.json"))?)?;
        assert_eq!(package["type"], "module");
        assert_eq!(manifest["schemaVersion"], 2);
        for module in [&manifest["api"]["module"], &manifest["web"]["module"]] {
            assert!(path.join(module.as_str().ok_or("missing module")?).is_file());
        }
        assert_eq!(
            package["exports"].get("./events").is_some(),
            native.is_some(),
            "{package}"
        );
        let declared = manifest["platforms"]
            .as_object()
            .ok_or("missing platforms")?;
        assert_eq!(declared.len(), platforms.len());
        for platform in platforms {
            for source in declared[platform]["sources"]
                .as_array()
                .ok_or("missing sources")?
            {
                assert!(
                    path.join(source.as_str().ok_or("invalid source")?)
                        .is_file()
                );
            }
        }
        let readme = path.join("README.md");
        std::fs::write(&readme, "author's work")?;
        Command::cargo_bin("tok")?
            .arg("plugin")
            .arg(&path)
            .assert()
            .failure();
        assert_eq!(std::fs::read_to_string(readme)?, "author's work");
    }
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
            .arg("plugin")
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
