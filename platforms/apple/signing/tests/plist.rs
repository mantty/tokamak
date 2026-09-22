// The fake toolchain commands use POSIX shell scripts; Apple platform-pack CI runs on macOS.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use plist::Value;

fn write_executable(path: &Path, contents: &str) -> Result<()> {
    fs::write(path, contents)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

fn write_input(root: &Path, platform: &str) -> Result<std::path::PathBuf> {
    let input = root.join("input");
    let metadata = input.join("metadata");
    fs::create_dir_all(&metadata)?;
    for (name, value) in [
        ("app-name", "Demo App"),
        ("app-slug", "demo-app"),
        ("identifier", "com.example.demo"),
        ("host", "demo-app.tokamak.local"),
        ("platform", platform),
        (
            "project-dir",
            root.to_str().context("temporary path is UTF-8")?,
        ),
        ("version", "1.2.3"),
    ] {
        fs::write(metadata.join(name), value)?;
    }
    Ok(input)
}

fn value<'a>(plist: &'a Value, key: &str) -> Option<&'a str> {
    plist.as_dictionary()?.get(key).and_then(Value::as_string)
}

fn assert_values(plist: &Value, values: &[(&str, &str)]) {
    for (key, expected) in values {
        assert_eq!(
            value(plist, key),
            Some(*expected),
            "missing or incorrect {key}"
        );
    }
}

fn run_plist(
    input: &Path,
    output: &Path,
    tools: &Path,
    plist_variable: &str,
    user_plist: Option<&Path>,
) -> Result<()> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tokamak-apple-signing"));
    command
        .args([
            "plist",
            "--input",
            input.to_str().context("input path is UTF-8")?,
            "--output",
            output.to_str().context("output path is UTF-8")?,
        ])
        .env("PATH", tools)
        .env_remove(plist_variable)
        .env_remove("TOKAMAK_IOS_BUILD_NUMBER")
        .env_remove("TOKAMAK_MACOS_BUILD_NUMBER");
    if let Some(user_plist) = user_plist {
        command.env(plist_variable, user_plist);
    }
    let result = command.output()?;
    if !result.status.success() {
        bail!(
            "plist command failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    Ok(())
}

fn write_user_plist(path: &Path) -> Result<()> {
    let mut user = plist::Dictionary::new();
    for (key, value) in [
        ("DTPlatformName", "user-platform"),
        ("DTPlatformVersion", "user-version"),
        ("DTSDKName", "user-sdk"),
        ("DTSDKBuild", "user-sdk-build"),
        ("DTPlatformBuild", "user-platform-build"),
        ("DTXcode", "user-xcode"),
        ("DTXcodeBuild", "user-xcode-build"),
        ("UserValue", "preserved"),
    ] {
        user.insert(key.into(), Value::String(value.into()));
    }
    Value::Dictionary(user).to_file_xml(path)?;
    Ok(())
}

fn plist_variable(platform: &str) -> &'static str {
    if platform == "macos" {
        "TOKAMAK_MACOS_PLIST"
    } else {
        "TOKAMAK_IOS_PLIST"
    }
}

fn write_toolchain_commands(root: &Path) -> Result<std::path::PathBuf> {
    let tools = root.join("tools");
    fs::create_dir(&tools)?;
    let platform = tools.join("platform");
    fs::create_dir(&platform)?;
    let mut platform_version = plist::Dictionary::new();
    platform_version.insert(
        "ProductBuildVersion".into(),
        Value::String("PLATFORM23".into()),
    );
    Value::Dictionary(platform_version).to_file_xml(platform.join("version.plist"))?;
    write_executable(
        &tools.join("xcrun"),
        r#"#!/bin/sh
case "$1:$2:$3" in
  --sdk:iphoneos:--show-sdk-version|--sdk:iphonesimulator:--show-sdk-version|--sdk:macosx:--show-sdk-version)
    echo 26.5
    ;;
  --sdk:iphoneos:--show-sdk-build-version|--sdk:iphonesimulator:--show-sdk-build-version|--sdk:macosx:--show-sdk-build-version)
    echo 23F73
    ;;
  --sdk:iphoneos:--show-sdk-platform-path|--sdk:iphonesimulator:--show-sdk-platform-path|--sdk:macosx:--show-sdk-platform-path)
    printf '%s\n' "${0%/*}/platform"
    ;;
  *)
    exit 1
    ;;
esac
"#,
    )?;
    write_executable(
        &tools.join("xcodebuild"),
        "#!/bin/sh\n[ \"$#\" -eq 1 ] && [ \"$1\" = \"-version\" ] || exit 1\nprintf 'Xcode 26.5\\nBuild version 17F113\\n'\n",
    )?;
    Ok(tools)
}

fn test_platform(platform: &str, platform_name: &str) -> Result<()> {
    let temporary = tempfile::tempdir()?;
    let tools = write_toolchain_commands(temporary.path())?;
    let input = write_input(temporary.path(), platform)?;
    let generated_output = temporary.path().join("generated.plist");
    run_plist(
        &input,
        &generated_output,
        &tools,
        plist_variable(platform),
        None,
    )?;
    let generated = Value::from_file(&generated_output)?;
    let sdk_name = format!("{platform_name}26.5");
    assert_values(
        &generated,
        &[
            ("DTPlatformName", platform_name),
            ("DTPlatformVersion", "26.5"),
            ("DTSDKName", sdk_name.as_str()),
            ("DTSDKBuild", "23F73"),
            ("DTPlatformBuild", "PLATFORM23"),
            ("DTXcode", "2650"),
            ("DTXcodeBuild", "17F113"),
        ],
    );

    let user_plist = temporary.path().join("user.plist");
    write_user_plist(&user_plist)?;
    let overlay_output = temporary.path().join("overlay.plist");
    run_plist(
        &input,
        &overlay_output,
        &tools,
        plist_variable(platform),
        Some(&user_plist),
    )?;
    let overlay = Value::from_file(&overlay_output)?;
    assert_values(
        &overlay,
        &[
            ("DTPlatformName", "user-platform"),
            ("DTPlatformVersion", "user-version"),
            ("DTSDKName", "user-sdk"),
            ("DTSDKBuild", "user-sdk-build"),
            ("DTPlatformBuild", "user-platform-build"),
            ("DTXcode", "user-xcode"),
            ("DTXcodeBuild", "user-xcode-build"),
            ("UserValue", "preserved"),
        ],
    );
    Ok(())
}

#[test]
fn generates_ios_metadata_and_applies_user_overlay() -> Result<()> {
    test_platform("ios", "iphoneos")
}

#[test]
fn generates_ios_simulator_metadata_and_applies_user_overlay() -> Result<()> {
    test_platform("ios-simulator", "iphonesimulator")
}

#[test]
fn generates_macos_metadata_and_applies_user_overlay() -> Result<()> {
    test_platform("macos", "macosx")
}
