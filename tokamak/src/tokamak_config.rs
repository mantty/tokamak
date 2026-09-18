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
    /// Configured display names, when configured.
    pub name: Option<TokamakName>,
    /// Application identifiers, when configured.
    pub identifier: Option<TokamakIdentifier>,
    /// Application version, when configured.
    pub version: Option<String>,
    /// Platform-specific application assets.
    pub icons: Option<TokamakIcons>,
}

/// Configured display names for the supported platforms.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TokamakName {
    /// Display name used when a platform-specific name is not configured.
    pub default: String,
    /// Android display name override.
    pub android: Option<String>,
    /// iOS display name override, also used by iOS simulators.
    pub ios: Option<String>,
    /// macOS display name override.
    pub macos: Option<String>,
    /// Windows display name override.
    pub windows: Option<String>,
}

impl TokamakName {
    /// Return the configured display name for a platform, falling back to default.
    #[must_use]
    pub fn for_platform(&self, platform: &str) -> &str {
        let platform_name = match platform {
            "android" => self.android.as_deref(),
            "ios" | "ios-simulator" => self.ios.as_deref(),
            "macos" => self.macos.as_deref(),
            "windows" => self.windows.as_deref(),
            _ => None,
        };
        platform_name.unwrap_or(&self.default)
    }

    /// Return the lowercase ASCII slug for a platform's display name.
    #[must_use]
    pub fn slug_for_platform(&self, platform: &str) -> String {
        normalize_name(self.for_platform(platform))
    }
}

/// Platform-specific application identifiers.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TokamakIdentifier {
    /// Identifier used when a platform-specific identifier is not configured.
    pub default: String,
    /// Android application identifier override.
    pub android: Option<String>,
    /// iOS application identifier override, also used by iOS simulators.
    pub ios: Option<String>,
    /// macOS application identifier override.
    pub macos: Option<String>,
    /// Windows application identifier override.
    pub windows: Option<String>,
}

impl TokamakIdentifier {
    /// Return the configured identifier for a platform, falling back to default.
    #[must_use]
    pub fn for_platform(&self, platform: &str) -> &str {
        let platform_identifier = match platform {
            "android" => self.android.as_deref(),
            "ios" | "ios-simulator" => self.ios.as_deref(),
            "macos" => self.macos.as_deref(),
            "windows" => self.windows.as_deref(),
            _ => None,
        };
        platform_identifier.unwrap_or(&self.default)
    }
}

/// Platform-specific application asset paths.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TokamakIcons {
    /// Android `res` directory contents.
    pub android: Option<PathBuf>,
    /// Apple Icon Composer `.icon` package.
    pub ios: Option<PathBuf>,
    /// Apple Icon Composer `.icon` package.
    pub macos: Option<PathBuf>,
    /// Windows `.ico` file.
    pub windows: Option<PathBuf>,
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
/// Relative asset paths are resolved against the configuration file directory.
///
/// # Errors
///
/// Returns an error when the file cannot be read, parsed, or validated.
pub fn load_config(config_path: &Path) -> Result<TokamakConfig> {
    let config_path = absolute_path(config_path)?;
    validate_config_extension(&config_path)?;
    let raw = parse_config(&config_path)?;
    let config_dir = config_path
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let name = raw
        .name
        .map(|name| resolve_name(&config_path, name))
        .transpose()?;
    let identifier = raw
        .identifier
        .map(|identifier| resolve_identifier(&config_path, identifier))
        .transpose()?;
    let version = raw
        .version
        .map(|version| validate_value(&config_path, "version", version))
        .transpose()?;
    let icons = raw
        .icons
        .map(|icons| resolve_icons(&config_path, &config_dir, icons))
        .transpose()?;

    Ok(TokamakConfig {
        path: Some(config_path),
        name,
        identifier,
        version,
        icons,
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTokamakConfig {
    name: Option<RawTokamakName>,
    identifier: Option<RawTokamakIdentifier>,
    version: Option<String>,
    icons: Option<RawTokamakIcons>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTokamakName {
    default: String,
    android: Option<String>,
    ios: Option<String>,
    macos: Option<String>,
    windows: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTokamakIdentifier {
    default: String,
    android: Option<String>,
    ios: Option<String>,
    macos: Option<String>,
    windows: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTokamakIcons {
    android: Option<String>,
    ios: Option<String>,
    macos: Option<String>,
    windows: Option<String>,
}

fn resolve_name(config_path: &Path, name: RawTokamakName) -> Result<TokamakName> {
    validate_name(config_path, "name.default", &name.default)?;
    validate_optional_name(config_path, "name.android", name.android.as_deref())?;
    validate_optional_name(config_path, "name.ios", name.ios.as_deref())?;
    validate_optional_name(config_path, "name.macos", name.macos.as_deref())?;
    validate_optional_name(config_path, "name.windows", name.windows.as_deref())?;
    Ok(TokamakName {
        default: name.default,
        android: name.android,
        ios: name.ios,
        macos: name.macos,
        windows: name.windows,
    })
}

fn resolve_identifier(
    config_path: &Path,
    identifier: RawTokamakIdentifier,
) -> Result<TokamakIdentifier> {
    Ok(TokamakIdentifier {
        default: validate_value(config_path, "identifier.default", identifier.default)?,
        android: validate_optional_value(config_path, "identifier.android", identifier.android)?,
        ios: validate_optional_value(config_path, "identifier.ios", identifier.ios)?,
        macos: validate_optional_value(config_path, "identifier.macos", identifier.macos)?,
        windows: validate_optional_value(config_path, "identifier.windows", identifier.windows)?,
    })
}

fn validate_optional_name(config_path: &Path, field: &str, value: Option<&str>) -> Result<()> {
    if let Some(value) = value {
        validate_name(config_path, field, value)?;
    }
    Ok(())
}

fn validate_name(config_path: &Path, field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() || value != value.trim() || value.chars().any(char::is_control) {
        return Err(Error::InvalidConfig {
            path: config_path.to_path_buf(),
            message: format!(
                "{field} must be non-empty, without leading or trailing whitespace, and contain no control characters"
            ),
        });
    }
    let normalized = normalize_name(value);
    if normalized.is_empty() {
        return Err(Error::InvalidConfig {
            path: config_path.to_path_buf(),
            message: format!("{field} must contain an ASCII letter or digit"),
        });
    }
    if normalized.len() > 63 {
        return Err(Error::InvalidConfig {
            path: config_path.to_path_buf(),
            message: format!("{field} must normalize to at most 63 characters"),
        });
    }
    Ok(())
}

fn normalize_name(value: &str) -> String {
    let mut normalized = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            normalized.push(character.to_ascii_lowercase());
        } else if !normalized.is_empty() && !normalized.ends_with('-') {
            normalized.push('-');
        }
    }
    normalized.trim_end_matches('-').to_owned()
}

fn validate_optional_value(
    config_path: &Path,
    field: &str,
    value: Option<String>,
) -> Result<Option<String>> {
    value
        .map(|value| validate_value(config_path, field, value))
        .transpose()
}

fn validate_value(config_path: &Path, field: &str, value: String) -> Result<String> {
    if value.trim().is_empty() || value != value.trim() || value.chars().any(char::is_control) {
        return Err(Error::InvalidConfig {
            path: config_path.to_path_buf(),
            message: format!(
                "{field} must be a non-empty value without whitespace or control characters"
            ),
        });
    }
    Ok(value)
}

fn resolve_icons(
    config_path: &Path,
    config_dir: &Path,
    icons: RawTokamakIcons,
) -> Result<TokamakIcons> {
    Ok(TokamakIcons {
        android: resolve_path_value(config_path, config_dir, "icons.android", icons.android)?,
        ios: resolve_path_value(config_path, config_dir, "icons.ios", icons.ios)?,
        macos: resolve_path_value(config_path, config_dir, "icons.macos", icons.macos)?,
        windows: resolve_path_value(config_path, config_dir, "icons.windows", icons.windows)?,
    })
}

fn resolve_path_value(
    config_path: &Path,
    config_dir: &Path,
    field: &str,
    value: Option<String>,
) -> Result<Option<PathBuf>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.trim().is_empty() {
        return Err(Error::InvalidConfig {
            path: config_path.to_path_buf(),
            message: format!("{field} must not be empty"),
        });
    }
    Ok(Some(resolve_path(config_dir, Path::new(&value))))
}

fn parse_config(config_path: &Path) -> Result<RawTokamakConfig> {
    let content = fs::read_to_string(config_path)?;
    parse_to_serde_value(&content, &ParseOptions::default()).map_err(|error| Error::InvalidConfig {
        path: config_path.to_path_buf(),
        message: error.to_string(),
    })
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

    use super::{Error, TokamakIdentifier, TokamakName, load_config, resolve_config_path};

    #[test]
    fn preserves_names_and_provides_platform_slugs() -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempfile::tempdir()?;
        let config_path = temporary.path().join("tokamak.jsonc");
        fs::write(
            &config_path,
            r#"{
                "name": {
                    "default": "My App",
                    "ios": "Myapp Pro",
                },
            }"#,
        )?;

        let config = load_config(&config_path)?;
        let name = config.name.ok_or("name was not loaded")?;

        assert_eq!(
            name,
            TokamakName {
                default: "My App".to_owned(),
                ios: Some("Myapp Pro".to_owned()),
                ..TokamakName::default()
            }
        );
        assert_eq!(name.for_platform("ios"), "Myapp Pro");
        assert_eq!(name.for_platform("ios-simulator"), "Myapp Pro");
        assert_eq!(name.for_platform("macos"), "My App");
        assert_eq!(name.slug_for_platform("ios"), "myapp-pro");
        assert_eq!(name.slug_for_platform("macos"), "my-app");
        Ok(())
    }

    #[test]
    fn rejects_a_name_that_normalizes_to_nothing() -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempfile::tempdir()?;
        let config_path = temporary.path().join("tokamak.json");
        fs::write(&config_path, r#"{"name":{"default":"!!!"}}"#)?;

        assert!(matches!(
            load_config(&config_path),
            Err(Error::InvalidConfig { .. })
        ));
        Ok(())
    }

    #[test]
    fn rejects_a_name_with_outer_whitespace() -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempfile::tempdir()?;
        let config_path = temporary.path().join("tokamak.json");
        fs::write(&config_path, r#"{"name":{"default":" Vigilus"}}"#)?;

        assert!(matches!(
            load_config(&config_path),
            Err(Error::InvalidConfig { .. })
        ));
        Ok(())
    }

    #[test]
    fn requires_default_when_name_is_configured() -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempfile::tempdir()?;
        let config_path = temporary.path().join("tokamak.json");
        fs::write(&config_path, r#"{"name":{"ios":"My App"}}"#)?;

        assert!(matches!(
            load_config(&config_path),
            Err(Error::InvalidConfig { .. })
        ));
        Ok(())
    }

    #[test]
    fn loads_identifiers_and_version() -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempfile::tempdir()?;
        let config_path = temporary.path().join("tokamak.json");
        fs::write(
            &config_path,
            r#"{
                "identifier": {
                    "default": "com.example.app",
                    "ios": "com.example.ios",
                },
                "version": "1.2.3",
            }"#,
        )?;

        let config = load_config(&config_path)?;

        assert_eq!(
            config.identifier,
            Some(TokamakIdentifier {
                default: "com.example.app".to_owned(),
                ios: Some("com.example.ios".to_owned()),
                ..TokamakIdentifier::default()
            })
        );
        assert_eq!(config.version.as_deref(), Some("1.2.3"));
        assert_eq!(config.icons, None);
        Ok(())
    }

    #[test]
    fn requires_default_when_identifier_is_configured() -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempfile::tempdir()?;
        let config_path = temporary.path().join("tokamak.json");
        fs::write(&config_path, r#"{"identifier":{"ios":"com.example.ios"}}"#)?;

        assert!(matches!(
            load_config(&config_path),
            Err(Error::InvalidConfig { .. })
        ));
        Ok(())
    }

    #[test]
    fn rejects_an_empty_version() -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempfile::tempdir()?;
        let config_path = temporary.path().join("tokamak.json");
        fs::write(&config_path, r#"{"version":"  "}"#)?;

        assert!(matches!(
            load_config(&config_path),
            Err(Error::InvalidConfig { .. })
        ));
        Ok(())
    }

    #[test]
    fn loads_jsonc_and_resolves_paths_from_its_directory() -> Result<(), Box<dyn std::error::Error>>
    {
        let temporary = tempfile::tempdir()?;
        let config_path = temporary.path().join("tokamak.jsonc");
        fs::write(
            &config_path,
            r#"{
                // Native application assets.
                "icons": {
                    "ios": "assets/AppIcon.icon",
                },
            }"#,
        )?;

        let config = load_config(&config_path)?;

        assert_eq!(config.path, Some(config_path));
        assert_eq!(
            config.icons.and_then(|icons| icons.ios),
            Some(temporary.path().join("assets/AppIcon.icon"))
        );
        Ok(())
    }

    #[test]
    fn loads_plain_json() -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempfile::tempdir()?;
        let config_path = temporary.path().join("tokamak.json");
        fs::write(&config_path, r#"{"icons":{"windows":"icon.ico"}}"#)?;

        let config = load_config(&config_path)?;

        assert_eq!(
            config.icons.and_then(|icons| icons.windows),
            Some(temporary.path().join("icon.ico"))
        );
        Ok(())
    }

    #[test]
    fn prefers_jsonc_over_json() -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempfile::tempdir()?;
        fs::write(temporary.path().join("tokamak.json"), "{}")?;
        fs::write(temporary.path().join("tokamak.jsonc"), "{}")?;

        let path = resolve_config_path(temporary.path(), None)?;

        assert_eq!(path, Some(temporary.path().join("tokamak.jsonc")));
        Ok(())
    }

    #[test]
    fn allows_an_absent_optional_config() -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempfile::tempdir()?;

        assert_eq!(resolve_config_path(temporary.path(), None)?, None);
        Ok(())
    }

    #[test]
    fn rejects_unknown_fields() -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempfile::tempdir()?;
        let config_path = temporary.path().join("tokamak.json");
        fs::write(&config_path, r#"{"unknown":true}"#)?;

        assert!(matches!(
            load_config(&config_path),
            Err(Error::InvalidConfig { .. })
        ));
        Ok(())
    }
}
