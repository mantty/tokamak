//! The app's tokamak configuration: the `config` export of its configuration
//! file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use serde::Deserialize;
use serde_json::{Map, Value};
use tokamak_cli::{SHARED_PLATFORM_KEYS, is_valid_key};

const PLATFORMS: [&str; 4] = ["android", "ios", "macos", "windows"];

/// Resolved Tokamak application configuration.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct TokamakConfig {
    /// Absolute path to the configuration file, when one was loaded.
    pub(crate) path: Option<PathBuf>,
    /// Display names.
    pub(crate) name: PlatformValues<String>,
    /// Application identifiers.
    pub(crate) identifier: PlatformValues<String>,
    /// Application icon paths, absolute.
    pub(crate) icon: PlatformValues<PathBuf>,
    /// Application version, when configured.
    pub(crate) version: Option<String>,
    /// Values for each platform's pack, by platform then key; numbers and
    /// booleans are written as strings.
    pub(crate) pack_values: BTreeMap<String, BTreeMap<String, String>>,
}

impl TokamakConfig {
    /// The directory relative paths in the configuration are relative to.
    pub(crate) fn directory(&self) -> Option<&Path> {
        self.path.as_deref().map(config_dir)
    }
}

/// A configuration value with an optional default and per-platform overrides.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct PlatformValues<T> {
    /// Value used when the platform has no override.
    pub(crate) default: Option<T>,
    /// Android override.
    pub(crate) android: Option<T>,
    /// iOS override, also used by iOS simulators.
    pub(crate) ios: Option<T>,
    /// macOS override.
    pub(crate) macos: Option<T>,
    /// Windows override.
    pub(crate) windows: Option<T>,
}

impl<T> PlatformValues<T> {
    /// Return the value for a platform namespace (`android`, `ios`, `macos`, or
    /// `windows`), falling back to the default.
    pub(crate) fn for_platform(&self, platform: &str) -> Option<&T> {
        let value = match platform {
            "android" => &self.android,
            "ios" => &self.ios,
            "macos" => &self.macos,
            "windows" => &self.windows,
            _ => &None,
        };
        value.as_ref().or(self.default.as_ref())
    }

    /// Convert each value, telling `convert` which field it came from.
    fn try_map<U>(
        self,
        key: &str,
        mut convert: impl FnMut(&str, T) -> Result<U>,
    ) -> Result<PlatformValues<U>> {
        let mut convert =
            |field: String, value: Option<T>| value.map(|value| convert(&field, value)).transpose();
        Ok(PlatformValues {
            default: convert(key.to_owned(), self.default)?,
            android: convert(format!("android.{key}"), self.android)?,
            ios: convert(format!("ios.{key}"), self.ios)?,
            macos: convert(format!("macos.{key}"), self.macos)?,
            windows: convert(format!("windows.{key}"), self.windows)?,
        })
    }
}

/// Return the lowercase ASCII slug of a display name, as used for bundle
/// filenames, application identifiers, and `tokamak.local` hosts.
pub(crate) fn slug(name: &str) -> String {
    let mut slug = String::new();
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').to_owned()
}

/// The configuration `config` that the configuration file `file` exports.
///
/// Top-level values are defaults; a platform object overrides them for that
/// platform. Relative icon paths are resolved against the file's directory.
/// Other platform-object keys are values for that platform's pack. `null`
/// leaves a value unset.
pub(crate) fn parse_config(file: &Path, config: Value) -> Result<TokamakConfig> {
    let Value::Object(mut object) = config else {
        return Err(invalid(file, "config must be an object"));
    };
    let pack_values = take_pack_values(file, &mut object)?;
    let config = resolve_values(file, deserialize(file, object)?)?;
    Ok(TokamakConfig {
        path: Some(file.to_path_buf()),
        pack_values,
        ..config
    })
}

/// Remove the platform-pack values from each platform object in `object`.
fn take_pack_values(
    config_path: &Path,
    object: &mut Map<String, Value>,
) -> Result<BTreeMap<String, BTreeMap<String, String>>> {
    let mut taken = BTreeMap::new();
    for platform in PLATFORMS {
        let Some(Value::Object(platform_object)) = object.get_mut(platform) else {
            continue;
        };
        let values = take_platform_pack_values(config_path, platform, platform_object)?;
        if !values.is_empty() {
            taken.insert(platform.to_owned(), values);
        }
    }
    Ok(taken)
}

fn take_platform_pack_values(
    config_path: &Path,
    platform: &str,
    platform_object: &mut Map<String, Value>,
) -> Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    for (key, value) in std::mem::take(platform_object) {
        if SHARED_PLATFORM_KEYS.contains(&key.as_str()) {
            platform_object.insert(key, value);
            continue;
        }
        let field = format!("{platform}.{key}");
        if !is_valid_key(&key) {
            return Err(invalid(
                config_path,
                format!("{field} must be lowercase words joined by hyphens"),
            ));
        }
        if let Some(value) = pack_value(config_path, &field, value)? {
            values.insert(key, value);
        }
    }
    Ok(values)
}

fn pack_value(config_path: &Path, field: &str, value: Value) -> Result<Option<String>> {
    let value = match value {
        Value::Null => return Ok(None),
        Value::String(value) => value,
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Array(_) | Value::Object(_) => {
            return Err(invalid(
                config_path,
                format!("{field} must be a string, number, or boolean"),
            ));
        }
    };
    validate_value(config_path, field, value).map(Some)
}

fn deserialize(config_path: &Path, object: Map<String, Value>) -> Result<RawTokamakConfig> {
    serde_json::from_value(Value::Object(object)).map_err(|error| invalid(config_path, error))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTokamakConfig {
    name: Option<String>,
    identifier: Option<String>,
    icon: Option<String>,
    version: Option<String>,
    android: Option<RawPlatformConfig>,
    ios: Option<RawPlatformConfig>,
    macos: Option<RawPlatformConfig>,
    windows: Option<RawPlatformConfig>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPlatformConfig {
    name: Option<String>,
    identifier: Option<String>,
    icon: Option<String>,
}

/// Validate the values and resolve relative icon paths against `config_path`'s directory.
fn resolve_values(config_path: &Path, raw: RawTokamakConfig) -> Result<TokamakConfig> {
    let config_dir = config_dir(config_path);
    let RawTokamakConfig {
        name,
        identifier,
        icon,
        version,
        android,
        ios,
        macos,
        windows,
    } = raw;
    let android = android.unwrap_or_default();
    let ios = ios.unwrap_or_default();
    let macos = macos.unwrap_or_default();
    let windows = windows.unwrap_or_default();
    let name = PlatformValues {
        default: name,
        android: android.name,
        ios: ios.name,
        macos: macos.name,
        windows: windows.name,
    };
    let identifier = PlatformValues {
        default: identifier,
        android: android.identifier,
        ios: ios.identifier,
        macos: macos.identifier,
        windows: windows.identifier,
    };
    let icon = PlatformValues {
        default: icon,
        android: android.icon,
        ios: ios.icon,
        macos: macos.icon,
        windows: windows.icon,
    };
    Ok(TokamakConfig {
        path: None,
        name: name.try_map("name", |field, value| {
            validate_name(config_path, field, value)
        })?,
        identifier: identifier.try_map("identifier", |field, value| {
            validate_value(config_path, field, value)
        })?,
        icon: icon.try_map("icon", |field, value| {
            validate_value(config_path, field, value).map(|value| config_dir.join(value))
        })?,
        version: version
            .map(|version| validate_value(config_path, "version", version))
            .transpose()?,
        pack_values: BTreeMap::new(),
    })
}

fn config_dir(config_path: &Path) -> &Path {
    config_path.parent().unwrap_or(Path::new("."))
}

/// Why `name` cannot be an app display name, when it cannot.
pub(crate) fn app_name_problem(name: &str) -> Option<&'static str> {
    let slug = slug(name);
    if slug.is_empty() {
        Some("must contain an ASCII letter or digit")
    } else if slug.len() > 63 {
        Some("slug must be at most 63 characters")
    } else {
        None
    }
}

fn validate_name(config_path: &Path, field: &str, value: String) -> Result<String> {
    let value = validate_value(config_path, field, value)?;
    match app_name_problem(&value) {
        Some(problem) => Err(invalid(config_path, format!("{field} {problem}"))),
        None => Ok(value),
    }
}

/// Why `value` cannot be a setting value, when it cannot.
pub(crate) fn value_problem(value: &str) -> Option<&'static str> {
    (value.trim().is_empty() || value != value.trim() || value.chars().any(char::is_control))
        .then_some("must be a non-empty value without surrounding whitespace or control characters")
}

fn validate_value(config_path: &Path, field: &str, value: String) -> Result<String> {
    match value_problem(&value) {
        Some(problem) => Err(invalid(config_path, format!("{field} {problem}"))),
        None => Ok(value),
    }
}

fn invalid(config_path: &Path, message: impl std::fmt::Display) -> anyhow::Error {
    anyhow!(
        "invalid tokamak config {}: {message}",
        config_path.display()
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use anyhow::{Context, Result};

    use super::{PlatformValues, TokamakConfig, parse_config, slug};

    type TestResult<T = ()> = Result<T>;

    /// The configuration file the tests' configurations come from.
    fn config_file(directory: &Path) -> PathBuf {
        directory.join("src/tokamak.ts")
    }

    fn load(directory: &Path, config: &str) -> TestResult<TokamakConfig> {
        parse_config(&config_file(directory), serde_json::from_str(config)?)
    }

    /// What is wrong with `config`, after the prefix naming its file.
    fn invalid_message(directory: &Path, config: &str) -> TestResult<String> {
        let Err(error) = load(directory, config) else {
            anyhow::bail!("accepted {config}");
        };
        let prefix = format!(
            "invalid tokamak config {}: ",
            config_file(directory).display()
        );
        error
            .to_string()
            .strip_prefix(&prefix)
            .map(str::to_owned)
            .with_context(|| format!("{error} does not start with {prefix}"))
    }

    fn strings(values: &PlatformValues<&str>) -> PlatformValues<String> {
        PlatformValues {
            default: values.default.map(str::to_owned),
            android: values.android.map(str::to_owned),
            ios: values.ios.map(str::to_owned),
            macos: values.macos.map(str::to_owned),
            windows: values.windows.map(str::to_owned),
        }
    }

    #[test]
    fn top_level_values_are_defaults_and_platform_objects_override_them() -> TestResult {
        let temporary = tempfile::tempdir()?;
        let config = load(
            temporary.path(),
            r#"{
              "name": "My App",
              "identifier": "com.example.myapp",
              "icon": "assets/AppIcon.icon",
              "version": "1.0.0",
              "ios": { "name": "Myapp Pro", "icon": "/icons/Pro.icon" },
              "android": { "identifier": "com.example.myapp.android" }
            }"#,
        )?;
        let source = temporary.path().join("src");

        assert_eq!(
            config,
            TokamakConfig {
                path: Some(config_file(temporary.path())),
                name: strings(&PlatformValues {
                    default: Some("My App"),
                    ios: Some("Myapp Pro"),
                    ..PlatformValues::default()
                }),
                identifier: strings(&PlatformValues {
                    default: Some("com.example.myapp"),
                    android: Some("com.example.myapp.android"),
                    ..PlatformValues::default()
                }),
                icon: PlatformValues {
                    default: Some(source.join("assets/AppIcon.icon")),
                    ios: Some(PathBuf::from("/icons/Pro.icon")),
                    ..PlatformValues::default()
                },
                version: Some("1.0.0".to_owned()),
                pack_values: BTreeMap::new(),
            }
        );
        assert_eq!(
            config.name.for_platform("ios").map(String::as_str),
            Some("Myapp Pro")
        );
        assert_eq!(
            config.name.for_platform("macos").map(String::as_str),
            Some("My App")
        );
        Ok(())
    }

    #[test]
    fn a_platform_value_without_a_default_leaves_other_platforms_unset() -> TestResult {
        let temporary = tempfile::tempdir()?;
        let config = load(temporary.path(), r#"{ "ios": { "name": "Only iOS" } }"#)?;

        assert_eq!(
            config.name.for_platform("ios").map(String::as_str),
            Some("Only iOS")
        );
        assert_eq!(config.name.for_platform("android"), None);
        assert_eq!(config.identifier, PlatformValues::default());
        Ok(())
    }

    #[test]
    fn null_leaves_a_value_unset() -> TestResult {
        let temporary = tempfile::tempdir()?;
        let config = load(
            temporary.path(),
            r#"{ "version": null, "ios": { "name": null, "team-id": null } }"#,
        )?;

        assert_eq!(config.version, None);
        assert_eq!(config.name, PlatformValues::default());
        assert!(config.pack_values.is_empty());
        Ok(())
    }

    #[test]
    fn slugs_names_for_hosts_and_filenames() {
        assert_eq!(slug("My App"), "my-app");
        assert_eq!(slug("  Über--App!! "), "ber-app");
    }

    #[test]
    fn rejects_invalid_values_and_reports_the_field() -> TestResult {
        let temporary = tempfile::tempdir()?;
        let cases = [
            (r#"{ "name": "!!!" }"#, "name must contain an ASCII letter"),
            (
                r#"{ "ios": { "name": " padded" } }"#,
                "ios.name must be a non-empty value",
            ),
            (r#"{ "version": "" }"#, "version must be a non-empty value"),
            (
                r#"{ "windows": { "icon": "  " } }"#,
                "windows.icon must be a non-empty value",
            ),
            ("[]", "config must be an object"),
        ];
        for (content, expected) in cases {
            let message = invalid_message(temporary.path(), content)?;
            assert!(message.starts_with(expected), "{content}: {message}");
        }
        Ok(())
    }

    #[test]
    fn rejects_unknown_top_level_fields() -> TestResult {
        let temporary = tempfile::tempdir()?;
        assert!(invalid_message(temporary.path(), r#"{ "icons": {} }"#)?.contains("unknown field"));
        assert!(invalid_message(temporary.path(), r#"{ "ios": "x" }"#)?.contains("invalid type"));
        Ok(())
    }

    #[test]
    fn passes_other_platform_keys_to_the_pack() -> TestResult {
        let temporary = tempfile::tempdir()?;
        let config = load(
            temporary.path(),
            r#"{
              "ios": { "name": "iOS App", "plist": "native/Info.plist", "build-number": 5 },
              "macos": { "hardened-runtime": true }
            }"#,
        )?;

        assert_eq!(config.name.ios.as_deref(), Some("iOS App"));
        assert_eq!(
            config.pack_values,
            BTreeMap::from([
                (
                    "ios".to_owned(),
                    BTreeMap::from([
                        ("build-number".to_owned(), "5".to_owned()),
                        ("plist".to_owned(), "native/Info.plist".to_owned()),
                    ])
                ),
                (
                    "macos".to_owned(),
                    BTreeMap::from([("hardened-runtime".to_owned(), "true".to_owned())])
                ),
            ])
        );
        assert_eq!(
            config.directory(),
            Some(temporary.path().join("src").as_path())
        );
        Ok(())
    }

    #[test]
    fn rejects_invalid_pack_keys_and_values() -> TestResult {
        let temporary = tempfile::tempdir()?;
        assert_eq!(
            invalid_message(temporary.path(), r#"{ "ios": { "Plist": "Info.plist" } }"#)?,
            "ios.Plist must be lowercase words joined by hyphens"
        );
        assert_eq!(
            invalid_message(
                temporary.path(),
                r#"{ "ios": { "plist": ["Info.plist"] } }"#
            )?,
            "ios.plist must be a string, number, or boolean"
        );
        assert!(
            invalid_message(temporary.path(), r#"{ "ios": { "plist": " " } }"#)?
                .starts_with("ios.plist")
        );
        Ok(())
    }
}
