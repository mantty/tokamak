//! Tokamak application configuration loading.

use std::fs;
use std::path::{Path, PathBuf};

use jsonc_parser::{ParseOptions, parse_to_serde_value};
use serde::Deserialize;
use thiserror::Error;

const CONFIG_FILE_NAMES: [&str; 2] = ["tokamak.jsonc", "tokamak.json"];

/// Failures loading or validating a Tokamak configuration.
#[derive(Debug, Error)]
pub enum Error {
    /// Operating-system IO failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// A JSON value could not be decoded into the configuration types.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// No Tokamak configuration file could be found at the requested path.
    #[error("tokamak config not found: {0}")]
    ConfigNotFound(PathBuf),
    /// The requested Tokamak configuration file uses an unsupported format.
    #[error("unsupported tokamak config format: {0}")]
    UnsupportedConfigFormat(PathBuf),
    /// A Tokamak configuration file is syntactically invalid or uses invalid fields.
    #[error("invalid tokamak config {path}: {message}")]
    InvalidConfig {
        /// Path to the invalid configuration file.
        path: PathBuf,
        /// Parser or validation error details.
        message: String,
    },
}

/// Result type for Tokamak configuration operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Resolved Tokamak application configuration.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TokamakConfig {
    /// Absolute path to the configuration file, when one was loaded.
    pub path: Option<PathBuf>,
    /// Display names.
    pub name: PlatformValues<String>,
    /// Application identifiers.
    pub identifier: PlatformValues<String>,
    /// Application icon paths, absolute.
    pub icon: PlatformValues<PathBuf>,
    /// Application version, when configured.
    pub version: Option<String>,
}

/// A configuration value with an optional default and per-platform overrides.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PlatformValues<T> {
    /// Value used when the platform has no override.
    pub default: Option<T>,
    /// Android override.
    pub android: Option<T>,
    /// iOS override, also used by iOS simulators.
    pub ios: Option<T>,
    /// macOS override.
    pub macos: Option<T>,
    /// Windows override.
    pub windows: Option<T>,
}

impl<T> PlatformValues<T> {
    /// Return the value for a platform, falling back to the default.
    #[must_use]
    pub fn for_platform(&self, platform: &str) -> Option<&T> {
        let value = match platform {
            "android" => &self.android,
            "ios" | "ios-simulator" => &self.ios,
            "macos" => &self.macos,
            "windows" => &self.windows,
            _ => &None,
        };
        value.as_ref().or(self.default.as_ref())
    }

    /// Take each field from `self`, falling back to `base` where `self` has none.
    fn or(self, base: Self) -> Self {
        Self {
            default: self.default.or(base.default),
            android: self.android.or(base.android),
            ios: self.ios.or(base.ios),
            macos: self.macos.or(base.macos),
            windows: self.windows.or(base.windows),
        }
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

/// A loaded configuration and its warnings.
#[derive(Debug)]
pub struct LoadedConfig {
    /// The resolved configuration.
    pub config: TokamakConfig,
    /// Problems that did not prevent loading, each naming the file concerned.
    pub warnings: Vec<String>,
}

/// Return the lowercase ASCII slug of a display name, as used for bundle
/// filenames, application identifiers, and `tokamak.local` hosts.
#[must_use]
pub fn slug(name: &str) -> String {
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

/// Resolve a Tokamak configuration file or directory.
///
/// An explicit file is returned directly. An explicit directory, or the
/// reference directory when no path is supplied, is searched for
/// `tokamak.jsonc` and then `tokamak.json`. A directory without either file
/// represents an absent optional configuration.
///
/// # Errors
///
/// Returns an error when an explicit path does not exist or uses an
/// unsupported file extension.
pub fn resolve_config_path(
    reference_dir: &Path,
    explicit_config: Option<&Path>,
) -> Result<Option<PathBuf>> {
    let reference_dir = absolute_path(reference_dir)?;
    let path = explicit_config
        .map(|path| resolve_path(&reference_dir, path))
        .unwrap_or(reference_dir);

    if path.is_file() {
        validate_config_extension(&path)?;
        return Ok(Some(path));
    }
    if !path.is_dir() {
        return Err(Error::ConfigNotFound(path));
    }

    for file_name in CONFIG_FILE_NAMES {
        let candidate = path.join(file_name);
        if candidate.is_file() {
            return Ok(Some(candidate));
        }
    }

    Ok(None)
}

/// Load a Tokamak configuration file.
///
/// JSONC parsing is used for both supported extensions, so plain JSON remains
/// valid while comments and trailing commas are available in `.jsonc` files.
/// Top-level values are defaults; a platform object overrides them for that
/// platform. Relative icon paths are resolved against the directory of the
/// file that names them.
///
/// A file may `include` one other configuration file, absolute or relative to
/// the including file. Merging is per field: each top-level and platform value
/// comes from the including file when set there, otherwise from the included
/// file. Only the loaded file may include: an `include` inside the included
/// file is ignored with a warning.
///
/// # Errors
///
/// Returns an error when a file cannot be read, parsed, or validated.
pub fn load_config(config_path: &Path) -> Result<LoadedConfig> {
    let config_path = absolute_path(config_path)?;
    validate_config_extension(&config_path)?;
    let raw = parse_config(&config_path)?;
    let mut warnings = Vec::new();
    let base = raw
        .include
        .as_deref()
        .map(|include| load_include(&config_path, include, &mut warnings))
        .transpose()?
        .unwrap_or_default();
    let own = resolve_values(&config_path, raw)?;
    Ok(LoadedConfig {
        config: TokamakConfig {
            path: Some(config_path),
            name: own.name.or(base.name),
            identifier: own.identifier.or(base.identifier),
            icon: own.icon.or(base.icon),
            version: own.version.or(base.version),
        },
        warnings,
    })
}

fn load_include(
    config_path: &Path,
    include: &str,
    warnings: &mut Vec<String>,
) -> Result<TokamakConfig> {
    let include = validate_value(config_path, "include", include.to_owned())?;
    let include_path = resolve_path(config_dir(config_path), Path::new(&include));
    if include_path == config_path {
        return Err(invalid(
            config_path,
            "include must not name the file itself",
        ));
    }
    validate_config_extension(&include_path)?;
    let raw = parse_config(&include_path).map_err(|error| match error {
        Error::Io(error) => invalid(
            config_path,
            format!("include {}: {error}", include_path.display()),
        ),
        error => error,
    })?;
    if raw.include.is_some() {
        warnings.push(format!(
            "{}: nested include is ignored; only the loaded file may include another",
            include_path.display()
        ));
    }
    resolve_values(&include_path, raw)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTokamakConfig {
    include: Option<String>,
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

/// Validate one file's values and resolve its icon paths against its directory.
fn resolve_values(config_path: &Path, raw: RawTokamakConfig) -> Result<TokamakConfig> {
    let config_dir = config_dir(config_path);
    let RawTokamakConfig {
        include: _,
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
            validate_value(config_path, field, value)
                .map(|value| resolve_path(config_dir, Path::new(&value)))
        })?,
        version: version
            .map(|version| validate_value(config_path, "version", version))
            .transpose()?,
    })
}

fn config_dir(config_path: &Path) -> &Path {
    config_path.parent().unwrap_or(Path::new("."))
}

fn validate_name(config_path: &Path, field: &str, value: String) -> Result<String> {
    let value = validate_value(config_path, field, value)?;
    let slug = slug(&value);
    if slug.is_empty() {
        return Err(invalid(
            config_path,
            format!("{field} must contain an ASCII letter or digit"),
        ));
    }
    if slug.len() > 63 {
        return Err(invalid(
            config_path,
            format!("{field} slug must be at most 63 characters"),
        ));
    }
    Ok(value)
}

fn validate_value(config_path: &Path, field: &str, value: String) -> Result<String> {
    if value.trim().is_empty() || value != value.trim() || value.chars().any(char::is_control) {
        return Err(invalid(
            config_path,
            format!(
                "{field} must be a non-empty value without surrounding whitespace or control characters"
            ),
        ));
    }
    Ok(value)
}

fn invalid(config_path: &Path, message: impl Into<String>) -> Error {
    Error::InvalidConfig {
        path: config_path.to_path_buf(),
        message: message.into(),
    }
}

fn parse_config(config_path: &Path) -> Result<RawTokamakConfig> {
    let content = fs::read_to_string(config_path)?;
    parse_to_serde_value(&content, &ParseOptions::default())
        .map_err(|error| invalid(config_path, error.to_string()))
}

fn validate_config_extension(config_path: &Path) -> Result<()> {
    match config_path
        .extension()
        .and_then(|extension| extension.to_str())
    {
        Some("json" | "jsonc") => Ok(()),
        _ => Err(Error::UnsupportedConfigFormat(config_path.to_path_buf())),
    }
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn resolve_path(base_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::{
        Error, LoadedConfig, PlatformValues, TokamakConfig, load_config, resolve_config_path, slug,
    };

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    fn load(path: &Path, content: &str) -> TestResult<LoadedConfig> {
        fs::write(path, content)?;
        Ok(load_config(path)?)
    }

    fn invalid_message(path: &Path, content: &str) -> TestResult<String> {
        fs::write(path, content)?;
        match load_config(path) {
            Err(Error::InvalidConfig { message, .. }) => Ok(message),
            other => Err(format!("expected an invalid config, got {other:?}").into()),
        }
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
            &temporary.path().join("tokamak.jsonc"),
            r#"{
              // Defaults
              "name": "My App",
              "identifier": "com.example.myapp",
              "icon": "assets/AppIcon.icon",
              "version": "1.0.0",
              "ios": { "name": "Myapp Pro", "icon": "assets/Pro.icon" },
              "android": { "identifier": "com.example.myapp.android" },
            }"#,
        )?
        .config;

        assert_eq!(
            config,
            TokamakConfig {
                path: Some(temporary.path().join("tokamak.jsonc")),
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
                    default: Some(temporary.path().join("assets/AppIcon.icon")),
                    ios: Some(temporary.path().join("assets/Pro.icon")),
                    ..PlatformValues::default()
                },
                version: Some("1.0.0".to_owned()),
            }
        );
        assert_eq!(
            config
                .name
                .for_platform("ios-simulator")
                .map(String::as_str),
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
        let config = load(
            &temporary.path().join("tokamak.json"),
            r#"{ "ios": { "name": "Only iOS" } }"#,
        )?
        .config;

        assert_eq!(
            config.name.for_platform("ios").map(String::as_str),
            Some("Only iOS")
        );
        assert_eq!(config.name.for_platform("android"), None);
        assert_eq!(config.identifier, PlatformValues::default());
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
        let path = temporary.path().join("tokamak.jsonc");
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
        ];
        for (content, expected) in cases {
            let message = invalid_message(&path, content)?;
            assert!(message.starts_with(expected), "{content}: {message}");
        }
        Ok(())
    }

    #[test]
    fn rejects_unknown_fields_at_both_levels() -> TestResult {
        let temporary = tempfile::tempdir()?;
        let path = temporary.path().join("tokamak.json");
        assert!(invalid_message(&path, r#"{ "icons": {} }"#)?.contains("unknown field"));
        assert!(
            invalid_message(&path, r#"{ "ios": { "version": "1" } }"#)?.contains("unknown field")
        );
        Ok(())
    }

    #[test]
    fn includes_another_file_and_merges_each_field() -> TestResult {
        let temporary = tempfile::tempdir()?;
        let shared_dir = temporary.path().join("shared");
        fs::create_dir(&shared_dir)?;
        fs::write(
            shared_dir.join("tokamak.jsonc"),
            r#"{
              "name": "My App",
              "identifier": "com.example.myapp",
              "icon": "icons/AppIcon.icon",
              "version": "1.0.0",
              "ios": { "name": "My App for iOS", "icon": "icons/Pro.icon" },
            }"#,
        )?;
        let loaded = load(
            &temporary.path().join("tokamak.dev.jsonc"),
            r#"{
              "include": "shared/tokamak.jsonc",
              "name": "My Test App",
              "windows": { "icon": "windows/AppIcon.ico" },
            }"#,
        )?;

        assert!(loaded.warnings.is_empty());
        assert_eq!(
            loaded.config,
            TokamakConfig {
                path: Some(temporary.path().join("tokamak.dev.jsonc")),
                name: strings(&PlatformValues {
                    default: Some("My Test App"),
                    ios: Some("My App for iOS"),
                    ..PlatformValues::default()
                }),
                identifier: strings(&PlatformValues {
                    default: Some("com.example.myapp"),
                    ..PlatformValues::default()
                }),
                icon: PlatformValues {
                    default: Some(shared_dir.join("icons/AppIcon.icon")),
                    ios: Some(shared_dir.join("icons/Pro.icon")),
                    windows: Some(temporary.path().join("windows/AppIcon.ico")),
                    ..PlatformValues::default()
                },
                version: Some("1.0.0".to_owned()),
            }
        );
        Ok(())
    }

    #[test]
    fn includes_by_absolute_path() -> TestResult {
        let temporary = tempfile::tempdir()?;
        let shared = temporary.path().join("base.json");
        fs::write(&shared, r#"{ "version": "2.0.0" }"#)?;
        let nested = temporary.path().join("nested");
        fs::create_dir(&nested)?;
        let config = load(
            &nested.join("tokamak.jsonc"),
            &format!(r#"{{ "include": {:?} }}"#, shared.display().to_string()),
        )?
        .config;

        assert_eq!(config.version.as_deref(), Some("2.0.0"));
        Ok(())
    }

    #[test]
    fn ignores_a_nested_include_with_a_warning() -> TestResult {
        let temporary = tempfile::tempdir()?;
        fs::write(
            temporary.path().join("grandparent.jsonc"),
            r#"{ "version": "9.9.9", "name": "Grandparent" }"#,
        )?;
        fs::write(
            temporary.path().join("parent.jsonc"),
            r#"{ "include": "grandparent.jsonc", "identifier": "com.example.parent" }"#,
        )?;
        let loaded = load(
            &temporary.path().join("tokamak.jsonc"),
            r#"{ "include": "parent.jsonc", "name": "Child" }"#,
        )?;

        assert_eq!(
            loaded.warnings,
            vec![format!(
                "{}: nested include is ignored; only the loaded file may include another",
                temporary.path().join("parent.jsonc").display()
            )]
        );
        assert_eq!(loaded.config.version, None);
        assert_eq!(
            loaded.config.identifier.default.as_deref(),
            Some("com.example.parent")
        );
        assert_eq!(loaded.config.name.default.as_deref(), Some("Child"));
        Ok(())
    }

    #[test]
    fn reports_a_missing_or_invalid_include() -> TestResult {
        let temporary = tempfile::tempdir()?;
        let path = temporary.path().join("tokamak.jsonc");
        let message = invalid_message(&path, r#"{ "include": "missing.jsonc" }"#)?;
        assert!(message.starts_with("include "), "{message}");
        assert!(message.contains("missing.jsonc"), "{message}");
        assert!(
            invalid_message(&path, r#"{ "include": "tokamak.jsonc" }"#)?
                .contains("must not name the file itself")
        );
        assert!(
            invalid_message(&path, r#"{ "include": "" }"#)?
                .starts_with("include must be a non-empty value")
        );

        fs::write(temporary.path().join("base.jsonc"), r#"{ "name": "!!!" }"#)?;
        fs::write(&path, r#"{ "include": "base.jsonc" }"#)?;
        let Err(Error::InvalidConfig { path: reported, .. }) = load_config(&path) else {
            return Err("invalid included file was accepted".into());
        };
        assert_eq!(reported, temporary.path().join("base.jsonc"));

        fs::write(&path, r#"{ "include": "base.yaml" }"#)?;
        assert!(matches!(
            load_config(&path),
            Err(Error::UnsupportedConfigFormat(reported)) if reported == temporary.path().join("base.yaml")
        ));
        Ok(())
    }

    #[test]
    fn prefers_jsonc_over_json() -> TestResult {
        let temporary = tempfile::tempdir()?;
        fs::write(temporary.path().join("tokamak.jsonc"), "{}")?;
        fs::write(temporary.path().join("tokamak.json"), "{}")?;

        assert_eq!(
            resolve_config_path(temporary.path(), None)?,
            Some(temporary.path().join("tokamak.jsonc"))
        );
        Ok(())
    }

    #[test]
    fn allows_an_absent_optional_config() -> TestResult {
        let temporary = tempfile::tempdir()?;
        fs::write(temporary.path().join("tokamak.yaml"), "")?;

        assert_eq!(resolve_config_path(temporary.path(), None)?, None);
        assert!(matches!(
            resolve_config_path(temporary.path(), Some(Path::new("missing"))),
            Err(Error::ConfigNotFound(_))
        ));
        assert!(matches!(
            resolve_config_path(temporary.path(), Some(Path::new("tokamak.yaml"))),
            Err(Error::UnsupportedConfigFormat(_))
        ));
        Ok(())
    }
}
