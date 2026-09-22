//! Minimal Wrangler configuration loading for tokamak packaging.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use jsonc_parser::{ParseOptions, parse_to_serde_value};
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

/// Failures loading or validating a Wrangler configuration.
#[derive(Debug, Error)]
pub enum Error {
    /// Operating-system IO failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// JSON encoding or decoding failed.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// Static asset routing configuration is not valid.
    #[error("invalid asset configuration: {0}")]
    InvalidAssetConfig(String),
    /// A Wrangler module rule is not valid.
    #[error("invalid module rule: {0}")]
    InvalidModuleRule(String),
    /// No Wrangler configuration file could be found.
    #[error("wrangler config not found starting from {0}")]
    ConfigNotFound(PathBuf),
    /// A Wrangler configuration file uses an unsupported format.
    #[error("unsupported wrangler config format: {0}")]
    UnsupportedConfigFormat(PathBuf),
    /// A Wrangler configuration file is syntactically invalid.
    #[error("invalid wrangler config {path}: {message}")]
    InvalidConfig {
        /// Path to the invalid configuration file.
        path: PathBuf,
        /// Parser or validation error details.
        message: String,
    },
    /// tokamak needs a field that is not present in the Wrangler configuration.
    #[error("wrangler config {path} is missing required field {field}")]
    MissingConfigField {
        /// Path to the configuration file.
        path: PathBuf,
        /// Name of the missing field.
        field: &'static str,
    },
    /// A Wrangler name cannot be used as a tokamak app identity.
    #[error("wrangler config name is not a safe app name: {0}")]
    InvalidAppName(String),
    /// The requested environment is not declared in the Wrangler configuration.
    #[error(
        "wrangler environment '{name}' not found in {path}; define env.{name} or omit --env to use top-level values"
    )]
    EnvironmentNotFound {
        /// Path to the configuration file.
        path: PathBuf,
        /// Requested environment name.
        name: String,
    },
}

/// Result type for Wrangler configuration operations.
pub type Result<T> = std::result::Result<T, Error>;

const CONFIG_FILE_NAMES: [&str; 3] = ["wrangler.json", "wrangler.jsonc", "wrangler.toml"];

/// Resolved subset of a Wrangler config that tokamak consumes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WranglerConfig {
    /// Absolute path to the config file that was parsed.
    pub path: PathBuf,
    /// Original Wrangler file whose `vars` supply packaged environment values.
    pub vars_source: PathBuf,
    /// Top-level Worker name used as the tokamak application identity.
    pub name: String,
    /// Worker entrypoint, resolved relative to the config file directory.
    pub main: PathBuf,
    /// Static asset configuration, when the Worker declares assets.
    pub assets: Option<WranglerAssets>,
    /// Text and JSON environment bindings declared in `vars`.
    pub vars: BTreeMap<String, Value>,
    /// Additional module rules declared in `rules`.
    pub rules: Vec<WranglerRule>,
    /// Whether Wrangler should traverse `base_dir` for additional modules.
    pub find_additional_modules: bool,
    /// Directory against which additional-module globs are evaluated.
    pub base_dir: PathBuf,
    /// Named Cloudflare bindings declared by the configuration.
    pub bindings: Vec<WranglerBinding>,
}

/// A named binding declared in a Wrangler configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WranglerBinding {
    /// Binding name exposed to the Worker.
    pub name: String,
    /// Wrangler configuration key that declares the binding.
    pub kind: String,
}

/// The module type declared by a Wrangler rule.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WranglerModuleType {
    /// JavaScript ES module source.
    EsModule,
    /// `CommonJS` JavaScript source.
    CommonJs,
    /// Compiled WebAssembly binary.
    CompiledWasm,
    /// Text data.
    Text,
    /// Arbitrary binary data.
    Data,
}

impl WranglerModuleType {
    /// Parse a Wrangler module rule type.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsupported module type.
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "ESModule" => Ok(Self::EsModule),
            "CommonJS" => Ok(Self::CommonJs),
            "CompiledWasm" => Ok(Self::CompiledWasm),
            "Text" => Ok(Self::Text),
            "Data" => Ok(Self::Data),
            _ => Err(Error::InvalidModuleRule(format!(
                "unsupported module type '{value}'"
            ))),
        }
    }

    /// Whether this module type is a non-code file for `/bundle`.
    #[must_use]
    pub const fn is_bundle_file(self) -> bool {
        matches!(self, Self::CompiledWasm | Self::Text | Self::Data)
    }
}

/// A Wrangler rule selecting additional Worker modules.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WranglerRule {
    /// Module type applied to matching files.
    pub module_type: WranglerModuleType,
    /// POSIX glob patterns evaluated relative to [`WranglerConfig::base_dir`].
    pub globs: Vec<String>,
    /// Whether later matching rules may also apply.
    pub fallthrough: bool,
}

/// Static asset subset of a Wrangler config that tokamak consumes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WranglerAssets {
    /// Static asset directory, resolved relative to the config file directory.
    pub directory: PathBuf,
    /// Worker binding name. Defaults to `ASSETS`.
    pub binding: String,
    /// Cloudflare-style HTML path handling mode.
    pub html_handling: HtmlHandling,
    /// Cloudflare-style asset miss handling mode.
    pub not_found_handling: NotFoundHandling,
}

/// Cloudflare static asset `html_handling` mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HtmlHandling {
    /// Match asset paths exactly.
    None,
    /// Use Cloudflare's automatic trailing-slash behavior.
    Auto,
    /// Prefer directory-index paths.
    Force,
    /// Prefer extension paths.
    Drop,
}

impl HtmlHandling {
    /// Parse a Wrangler `assets.html_handling` value.
    ///
    /// # Errors
    ///
    /// Returns an error for values tokamak does not support.
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "none" => Ok(Self::None),
            "auto-trailing-slash" => Ok(Self::Auto),
            "force-trailing-slash" => Ok(Self::Force),
            "drop-trailing-slash" => Ok(Self::Drop),
            _ => Err(Error::InvalidAssetConfig(format!(
                "unsupported assets.html_handling value '{value}'"
            ))),
        }
    }

    /// Return the Wrangler string representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Auto => "auto-trailing-slash",
            Self::Force => "force-trailing-slash",
            Self::Drop => "drop-trailing-slash",
        }
    }
}

/// Cloudflare static asset `not_found_handling` mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotFoundHandling {
    /// Return a plain 404 when no asset matches.
    None,
    /// Serve `/index.html` with status 200 when no asset matches.
    SinglePageApplication,
    /// Serve the nearest `404.html` with status 404 when no asset matches.
    Page404,
}

impl NotFoundHandling {
    /// Parse a Wrangler `assets.not_found_handling` value.
    ///
    /// # Errors
    ///
    /// Returns an error for values tokamak does not support.
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "none" => Ok(Self::None),
            "single-page-application" => Ok(Self::SinglePageApplication),
            "404-page" => Ok(Self::Page404),
            _ => Err(Error::InvalidAssetConfig(format!(
                "unsupported assets.not_found_handling value '{value}'"
            ))),
        }
    }

    /// Return the Wrangler string representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::SinglePageApplication => "single-page-application",
            Self::Page404 => "404-page",
        }
    }
}

/// Resolve the Wrangler config path to use.
///
/// When `explicit_config` is provided, it is resolved relative to
/// `reference_dir` unless already absolute. Without an explicit path, tokamak
/// mirrors Wrangler's file order and parent-directory search:
/// `wrangler.json`, then `wrangler.jsonc`, then `wrangler.toml`.
///
/// # Errors
///
/// Returns an error if no config file can be found.
pub fn resolve_config_path(
    reference_dir: &Path,
    explicit_config: Option<&Path>,
) -> Result<PathBuf> {
    if let Some(path) = explicit_config {
        return Ok(resolve_path(reference_dir, path));
    }

    for file_name in CONFIG_FILE_NAMES {
        if let Some(path) = find_file_upwards(reference_dir, file_name) {
            return Ok(path);
        }
    }

    Err(Error::ConfigNotFound(reference_dir.to_path_buf()))
}

/// Load a Wrangler configuration file.
///
/// # Errors
///
/// Returns an error when the file cannot be read, cannot be parsed, uses an
/// unsupported format, omits a field tokamak needs to package a Worker, or uses a
/// name that cannot identify a tokamak application.
pub fn load_config(config_path: &Path) -> Result<WranglerConfig> {
    load_config_for_env(config_path, None)
}

/// Load the top-level or a named Wrangler environment.
///
/// # Errors
///
/// Returns an error when the configuration is invalid or the named environment
/// does not exist.
pub fn load_config_for_env(
    config_path: &Path,
    environment: Option<&str>,
) -> Result<WranglerConfig> {
    let config_path = absolute_path(config_path)?;
    let (raw, source) = select_environment(parse_config(&config_path)?, &config_path, environment)?;
    let config_dir = config_path
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let main = raw.main.ok_or_else(|| Error::MissingConfigField {
        path: config_path.clone(),
        field: "main",
    })?;
    let name = raw.name.ok_or_else(|| Error::MissingConfigField {
        path: config_path.clone(),
        field: "name",
    })?;
    if !is_valid_app_name(&name) {
        return Err(Error::InvalidAppName(name));
    }
    let assets = raw
        .assets
        .map(|assets| resolve_assets(&config_path, &config_dir, assets))
        .transpose()?;

    Ok(WranglerConfig {
        vars_source: source.unwrap_or_else(|| config_path.clone()),
        path: config_path,
        name,
        main: resolve_path(&config_dir, Path::new(&main)),
        assets,
        vars: raw.vars,
        rules: raw
            .rules
            .unwrap_or_default()
            .into_iter()
            .map(resolve_rule)
            .collect::<Result<Vec<_>>>()?,
        find_additional_modules: raw.find_additional_modules.unwrap_or_default(),
        base_dir: resolve_path(
            &config_dir,
            raw.base_dir
                .as_deref()
                .map(Path::new)
                .or_else(|| Path::new(&main).parent())
                .unwrap_or(Path::new(".")),
        ),
        bindings: collect_bindings(&raw.other),
    })
}

#[derive(Debug, Deserialize)]
struct RawWranglerConfig {
    name: Option<String>,
    main: Option<String>,
    assets: Option<RawWranglerAssets>,
    #[serde(default)]
    vars: BTreeMap<String, Value>,
    rules: Option<Vec<RawWranglerRule>>,
    find_additional_modules: Option<bool>,
    base_dir: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, RawWranglerConfig>,
    #[serde(flatten)]
    other: BTreeMap<String, Value>,
}

fn select_environment(
    mut raw: RawWranglerConfig,
    config_path: &Path,
    environment: Option<&str>,
) -> Result<(RawWranglerConfig, Option<PathBuf>)> {
    let source = raw
        .other
        .get("userConfigPath")
        .and_then(Value::as_str)
        .map(|path| {
            resolve_path(
                config_path.parent().unwrap_or(Path::new(".")),
                Path::new(path),
            )
        });
    if let Some(name) = environment {
        if let Some(selected) = raw.env.remove(name) {
            raw.name = selected.name.or(raw.name);
            raw.main = selected.main.or(raw.main);
            raw.assets = selected.assets.or(raw.assets);
            raw.rules = selected.rules.or(raw.rules);
            raw.find_additional_modules = selected
                .find_additional_modules
                .or(raw.find_additional_modules);
            raw.base_dir = selected.base_dir.or(raw.base_dir);
            raw.vars = selected.vars;
            raw.other = selected.other;
        } else if source.is_none() {
            return Err(Error::EnvironmentNotFound {
                path: config_path.to_path_buf(),
                name: name.to_owned(),
            });
        }
        if let Some(source) = source.as_ref() {
            raw.vars = parse_config(source)?
                .env
                .remove(name)
                .ok_or_else(|| Error::EnvironmentNotFound {
                    path: source.clone(),
                    name: name.to_owned(),
                })?
                .vars;
        }
    } else if let Some(source) = source.as_ref() {
        raw.vars = parse_config(source)?.vars;
    }
    Ok((raw, source))
}

#[derive(Debug, Deserialize)]
struct RawWranglerRule {
    #[serde(rename = "type")]
    module_type: String,
    globs: Vec<String>,
    #[serde(default)]
    fallthrough: bool,
}

#[derive(Debug, Deserialize)]
struct RawWranglerAssets {
    directory: Option<String>,
    binding: Option<String>,
    html_handling: Option<String>,
    not_found_handling: Option<String>,
}

fn resolve_assets(
    config_path: &Path,
    config_dir: &Path,
    assets: RawWranglerAssets,
) -> Result<WranglerAssets> {
    let directory = assets.directory.ok_or_else(|| Error::MissingConfigField {
        path: config_path.to_path_buf(),
        field: "assets.directory",
    })?;
    let binding = assets.binding.unwrap_or_else(|| "ASSETS".to_owned());
    if binding.is_empty() {
        return Err(Error::InvalidAssetConfig(
            "assets.binding must not be empty".to_owned(),
        ));
    }

    Ok(WranglerAssets {
        directory: resolve_path(config_dir, Path::new(&directory)),
        binding,
        html_handling: assets
            .html_handling
            .as_deref()
            .map(HtmlHandling::parse)
            .transpose()?
            .unwrap_or(HtmlHandling::Auto),
        not_found_handling: assets
            .not_found_handling
            .as_deref()
            .map(NotFoundHandling::parse)
            .transpose()?
            .unwrap_or(NotFoundHandling::None),
    })
}

fn resolve_rule(rule: RawWranglerRule) -> Result<WranglerRule> {
    if rule.globs.is_empty() {
        return Err(Error::InvalidModuleRule(
            "rules.globs must not be empty".to_owned(),
        ));
    }
    Ok(WranglerRule {
        module_type: WranglerModuleType::parse(&rule.module_type)?,
        globs: rule.globs,
        fallthrough: rule.fallthrough,
    })
}

fn collect_bindings(values: &BTreeMap<String, Value>) -> Vec<WranglerBinding> {
    const BINDING_KINDS: &[&str] = &[
        "ai",
        "analytics_engine_datasets",
        "browser",
        "d1_databases",
        "dispatch_namespaces",
        "durable_objects",
        "hyperdrive",
        "images",
        "kv_namespaces",
        "mtls_certificates",
        "pipelines",
        "queues",
        "r2_buckets",
        "rate_limiting",
        "secrets_store_secrets",
        "send_email",
        "services",
        "vectorize",
    ];
    let mut bindings = Vec::new();
    for &kind in BINDING_KINDS {
        if let Some(value) = values.get(kind) {
            collect_binding_values(kind, value, &mut bindings);
        }
    }
    bindings.sort_by(|left, right| left.kind.cmp(&right.kind).then(left.name.cmp(&right.name)));
    bindings
}

fn collect_binding_values(kind: &str, value: &Value, bindings: &mut Vec<WranglerBinding>) {
    match value {
        Value::Array(values) => {
            for value in values {
                collect_binding_values(kind, value, bindings);
            }
        }
        Value::Object(values) => {
            if kind == "durable_objects"
                && let Some(value) = values.get("bindings")
            {
                collect_binding_values(kind, value, bindings);
                return;
            }
            if kind == "queues" {
                let mut nested = false;
                for key in ["producers", "consumers"] {
                    if let Some(value) = values.get(key) {
                        collect_binding_values(kind, value, bindings);
                        nested = true;
                    }
                }
                if nested {
                    return;
                }
            }
            let name = ["binding", "name", "queue", "dataset", "id"]
                .into_iter()
                .find_map(|key| values.get(key).and_then(Value::as_str))
                .unwrap_or("<unnamed>");
            bindings.push(WranglerBinding {
                name: name.to_owned(),
                kind: kind.to_owned(),
            });
        }
        Value::String(name) => bindings.push(WranglerBinding {
            name: name.clone(),
            kind: kind.to_owned(),
        }),
        _ => {}
    }
}

fn parse_config(config_path: &Path) -> Result<RawWranglerConfig> {
    let content = fs::read_to_string(config_path)?;
    let extension = config_path
        .extension()
        .and_then(|extension| extension.to_str());

    match extension {
        Some("json" | "jsonc") => parse_jsonc_config(config_path, &content),
        Some("toml") => parse_toml_config(config_path, &content),
        _ => Err(Error::UnsupportedConfigFormat(config_path.to_path_buf())),
    }
}

fn parse_jsonc_config(config_path: &Path, content: &str) -> Result<RawWranglerConfig> {
    parse_to_serde_value(content, &ParseOptions::default()).map_err(|error| Error::InvalidConfig {
        path: config_path.to_path_buf(),
        message: error.to_string(),
    })
}

fn parse_toml_config(config_path: &Path, content: &str) -> Result<RawWranglerConfig> {
    toml::from_str(content).map_err(|error| Error::InvalidConfig {
        path: config_path.to_path_buf(),
        message: error.to_string(),
    })
}

fn find_file_upwards(start: &Path, file_name: &str) -> Option<PathBuf> {
    let mut dir = absolute_path(start).ok()?;
    loop {
        let candidate = dir.join(file_name);
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
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

/// Return whether a name can be used as a tokamak app label.
///
/// Names become one DNS label of the app's `tokamak.local` host.
#[must_use]
pub fn is_valid_app_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

/// Return the `tokamak.local` host for an app name.
#[must_use]
pub fn app_host(name: &str) -> Option<String> {
    let name = name.to_ascii_lowercase();
    is_valid_app_name(&name).then(|| format!("{name}.tokamak.local"))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::Value;

    use super::{app_host, collect_bindings, is_valid_app_name};

    #[test]
    fn accepts_one_lower_case_dns_label() {
        assert!(is_valid_app_name("my-app"));
        assert!(!is_valid_app_name(""));
        assert!(!is_valid_app_name("-leading"));
        assert!(!is_valid_app_name("trailing-"));
        assert!(!is_valid_app_name("Upper"));
        assert!(!is_valid_app_name(&"a".repeat(64)));
    }

    #[test]
    fn derives_the_tokamak_local_host_from_an_app_name() {
        assert_eq!(app_host("my-app").as_deref(), Some("my-app.tokamak.local"));
        assert_eq!(
            app_host("Invalid").as_deref(),
            Some("invalid.tokamak.local")
        );
        assert_eq!(app_host("not valid"), None);
    }

    #[test]
    fn collects_named_bindings_from_wrangler_like_shapes() -> Result<(), Box<dyn std::error::Error>>
    {
        let values = serde_json::from_str::<BTreeMap<String, Value>>(
            r#"{
                "kv_namespaces": [{"binding": "CACHE", "id": "cache"}],
                "durable_objects": {"bindings": [{"name": "ROOMS", "class_name": "Room"}]},
                "queues": {"producers": [{"binding": "EVENTS", "queue": "events"}]}
            }"#,
        )?;
        let bindings = collect_bindings(&values);
        assert_eq!(bindings.len(), 3);
        assert!(
            bindings
                .iter()
                .any(|binding| { binding.name == "CACHE" && binding.kind == "kv_namespaces" })
        );
        assert!(
            bindings
                .iter()
                .any(|binding| binding.name == "ROOMS" && binding.kind == "durable_objects")
        );
        assert!(
            bindings
                .iter()
                .any(|binding| binding.name == "EVENTS" && binding.kind == "queues")
        );
        Ok(())
    }
}
