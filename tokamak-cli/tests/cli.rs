use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

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
