//! The app's tokamak configuration: the options of the tokamak Vite plugin.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use serde::Deserialize;
use serde_json::Value;
use tokamak_cli::{Platform, is_valid_key};

/// The app's tokamak configuration.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct TokamakConfig {
    /// The Vite root, which relative paths are relative to.
    pub(crate) root: PathBuf,
    /// Application version.
    pub(crate) version: Option<String>,
    /// Values for every platform, without pack values.
    top: PlatformConfig,
    /// Each platform's own values, by namespace.
    platforms: BTreeMap<&'static str, PlatformConfig>,
}

/// The values configured for a platform.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct PlatformConfig {
    /// Display name.
    pub(crate) name: Option<String>,
    /// Application identifier.
    pub(crate) identifier: Option<String>,
    /// Icon path, absolute.
    pub(crate) icon: Option<PathBuf>,
    /// Values for the platform's pack; numbers and booleans are written as strings.
    pub(crate) pack: BTreeMap<String, String>,
}

impl TokamakConfig {
    /// `platform`'s values, with the top-level ones where it has none.
    pub(crate) fn for_platform(&self, platform: Platform) -> PlatformConfig {
        let own = self
            .platforms
            .get(platform.namespace())
            .cloned()
            .unwrap_or_default();
        PlatformConfig {
            name: own.name.or_else(|| self.top.name.clone()),
            identifier: own.identifier.or_else(|| self.top.identifier.clone()),
            icon: own.icon.or_else(|| self.top.icon.clone()),
            pack: own.pack,
        }
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

/// The configuration `config`, the plugin's options, in the Vite root `root`.
///
/// Top-level values are defaults; a platform object overrides them for that
/// platform. Relative icon paths are resolved against `root`. Other
/// platform-object keys are values for that platform's pack. `null` leaves a
/// value unset.
pub(crate) fn parse_config(root: &Path, config: &Value) -> Result<TokamakConfig> {
    if !config.is_object() {
        return Err(invalid("options must be an object"));
    }
    let raw = RawConfig::deserialize(config).map_err(invalid)?;
    if let Some(key) = raw.top.pack.keys().next() {
        return Err(invalid(format!("unknown field `{key}`")));
    }
    let version = raw
        .version
        .map(|version| validate_value("version", version))
        .transpose()?;
    let top = platform_config(root, "", raw.top)?;
    let mut platforms = BTreeMap::new();
    for (namespace, object) in [
        ("android", raw.android),
        ("ios", raw.ios),
        ("macos", raw.macos),
        ("windows", raw.windows),
    ] {
        if let Some(object) = object {
            let values = platform_config(root, &format!("{namespace}."), object)?;
            platforms.insert(namespace, values);
        }
    }
    Ok(TokamakConfig {
        root: root.to_path_buf(),
        version,
        top,
        platforms,
    })
}

#[derive(Deserialize)]
struct RawConfig {
    version: Option<String>,
    android: Option<RawPlatformConfig>,
    ios: Option<RawPlatformConfig>,
    macos: Option<RawPlatformConfig>,
    windows: Option<RawPlatformConfig>,
    /// The top-level values; its pack values are unknown fields.
    #[serde(flatten)]
    top: RawPlatformConfig,
}

#[derive(Deserialize)]
struct RawPlatformConfig {
    name: Option<String>,
    identifier: Option<String>,
    icon: Option<String>,
    #[serde(flatten)]
    pack: BTreeMap<String, Value>,
}

/// The values of the object at `prefix`, with relative icon paths under `root`.
fn platform_config(root: &Path, prefix: &str, raw: RawPlatformConfig) -> Result<PlatformConfig> {
    let field = |key: &str| format!("{prefix}{key}");
    let mut pack = BTreeMap::new();
    for (key, value) in raw.pack {
        if !is_valid_key(&key) {
            let message = format!("{} must be lowercase words joined by hyphens", field(&key));
            return Err(invalid(message));
        }
        if let Some(value) = pack_value(&field(&key), value)? {
            pack.insert(key, value);
        }
    }
    Ok(PlatformConfig {
        name: raw
            .name
            .map(|name| validate_name(&field("name"), name))
            .transpose()?,
        identifier: raw
            .identifier
            .map(|identifier| validate_value(&field("identifier"), identifier))
            .transpose()?,
        icon: raw
            .icon
            .map(|icon| validate_value(&field("icon"), icon).map(|icon| root.join(icon)))
            .transpose()?,
        pack,
    })
}

fn pack_value(field: &str, value: Value) -> Result<Option<String>> {
    let value = match value {
        Value::Null => return Ok(None),
        Value::String(value) => value,
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Array(_) | Value::Object(_) => {
            return Err(invalid(format!(
                "{field} must be a string, number, or boolean"
            )));
        }
    };
    validate_value(field, value).map(Some)
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

fn validate_name(field: &str, value: String) -> Result<String> {
    let value = validate_value(field, value)?;
    match app_name_problem(&value) {
        Some(problem) => Err(invalid(format!("{field} {problem}"))),
        None => Ok(value),
    }
}

/// Why `value` cannot be a setting value, when it cannot.
pub(crate) fn value_problem(value: &str) -> Option<&'static str> {
    (value.trim().is_empty() || value != value.trim() || value.chars().any(char::is_control))
        .then_some("must be a non-empty value without surrounding whitespace or control characters")
}

fn validate_value(field: &str, value: String) -> Result<String> {
    match value_problem(&value) {
        Some(problem) => Err(invalid(format!("{field} {problem}"))),
        None => Ok(value),
    }
}

fn invalid(message: impl std::fmt::Display) -> anyhow::Error {
    anyhow!("invalid tokamak() options: {message}")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use anyhow::{Context, Result};
    use tokamak_cli::Platform;

    use super::{PlatformConfig, TokamakConfig, parse_config, slug};

    type TestResult<T = ()> = Result<T>;

    fn load(root: &Path, config: &str) -> TestResult<TokamakConfig> {
        parse_config(root, &serde_json::from_str(config)?)
    }

    /// What is wrong with `config`, after the error's prefix.
    fn invalid_message(root: &Path, config: &str) -> TestResult<String> {
        let Err(error) = load(root, config) else {
            anyhow::bail!("accepted {config}");
        };
        let prefix = "invalid tokamak() options: ";
        error
            .to_string()
            .strip_prefix(prefix)
            .map(str::to_owned)
            .with_context(|| format!("{error} does not start with {prefix}"))
    }

    fn named(
        name: Option<&str>,
        identifier: Option<&str>,
        icon: Option<PathBuf>,
    ) -> PlatformConfig {
        PlatformConfig {
            name: name.map(str::to_owned),
            identifier: identifier.map(str::to_owned),
            icon,
            pack: BTreeMap::new(),
        }
    }

    #[test]
    fn top_level_values_are_defaults_and_platform_objects_override_them() -> TestResult {
        let temporary = tempfile::tempdir()?;
        let pro_icon = temporary.path().join("icons/Pro.icon");
        let config = parse_config(
            temporary.path(),
            &serde_json::json!({
              "name": "My App",
              "identifier": "com.example.myapp",
              "icon": "assets/AppIcon.icon",
              "version": "1.0.0",
              "ios": { "name": "Myapp Pro", "icon": pro_icon },
              "android": { "identifier": "com.example.myapp.android" }
            }),
        )?;
        let icon = temporary.path().join("assets/AppIcon.icon");

        assert_eq!(config.root, temporary.path());
        assert_eq!(config.version.as_deref(), Some("1.0.0"));
        let ios = named(Some("Myapp Pro"), Some("com.example.myapp"), Some(pro_icon));
        assert_eq!(config.for_platform(Platform::Ios), ios);
        assert_eq!(config.for_platform(Platform::IosSimulator), ios);
        assert_eq!(
            config.for_platform(Platform::Android),
            named(
                Some("My App"),
                Some("com.example.myapp.android"),
                Some(icon.clone())
            )
        );
        assert_eq!(
            config.for_platform(Platform::Macos),
            named(Some("My App"), Some("com.example.myapp"), Some(icon))
        );
        Ok(())
    }

    #[test]
    fn a_platform_value_without_a_default_leaves_other_platforms_unset() -> TestResult {
        let temporary = tempfile::tempdir()?;
        let config = load(temporary.path(), r#"{ "ios": { "name": "Only iOS" } }"#)?;

        assert_eq!(
            config.for_platform(Platform::Ios),
            named(Some("Only iOS"), None, None)
        );
        assert_eq!(
            config.for_platform(Platform::Android),
            PlatformConfig::default()
        );
        Ok(())
    }

    #[test]
    fn null_leaves_a_value_unset() -> TestResult {
        let temporary = tempfile::tempdir()?;
        let config = load(
            temporary.path(),
            r#"{ "version": null, "macos": null, "ios": { "name": null, "team-id": null } }"#,
        )?;

        assert_eq!(config.version, None);
        assert_eq!(
            config.for_platform(Platform::Ios),
            PlatformConfig::default()
        );
        assert_eq!(
            config.for_platform(Platform::Macos),
            PlatformConfig::default()
        );
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
            ("[]", "options must be an object"),
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
        assert_eq!(
            invalid_message(temporary.path(), r#"{ "icons": {} }"#)?,
            "unknown field `icons`"
        );
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

        let ios = config.for_platform(Platform::Ios);
        assert_eq!(ios.name.as_deref(), Some("iOS App"));
        assert_eq!(
            ios.pack,
            BTreeMap::from([
                ("build-number".to_owned(), "5".to_owned()),
                ("plist".to_owned(), "native/Info.plist".to_owned()),
            ])
        );
        assert_eq!(
            config.for_platform(Platform::Macos).pack,
            BTreeMap::from([("hardened-runtime".to_owned(), "true".to_owned())])
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
