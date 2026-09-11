use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use plist::{Dictionary, Value};

const IOS_PLIST_ENV: &str = "TOKAMAK_IOS_PLIST";
const MACOS_PLIST_ENV: &str = "TOKAMAK_MACOS_PLIST";
const IOS_BUILD_NUMBER_ENV: &str = "TOKAMAK_IOS_BUILD_NUMBER";
const MACOS_BUILD_NUMBER_ENV: &str = "TOKAMAK_MACOS_BUILD_NUMBER";

/// Build the final Apple application information property list.
///
/// # Errors
///
/// Returns an error when Tokamak metadata or an optional user plist cannot be
/// read, when the plist contents are invalid, or when the final plist cannot
/// be written.
pub fn write_info_plist(input: &Path, output: &Path, icon_info_plist: Option<&Path>) -> Result<()> {
    let metadata = Metadata::read(input)?;
    let toolchain = ToolchainMetadata::detect(&metadata.platform)?;
    let user_plist = configured_user_plist(&metadata)?;
    let plist = build_info_plist(
        input,
        &metadata,
        &toolchain,
        user_plist.as_deref(),
        icon_info_plist,
    )?;
    Value::Dictionary(plist)
        .to_file_xml(output)
        .with_context(|| format!("write Apple application plist {}", output.display()))
}

struct Metadata {
    app_name: String,
    app_slug: String,
    identifier: String,
    host: String,
    platform: String,
    project_dir: PathBuf,
    version: String,
    build_number: String,
    dev_endpoint: Option<String>,
    dev_session_token: Option<String>,
}

impl Metadata {
    fn read(input: &Path) -> Result<Self> {
        let metadata = input.join("metadata");
        let version = read_optional(&metadata.join("version"))?;
        let mut build_number = version.clone().unwrap_or_else(|| "1".into());
        let version = version.unwrap_or_else(|| "1.0".into());
        let platform = read_required(&metadata.join("platform"))?;
        if let Some(environment) = build_number_environment(&platform)
            && let Some(value) = environment_value(environment)?
        {
            build_number = value;
        }
        validate_build_number(&build_number)?;

        let dev_endpoint = read_optional(&metadata.join("dev-endpoint"))?;
        let dev_session_token = read_optional(&metadata.join("dev-session-token"))?;
        if dev_endpoint.is_some() != dev_session_token.is_some() {
            bail!("development endpoint and session token must be provided together");
        }

        Ok(Self {
            app_name: read_required(&metadata.join("app-name"))?,
            app_slug: read_required(&metadata.join("app-slug"))?,
            identifier: read_required(&metadata.join("identifier"))?,
            host: read_required(&metadata.join("host"))?,
            platform,
            project_dir: PathBuf::from(read_required(&metadata.join("project-dir"))?),
            version,
            build_number,
            dev_endpoint,
            dev_session_token,
        })
    }
}

struct ToolchainMetadata {
    platform_name: &'static str,
    platform_version: String,
    sdk_build: String,
    platform_build: String,
    xcode: String,
    xcode_build: String,
}

impl ToolchainMetadata {
    fn detect(platform: &str) -> Result<Self> {
        let platform_name = match platform {
            "ios" => "iphoneos",
            "ios-simulator" => "iphonesimulator",
            "macos" => "macosx",
            platform => bail!("unsupported Apple platform: {platform}"),
        };
        let platform_version =
            command_output("xcrun", &["--sdk", platform_name, "--show-sdk-version"])?;
        let sdk_build = command_output(
            "xcrun",
            &["--sdk", platform_name, "--show-sdk-build-version"],
        )?;
        let platform_path = command_output(
            "xcrun",
            &["--sdk", platform_name, "--show-sdk-platform-path"],
        )?;
        let platform_build = read_platform_build(Path::new(&platform_path))?;
        let (xcode, xcode_build) =
            parse_xcode_version(&command_output("xcodebuild", &["-version"])?)?;

        Ok(Self {
            platform_name,
            platform_version,
            sdk_build,
            platform_build,
            xcode,
            xcode_build,
        })
    }
}

fn command_output(program: &str, arguments: &[&str]) -> Result<String> {
    let command = format!("{program} {}", arguments.join(" "));
    let output = Command::new(program)
        .args(arguments)
        .output()
        .with_context(|| format!("run {command}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        let detail = detail.trim();
        if detail.is_empty() {
            bail!("{command} failed with status {}", output.status);
        }
        bail!("{command} failed: {detail}");
    }
    let value = String::from_utf8(output.stdout)
        .with_context(|| format!("read output from {command}"))?
        .trim()
        .to_owned();
    if value.is_empty() {
        bail!("{command} returned no output");
    }
    Ok(value)
}

fn read_platform_build(platform_path: &Path) -> Result<String> {
    let path = platform_path.join("version.plist");
    let value = Value::from_file(&path)
        .with_context(|| format!("read Apple platform plist {}", path.display()))?;
    let dictionary = value.into_dictionary().with_context(|| {
        format!(
            "Apple platform plist root must be a dictionary: {}",
            path.display()
        )
    })?;
    dictionary
        .get("ProductBuildVersion")
        .and_then(Value::as_string)
        .map(str::to_owned)
        .with_context(|| {
            format!(
                "Apple platform plist has no ProductBuildVersion: {}",
                path.display()
            )
        })
}

fn parse_xcode_version(output: &str) -> Result<(String, String)> {
    let mut lines = output.lines();
    let version = lines
        .next()
        .and_then(|line| line.strip_prefix("Xcode "))
        .context("xcodebuild did not report an Xcode version")?;
    let build = lines
        .next()
        .and_then(|line| line.strip_prefix("Build version "))
        .context("xcodebuild did not report an Xcode build")?;
    let mut components = version.split('.');
    let major = components
        .next()
        .context("Xcode version is missing its major number")?
        .parse::<u32>()
        .context("Xcode version has an invalid major number")?;
    let minor = components
        .next()
        .unwrap_or("0")
        .parse::<u32>()
        .context("Xcode version has an invalid minor number")?;
    Ok((format!("{major}{minor}0"), build.into()))
}

fn build_info_plist(
    input: &Path,
    metadata: &Metadata,
    toolchain: &ToolchainMetadata,
    user_plist: Option<&Path>,
    icon_info_plist: Option<&Path>,
) -> Result<Dictionary> {
    let mut result = Dictionary::new();
    add_generated_plist(&mut result, metadata, toolchain)?;

    if let Some(path) = icon_info_plist {
        overlay_dictionary(&mut result, read_dictionary(path)?);
    }
    add_plugin_plist(&mut result, input)?;
    if let Some(path) = user_plist {
        overlay_dictionary(&mut result, read_dictionary(path)?);
    }
    Ok(result)
}

fn add_generated_plist(
    plist: &mut Dictionary,
    metadata: &Metadata,
    toolchain: &ToolchainMetadata,
) -> Result<()> {
    insert_string(plist, "CFBundleIdentifier", &metadata.identifier);
    insert_string(plist, "CFBundleName", &metadata.app_name);
    insert_string(plist, "CFBundleDisplayName", &metadata.app_name);
    insert_string(plist, "CFBundleExecutable", &metadata.app_slug);
    insert_string(plist, "CFBundlePackageType", "APPL");
    insert_string(plist, "CFBundleVersion", &metadata.build_number);
    insert_string(plist, "CFBundleShortVersionString", &metadata.version);
    insert_string(plist, "DTPlatformName", toolchain.platform_name);
    insert_string(plist, "DTPlatformVersion", &toolchain.platform_version);
    insert_string(
        plist,
        "DTSDKName",
        &format!("{}{}", toolchain.platform_name, toolchain.platform_version),
    );
    insert_string(plist, "DTSDKBuild", &toolchain.sdk_build);
    insert_string(plist, "DTPlatformBuild", &toolchain.platform_build);
    insert_string(plist, "DTXcode", &toolchain.xcode);
    insert_string(plist, "DTXcodeBuild", &toolchain.xcode_build);
    insert_string(plist, "TokamakHost", &metadata.host);
    if let Some(endpoint) = &metadata.dev_endpoint {
        insert_string(plist, "TokamakDevEndpoint", endpoint);
        insert_string(
            plist,
            "TokamakDevSessionToken",
            metadata
                .dev_session_token
                .as_deref()
                .context("development session token is missing")?,
        );
    }

    match metadata.platform.as_str() {
        "macos" => {
            insert_dictionary(
                plist,
                "NSAppTransportSecurity",
                [("NSAllowsLocalNetworking", Value::Boolean(true))],
            );
            plist.insert("NSHighResolutionCapable".into(), Value::Boolean(true));
        }
        "ios" | "ios-simulator" => {
            let supported_platform = if metadata.platform == "ios-simulator" {
                "iPhoneSimulator"
            } else {
                "iPhoneOS"
            };
            insert_array(
                plist,
                "CFBundleSupportedPlatforms",
                [Value::String(supported_platform.into())],
            );
            insert_dictionary(
                plist,
                "NSAppTransportSecurity",
                [("NSAllowsLocalNetworking", Value::Boolean(true))],
            );
            insert_string(plist, "MinimumOSVersion", "17.0");
            plist.insert("LSRequiresIPhoneOS".into(), Value::Boolean(true));
            insert_array(
                plist,
                "UIDeviceFamily",
                [Value::Integer(1.into()), Value::Integer(2.into())],
            );
            plist.insert(
                "UILaunchScreen".into(),
                Value::Dictionary(Dictionary::new()),
            );
            insert_array(
                plist,
                "UISupportedInterfaceOrientations",
                [
                    Value::String("UIInterfaceOrientationPortrait".into()),
                    Value::String("UIInterfaceOrientationLandscapeLeft".into()),
                    Value::String("UIInterfaceOrientationLandscapeRight".into()),
                ],
            );
        }
        platform => bail!("unsupported Apple platform: {platform}"),
    }
    Ok(())
}

fn add_plugin_plist(plist: &mut Dictionary, input: &Path) -> Result<()> {
    let plugins = input.join("plugins");
    if !plugins.is_dir() {
        return Ok(());
    }

    let mut plugin_directories = fs::read_dir(&plugins)
        .with_context(|| format!("read staged Apple plugins {}", plugins.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("read staged Apple plugins {}", plugins.display()))?;
    plugin_directories.retain(|path| path.is_dir());
    plugin_directories.sort();

    for plugin in plugin_directories {
        let plist_directory = plugin.join("plist");
        if !plist_directory.is_dir() {
            continue;
        }
        let mut entries = fs::read_dir(&plist_directory)
            .with_context(|| format!("read plugin plist {}", plist_directory.display()))?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()
            .with_context(|| format!("read plugin plist {}", plist_directory.display()))?;
        entries.retain(|path| path.is_dir());
        entries.sort();

        for entry in entries {
            let key = read_required(&entry.join("key"))?;
            let value = read_required(&entry.join("value"))?;
            insert_string(plist, &key, &value);
        }
    }
    Ok(())
}

fn configured_user_plist(metadata: &Metadata) -> Result<Option<PathBuf>> {
    let variable = match metadata.platform.as_str() {
        "ios" | "ios-simulator" => IOS_PLIST_ENV,
        "macos" => MACOS_PLIST_ENV,
        platform => bail!("unsupported Apple platform: {platform}"),
    };
    resolve_user_plist(&metadata.project_dir, variable, env::var_os(variable))
}

fn resolve_user_plist(
    project_dir: &Path,
    variable: &str,
    value: Option<OsString>,
) -> Result<Option<PathBuf>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let path = PathBuf::from(value);
    if path.as_os_str().is_empty() {
        bail!("{variable} must not be empty");
    }
    let path = if path.is_absolute() {
        path
    } else {
        project_dir.join(path)
    };
    if !path.is_file() {
        bail!("{variable} does not point to a file: {}", path.display());
    }
    Ok(Some(path))
}

fn read_dictionary(path: &Path) -> Result<Dictionary> {
    let value =
        Value::from_file(path).with_context(|| format!("read Apple plist {}", path.display()))?;
    value
        .into_dictionary()
        .ok_or_else(|| anyhow::anyhow!("Apple plist root must be a dictionary: {}", path.display()))
}

fn overlay_dictionary(destination: &mut Dictionary, source: Dictionary) {
    for (key, value) in source {
        match (destination.get_mut(&key), value) {
            (Some(Value::Dictionary(destination)), Value::Dictionary(source)) => {
                overlay_dictionary(destination, source);
            }
            (_, value) => {
                destination.insert(key, value);
            }
        }
    }
}

fn insert_string(plist: &mut Dictionary, key: &str, value: &str) {
    plist.insert(key.into(), Value::String(value.into()));
}

fn insert_dictionary<const N: usize>(
    plist: &mut Dictionary,
    key: &str,
    values: [(&str, Value); N],
) {
    let mut dictionary = Dictionary::new();
    for (key, value) in values {
        dictionary.insert(key.into(), value);
    }
    plist.insert(key.into(), Value::Dictionary(dictionary));
}

fn insert_array<const N: usize>(plist: &mut Dictionary, key: &str, values: [Value; N]) {
    plist.insert(key.into(), Value::Array(values.into()));
}

fn read_required(path: &Path) -> Result<String> {
    fs::read_to_string(path)
        .with_context(|| format!("read Apple build metadata {}", path.display()))
}

fn read_optional(path: &Path) -> Result<Option<String>> {
    if path.is_file() {
        Ok(Some(read_required(path)?))
    } else {
        Ok(None)
    }
}

fn environment_value(name: &str) -> Result<Option<String>> {
    match env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => bail!("{name} must contain valid UTF-8"),
    }
}

fn build_number_environment(platform: &str) -> Option<&'static str> {
    match platform {
        "ios" | "ios-simulator" => Some(IOS_BUILD_NUMBER_ENV),
        "macos" => Some(MACOS_BUILD_NUMBER_ENV),
        _ => None,
    }
}

fn validate_build_number(value: &str) -> Result<()> {
    let valid = !value.is_empty()
        && value.split('.').count() <= 3
        && value
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()));
    if valid {
        Ok(())
    } else {
        bail!("Apple build number must contain one to three period-separated integers: {value}");
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::{
        Metadata, ToolchainMetadata, build_info_plist, parse_xcode_version, resolve_user_plist,
        validate_build_number,
    };
    use anyhow::Context;
    use plist::{Dictionary, Value};

    fn input(root: &std::path::Path, platform: &str) -> anyhow::Result<std::path::PathBuf> {
        let input = root.join("input");
        let metadata = input.join("metadata");
        std::fs::create_dir_all(&metadata)?;
        std::fs::create_dir_all(input.join("plugins"))?;
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
            std::fs::write(metadata.join(name), value)?;
        }
        Ok(input)
    }

    fn toolchain(platform_name: &'static str) -> ToolchainMetadata {
        ToolchainMetadata {
            platform_name,
            platform_version: "26.5".into(),
            sdk_build: "23F73".into(),
            platform_build: "PLATFORM23".into(),
            xcode: "2650".into(),
            xcode_build: "17F113".into(),
        }
    }

    #[test]
    fn user_values_overlay_generated_values_and_preserve_other_typed_values() -> anyhow::Result<()>
    {
        let temporary = tempfile::tempdir()?;
        let input = input(temporary.path(), "ios")?;
        let user_path = temporary.path().join("User.plist");
        let mut transport = Dictionary::new();
        transport.insert("NSAllowsLocalNetworking".into(), Value::Boolean(false));
        transport.insert("NSAllowsArbitraryLoads".into(), Value::Boolean(true));
        let mut user = Dictionary::new();
        user.insert(
            "CFBundleIdentifier".into(),
            Value::String("com.user.override".into()),
        );
        user.insert(
            "UISupportedInterfaceOrientations".into(),
            Value::Array(vec![Value::String(
                "UIInterfaceOrientationPortraitUpsideDown".into(),
            )]),
        );
        user.insert(
            "NSAppTransportSecurity".into(),
            Value::Dictionary(transport),
        );
        user.insert("UserData".into(), Value::Data(vec![1, 2, 3]));
        user.insert(
            "UserDate".into(),
            Value::Date((SystemTime::UNIX_EPOCH + Duration::from_secs(1)).into()),
        );
        Value::Dictionary(user).to_file_binary(&user_path)?;

        let metadata = Metadata::read(&input)?;
        let result = build_info_plist(
            &input,
            &metadata,
            &toolchain("iphoneos"),
            Some(&user_path),
            None,
        )?;
        assert_eq!(
            result.get("CFBundleIdentifier").and_then(Value::as_string),
            Some("com.user.override")
        );
        assert_eq!(
            result.get("CFBundlePackageType").and_then(Value::as_string),
            Some("APPL")
        );
        let transport = result
            .get("NSAppTransportSecurity")
            .and_then(Value::as_dictionary)
            .context("transport dictionary")?;
        assert_eq!(
            transport
                .get("NSAllowsLocalNetworking")
                .and_then(Value::as_boolean),
            Some(false)
        );
        assert_eq!(
            transport
                .get("NSAllowsArbitraryLoads")
                .and_then(Value::as_boolean),
            Some(true)
        );
        assert_eq!(
            result.get("UserData").and_then(Value::as_data),
            Some(&[1, 2, 3][..])
        );
        assert!(result.get("UserDate").and_then(Value::as_date).is_some());
        assert_eq!(
            result
                .get("UISupportedInterfaceOrientations")
                .and_then(Value::as_array),
            Some(&vec![Value::String(
                "UIInterfaceOrientationPortraitUpsideDown".into(),
            )])
        );
        Ok(())
    }

    #[test]
    fn user_plist_takes_precedence_over_plugin_values() -> anyhow::Result<()> {
        let temporary = tempfile::tempdir()?;
        let input = input(temporary.path(), "ios")?;
        for (directory, key) in [("override", "PluginValue"), ("only", "PluginOnlyValue")] {
            let entry = input.join("plugins/example/plist").join(directory);
            std::fs::create_dir_all(&entry)?;
            std::fs::write(entry.join("key"), key)?;
            std::fs::write(entry.join("value"), "Plugin")?;
        }

        let user_path = temporary.path().join("User.plist");
        let mut user = Dictionary::new();
        user.insert("PluginValue".into(), Value::String("User".into()));
        Value::Dictionary(user).to_file_xml(&user_path)?;

        let metadata = Metadata::read(&input)?;
        let result = build_info_plist(
            &input,
            &metadata,
            &toolchain("iphoneos"),
            Some(&user_path),
            None,
        )?;
        assert_eq!(
            result.get("PluginValue").and_then(Value::as_string),
            Some("User")
        );
        assert_eq!(
            result.get("PluginOnlyValue").and_then(Value::as_string),
            Some("Plugin")
        );
        Ok(())
    }

    #[test]
    fn user_values_overlay_icon_values() -> anyhow::Result<()> {
        let temporary = tempfile::tempdir()?;
        let input = input(temporary.path(), "macos")?;
        let user_path = temporary.path().join("User.plist");
        let icon_path = temporary.path().join("icon-info.plist");
        let mut user = Dictionary::new();
        user.insert("CFBundleIconName".into(), Value::String("UserIcon".into()));
        Value::Dictionary(user).to_file_xml(&user_path)?;
        let mut icon = Dictionary::new();
        icon.insert("CFBundleIconName".into(), Value::String("AppIcon".into()));
        Value::Dictionary(icon).to_file_xml(&icon_path)?;

        let metadata = Metadata::read(&input)?;
        let result = build_info_plist(
            &input,
            &metadata,
            &toolchain("macosx"),
            Some(&user_path),
            Some(&icon_path),
        )?;
        assert_eq!(
            result.get("CFBundleIconName").and_then(Value::as_string),
            Some("UserIcon")
        );
        let result = build_info_plist(
            &input,
            &metadata,
            &toolchain("macosx"),
            None,
            Some(&icon_path),
        )?;
        assert_eq!(
            result.get("CFBundleIconName").and_then(Value::as_string),
            Some("AppIcon")
        );
        Ok(())
    }

    #[test]
    fn user_plist_is_optional() -> anyhow::Result<()> {
        let temporary = tempfile::tempdir()?;
        let input = input(temporary.path(), "macos")?;
        let metadata = Metadata::read(&input)?;
        let result = build_info_plist(&input, &metadata, &toolchain("macosx"), None, None)?;
        assert_eq!(
            result.get("CFBundleName").and_then(Value::as_string),
            Some("Demo App")
        );
        assert_eq!(
            result.get("DTPlatformName").and_then(Value::as_string),
            Some("macosx")
        );
        assert_eq!(
            result.get("DTSDKName").and_then(Value::as_string),
            Some("macosx26.5")
        );
        Ok(())
    }

    #[test]
    fn includes_toolchain_metadata() -> anyhow::Result<()> {
        let temporary = tempfile::tempdir()?;
        let input = input(temporary.path(), "ios")?;
        let metadata = Metadata::read(&input)?;
        let result = build_info_plist(&input, &metadata, &toolchain("iphoneos"), None, None)?;
        for (key, value) in [
            ("DTPlatformName", "iphoneos"),
            ("DTPlatformVersion", "26.5"),
            ("DTSDKName", "iphoneos26.5"),
            ("DTSDKBuild", "23F73"),
            ("DTPlatformBuild", "PLATFORM23"),
            ("DTXcode", "2650"),
            ("DTXcodeBuild", "17F113"),
        ] {
            assert_eq!(
                result.get(key).and_then(Value::as_string),
                Some(value),
                "missing or incorrect {key}"
            );
        }
        Ok(())
    }

    #[test]
    fn user_plist_path_is_relative_to_the_project() -> anyhow::Result<()> {
        let temporary = tempfile::tempdir()?;
        let plist = temporary.path().join("Info.plist");
        std::fs::write(&plist, b"plist")?;
        assert_eq!(
            resolve_user_plist(
                temporary.path(),
                "TOKAMAK_IOS_PLIST",
                Some("Info.plist".into()),
            )?,
            Some(plist)
        );
        Ok(())
    }

    #[test]
    fn rejects_a_non_dictionary_user_plist() -> anyhow::Result<()> {
        let temporary = tempfile::tempdir()?;
        let input = input(temporary.path(), "ios")?;
        let user_path = temporary.path().join("User.plist");
        Value::Array(Vec::new()).to_file_xml(&user_path)?;
        let metadata = Metadata::read(&input)?;
        let error = build_info_plist(
            &input,
            &metadata,
            &toolchain("iphoneos"),
            Some(&user_path),
            None,
        )
        .err()
        .context("expected a root dictionary error")?;
        assert!(error.to_string().contains("root must be a dictionary"));
        Ok(())
    }

    #[test]
    fn validates_apple_build_numbers() {
        for value in ["1", "1.2", "1.2.3", "001"] {
            assert!(validate_build_number(value).is_ok(), "accepted {value}");
        }
        for value in ["", "1.", ".1", "1..2", "1.2.3.4", "1.2-3"] {
            assert!(validate_build_number(value).is_err(), "accepted {value}");
        }
    }

    #[test]
    fn parses_xcode_version_metadata() -> anyhow::Result<()> {
        assert_eq!(
            parse_xcode_version("Xcode 26.5\nBuild version 17F113\n")?,
            ("2650".into(), "17F113".into())
        );
        Ok(())
    }
}
