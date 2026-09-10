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
fn accepts_ios_team_selection_for_dev_and_build() -> TestResult {
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

#[cfg(target_os = "macos")]
#[test]
fn dev_explains_conflicting_automatic_and_manual_signing() -> TestResult {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let project = tempfile::tempdir()?;
    fs::write(
        project.path().join("wrangler.json"),
        r#"{"name":"demo-app","main":"worker.js"}"#,
    )?;
    fs::write(project.path().join("worker.js"), "export default {};")?;
    let bin = project.path().join("bin");
    fs::create_dir(&bin)?;
    for (tool, script) in [
        ("adb", "#!/bin/sh\nexit 0\n"),
        ("emulator", "#!/bin/sh\nexit 0\n"),
        (
            "xcrun",
            "#!/bin/sh\ncase \"$1\" in\n  simctl) echo '{\"devices\":{}}' ;;\n  devicectl) echo '{\"devices\":[{\"identifier\":\"DEVICE\",\"platform\":\"iOS\",\"name\":\"Test iPhone\"}]}' ;;\n  *) exit 1 ;;\nesac\n",
        ),
    ] {
        fs::write(bin.join(tool), script)?;
        fs::set_permissions(bin.join(tool), fs::Permissions::from_mode(0o755))?;
    }
    for team_flag in [false, true] {
        let mut cmd = Command::cargo_bin("tok")?;
        cmd.args(["dev", "DEVICE", "--project"])
            .arg(project.path())
            .arg("--config")
            .arg(project.path())
            .env(
                "PATH",
                std::env::join_paths([
                    bin.as_path(),
                    std::path::Path::new("/usr/bin"),
                    std::path::Path::new("/bin"),
                ])?,
            )
            .env_remove("TOKAMAK_IOS_TEAM_ID")
            .env("TOKAMAK_IOS_SIGNING_IDENTITY", "IDENTITY_SHA1")
            .env(
                "TOKAMAK_IOS_PROVISIONING_PROFILE",
                project.path().join("manual.mobileprovision"),
            );
        if team_flag {
            cmd.args(["--ios-team-id", "TEAM"]);
        } else {
            cmd.env("TOKAMAK_IOS_TEAM_ID", "TEAM");
        }
        cmd.args(["--", "false"]).assert().failure().stderr(concat!(
            "error: automatic and manual iOS signing cannot be combined.\n\n",
            "Choose ONE signing mode:\n",
            "  Automatic: --ios-team-id or TOKAMAK_IOS_TEAM_ID\n",
            "  Manual:    both TOKAMAK_IOS_SIGNING_IDENTITY and\n",
            "             TOKAMAK_IOS_PROVISIONING_PROFILE\n"
        ));
    }
    Ok(())
}
