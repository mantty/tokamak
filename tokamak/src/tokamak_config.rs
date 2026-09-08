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
    /// Platform-specific application assets.
    pub icons: Option<TokamakIcons>,
}

/// Platform-specific application asset paths.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TokamakIcons {
    /// Android `res` directory contents.
    pub android: Option<PathBuf>,
    /// Apple iOS asset catalog directory.
    pub ios: Option<PathBuf>,
    /// macOS `.icns` file.
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
    if !path.exists() {
        return Err(Error::ConfigNotFound(path));
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
    let icons = raw
        .icons
        .map(|icons| resolve_icons(&config_path, &config_dir, icons))
        .transpose()?;

    Ok(TokamakConfig {
        path: Some(config_path),
        icons,
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTokamakConfig {
    #[serde(default)]
    icons: Option<RawTokamakIcons>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTokamakIcons {
    android: Option<String>,
    ios: Option<String>,
    macos: Option<String>,
    windows: Option<String>,
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

    use super::{Error, load_config, resolve_config_path};

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
                    "ios": "assets/ios",
                },
            }"#,
        )?;

        let config = load_config(&config_path)?;

        assert_eq!(config.path, Some(config_path));
        assert_eq!(
            config.icons.and_then(|icons| icons.ios),
            Some(temporary.path().join("assets/ios"))
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
        fs::write(
            temporary.path().join("tokamak.jsonc"),
            r#"{"icons":{"macos":"icon.icns"}}"#,
        )?;

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
