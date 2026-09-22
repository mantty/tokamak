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

#[cfg(not(target_os = "macos"))]
#[test]
fn cert_inventory_requires_a_macos_host() -> TestResult {
    Command::cargo_bin("tok")?
        .arg("certs")
        .assert()
        .failure()
        .stderr(contains("iOS signing discovery requires a macOS host"));
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
        cmd.args([command, target, "--set", "ios-team-id=TEAM", "--project"])
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
