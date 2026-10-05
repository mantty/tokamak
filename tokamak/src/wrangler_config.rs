//! Minimal Wrangler configuration loading for tokamak packaging.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use jsonc_parser::{ParseOptions, parse_to_serde_value};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use thiserror::Error;

use crate::packaging::ModuleType;

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
    /// A storage binding is not valid.
    #[error("invalid {kind} binding in {path}: {message}")]
    InvalidStorageBinding {
        /// Path to the configuration file.
        path: PathBuf,
        /// Wrangler configuration key that declares the binding.
        kind: &'static str,
        /// What is wrong with the binding.
        message: String,
    },
    /// A Wrangler name cannot be used as a tokamak app identity.
    #[error("wrangler config name is not a safe app name: {0}")]
    InvalidAppName(String),
    /// No deploy configuration was found.
    #[error("no {DEPLOY_CONFIG} found in {0} or its parent directories")]
    DeployConfigNotFound(PathBuf),
}

/// Result type for Wrangler configuration operations.
pub type Result<T> = std::result::Result<T, Error>;

const CONFIG_FILE_NAMES: [&str; 3] = ["wrangler.json", "wrangler.jsonc", "wrangler.toml"];
/// Where a build records the configuration it generated for deployment.
const DEPLOY_CONFIG: &str = ".wrangler/deploy/config.json";

/// Resolved subset of a Wrangler config that tokamak consumes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WranglerConfig {
    /// Absolute path to the config file that was parsed.
    pub path: PathBuf,
    /// Top-level Worker name used as the tokamak application identity.
    pub name: String,
    /// Worker entrypoint, resolved relative to the config file directory.
    pub main: PathBuf,
    /// Static asset configuration, when the Worker declares assets.
    pub assets: Option<WranglerAssets>,
    /// Text and JSON environment bindings declared in `vars`.
    pub vars: BTreeMap<String, Value>,
    /// The module rules Wrangler applies: those declared in `rules`, then its
    /// default rules, without the rules it drops.
    pub rules: Vec<WranglerRule>,
    /// Named Cloudflare bindings, other than storage, declared by the configuration.
    pub bindings: Vec<WranglerBinding>,
    /// Storage bindings declared by the configuration.
    pub storage: Vec<WranglerStorage>,
}

/// A storage binding declared in a Wrangler configuration, with the store
/// it names resolved as local Wrangler resolves it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WranglerStorage {
    /// A `kv_namespaces` entry.
    Kv {
        /// `binding`.
        name: String,
        /// `id`, or the binding name.
        id: String,
    },
    /// A `d1_databases` entry.
    D1 {
        /// `binding`.
        name: String,
        /// `database_id`, or the binding name.
        id: String,
        /// Where the database's migrations are.
        migrations: WranglerMigrations,
    },
    /// An `r2_buckets` entry.
    R2 {
        /// `binding`.
        name: String,
        /// `bucket_name`, or the binding name.
        id: String,
    },
}

impl WranglerStorage {
    /// Binding name in the Worker's `env`.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Kv { name, .. } | Self::D1 { name, .. } | Self::R2 { name, .. } => name,
        }
    }

    /// Identifier of the store the binding names.
    #[must_use]
    pub fn store(&self) -> &str {
        match self {
            Self::Kv { id, .. } | Self::D1 { id, .. } | Self::R2 { id, .. } => id,
        }
    }

    /// The Wrangler configuration key that declares the binding.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Kv { .. } => "kv_namespaces",
            Self::D1 { .. } => "d1_databases",
            Self::R2 { .. } => "r2_buckets",
        }
    }
}

/// A D1 binding's migrations settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WranglerMigrations {
    /// `migrations_dir`, resolved against the configuration file that
    /// declares it.
    pub directory: PathBuf,
    /// `migrations_pattern`, relative to `directory`.
    pub pattern: String,
    /// `migrations_table`.
    pub table: String,
}

/// A named binding declared in a Wrangler configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WranglerBinding {
    /// Binding name exposed to the Worker.
    pub name: String,
    /// Wrangler configuration key that declares the binding.
    pub kind: String,
}

/// A Wrangler rule selecting additional Worker modules.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct WranglerRule {
    /// Module type applied to matching files.
    #[serde(rename = "type")]
    pub module_type: ModuleType,
    /// POSIX glob patterns evaluated relative to the directory of
    /// [`WranglerConfig::main`].
    pub globs: Vec<String>,
    /// Whether later matching rules may also apply.
    #[serde(default)]
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

/// Find the Wrangler config file as Wrangler does: `wrangler.json`, then
/// `wrangler.jsonc`, then `wrangler.toml`, in `reference_dir` or its parent
/// directories.
///
/// # Errors
///
/// Returns an error if no config file can be found.
pub fn resolve_config_path(reference_dir: &Path) -> Result<PathBuf> {
    CONFIG_FILE_NAMES
        .iter()
        .find_map(|file_name| find_file_upwards(reference_dir, file_name))
        .ok_or_else(|| Error::ConfigNotFound(reference_dir.to_path_buf()))
}

/// The configuration a build generated for deployment, found as Wrangler
/// finds it: through the `configPath` of the first `.wrangler/deploy/config.json`
/// in `start` or its parent directories, relative to that file's directory.
///
/// # Errors
///
/// Returns an error when there is no deploy configuration, or it is invalid or
/// names a file that does not exist.
pub fn deploy_config_path(start: &Path) -> Result<PathBuf> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct DeployConfig {
        config_path: PathBuf,
    }

    let pointer = find_file_upwards(start, DEPLOY_CONFIG)
        .ok_or_else(|| Error::DeployConfigNotFound(start.to_path_buf()))?;
    let deploy: DeployConfig = parse_config(&pointer)?;
    let path = resolve_path(
        pointer.parent().unwrap_or(Path::new(".")),
        &deploy.config_path,
    );
    if !path.is_file() {
        return Err(Error::ConfigNotFound(path));
    }
    Ok(path)
}

/// Load a Wrangler configuration file.
///
/// # Errors
///
/// Returns an error when the file cannot be read, cannot be parsed, uses an
/// unsupported format, omits a field tokamak needs to package a Worker, or uses a
/// name that cannot identify a tokamak application.
pub fn load_config(config_path: &Path) -> Result<WranglerConfig> {
    let config_path = absolute_path(config_path)?;
    let raw: RawWranglerConfig = parse_config(&config_path)?;
    let config_dir = config_path
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let main = raw.main.ok_or_else(|| Error::MissingConfigField {
        path: config_path.clone(),
        field: "main",
    })?;
    let name = raw
        .top_level_name
        .or(raw.name)
        .ok_or_else(|| Error::MissingConfigField {
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
    let storage = collect_storage(&config_path, &config_dir, &raw.other)?;

    Ok(WranglerConfig {
        path: config_path,
        name,
        main: resolve_path(&config_dir, Path::new(&main)),
        assets,
        vars: raw.vars,
        rules: applied_rules(
            raw.rules
                .unwrap_or_default()
                .into_iter()
                .map(resolve_rule)
                .collect::<Result<Vec<_>>>()?,
        ),
        storage,
        bindings: collect_bindings(&raw.other),
    })
}

#[derive(Debug, Deserialize)]
struct RawWranglerConfig {
    name: Option<String>,
    /// The Worker's name without an environment's suffix, in a generated
    /// configuration.
    #[serde(rename = "topLevelName")]
    top_level_name: Option<String>,
    main: Option<String>,
    assets: Option<RawWranglerAssets>,
    #[serde(default)]
    vars: BTreeMap<String, Value>,
    rules: Option<Vec<WranglerRule>>,
    #[serde(flatten)]
    other: BTreeMap<String, Value>,
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

/// `rules` followed by Wrangler's default rules, without the rules Wrangler
/// drops: those after a rule of the same type without `fallthrough`.
fn applied_rules(rules: Vec<WranglerRule>) -> Vec<WranglerRule> {
    let defaults = [
        (ModuleType::Text, &["**/*.txt", "**/*.html", "**/*.sql"][..]),
        (ModuleType::Data, &["**/*.bin"]),
        (ModuleType::CompiledWasm, &["**/*.wasm", "**/*.wasm?module"]),
    ]
    .map(|(module_type, globs)| WranglerRule {
        module_type,
        globs: globs.iter().map(|glob| (*glob).to_owned()).collect(),
        fallthrough: false,
    });
    let mut completed = Vec::new();
    let mut applied = Vec::new();
    for rule in rules.into_iter().chain(defaults) {
        if completed.contains(&rule.module_type) {
            continue;
        }
        if !rule.fallthrough {
            completed.push(rule.module_type);
        }
        applied.push(rule);
    }
    applied
}

fn resolve_rule(rule: WranglerRule) -> Result<WranglerRule> {
    if rule.globs.is_empty() {
        return Err(Error::InvalidModuleRule(
            "rules.globs must not be empty".to_owned(),
        ));
    }
    Ok(rule)
}

#[derive(Deserialize)]
struct RawKvNamespace {
    binding: Option<String>,
    id: Option<String>,
}

#[derive(Deserialize)]
struct RawR2Bucket {
    binding: Option<String>,
    bucket_name: Option<String>,
}

#[derive(Deserialize)]
struct RawD1Database {
    binding: Option<String>,
    database_id: Option<String>,
    migrations_dir: Option<String>,
    migrations_pattern: Option<String>,
    migrations_table: Option<String>,
}

fn collect_storage(
    config_path: &Path,
    config_dir: &Path,
    values: &BTreeMap<String, Value>,
) -> Result<Vec<WranglerStorage>> {
    let mut storage = storage_of_kind(config_path, values, "kv_namespaces", resolve_kv)?;
    storage.extend(storage_of_kind(
        config_path,
        values,
        "d1_databases",
        |raw| resolve_d1(config_dir, raw),
    )?);
    storage.extend(storage_of_kind(
        config_path,
        values,
        "r2_buckets",
        resolve_r2,
    )?);
    Ok(storage)
}

/// The bindings of one kind, each resolved from its configuration entry.
fn storage_of_kind<T: DeserializeOwned>(
    config_path: &Path,
    values: &BTreeMap<String, Value>,
    kind: &'static str,
    resolve: impl Fn(T) -> std::result::Result<WranglerStorage, String>,
) -> Result<Vec<WranglerStorage>> {
    let invalid = |message: String| Error::InvalidStorageBinding {
        path: config_path.to_path_buf(),
        kind,
        message,
    };
    let Some(entries) = values.get(kind) else {
        return Ok(Vec::new());
    };
    Vec::<T>::deserialize(entries)
        .map_err(|error| invalid(error.to_string()))?
        .into_iter()
        .map(|raw| resolve(raw).map_err(invalid))
        .collect()
}

fn binding_name(binding: Option<String>) -> std::result::Result<String, String> {
    binding
        .filter(|binding| !binding.is_empty())
        .ok_or_else(|| "every binding needs a non-empty \"binding\" name".to_owned())
}

fn resolve_kv(raw: RawKvNamespace) -> std::result::Result<WranglerStorage, String> {
    let name = binding_name(raw.binding)?;
    Ok(WranglerStorage::Kv {
        id: raw.id.unwrap_or_else(|| name.clone()),
        name,
    })
}

fn resolve_r2(raw: RawR2Bucket) -> std::result::Result<WranglerStorage, String> {
    let name = binding_name(raw.binding)?;
    Ok(WranglerStorage::R2 {
        id: raw.bucket_name.unwrap_or_else(|| name.clone()),
        name,
    })
}

fn resolve_d1(
    config_dir: &Path,
    raw: RawD1Database,
) -> std::result::Result<WranglerStorage, String> {
    let name = binding_name(raw.binding)?;
    let pattern = match (&raw.migrations_dir, raw.migrations_pattern) {
        (_, None) => "*.sql".to_owned(),
        (Some(directory), Some(pattern)) => pattern_within(directory, &pattern)?,
        (None, Some(pattern)) => {
            return Err(format!(
                "migrations_pattern \"{pattern}\" needs migrations_dir"
            ));
        }
    };
    let directory = raw
        .migrations_dir
        .unwrap_or_else(|| "migrations".to_owned());
    Ok(WranglerStorage::D1 {
        id: raw.database_id.unwrap_or_else(|| name.clone()),
        name,
        migrations: WranglerMigrations {
            directory: resolve_path(config_dir, Path::new(&directory)),
            pattern,
            table: raw
                .migrations_table
                .unwrap_or_else(|| "d1_migrations".to_owned()),
        },
    })
}

/// `pattern`, which Wrangler requires to lie within `directory`, relative to
/// `directory`.
fn pattern_within(directory: &str, pattern: &str) -> std::result::Result<String, String> {
    let directory = normalize_relative_path(directory);
    let pattern = normalize_relative_path(pattern);
    if directory == "." {
        return Ok(pattern);
    }
    pattern
        .strip_prefix(&format!("{directory}/"))
        .map(str::to_owned)
        .ok_or_else(|| format!("migrations_pattern \"{pattern}\" must start with \"{directory}/\""))
}

/// A relative path with forward slashes and without `.` segments, as
/// Wrangler normalizes migrations settings before comparing them.
fn normalize_relative_path(path: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    for segment in path.split(['/', '\\']) {
        match segment {
            "" | "." => {}
            ".." if segments.last().is_some_and(|last| *last != "..") => {
                segments.pop();
            }
            segment => segments.push(segment),
        }
    }
    if segments.is_empty() {
        ".".to_owned()
    } else {
        segments.join("/")
    }
}

fn collect_bindings(values: &BTreeMap<String, Value>) -> Vec<WranglerBinding> {
    const BINDING_KINDS: &[&str] = &[
        "ai",
        "analytics_engine_datasets",
        "browser",
        "dispatch_namespaces",
        "durable_objects",
        "hyperdrive",
        "images",
        "mtls_certificates",
        "pipelines",
        "queues",
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

fn parse_config<T: DeserializeOwned>(config_path: &Path) -> Result<T> {
    let content = fs::read_to_string(config_path)?;
    let extension = config_path
        .extension()
        .and_then(|extension| extension.to_str());

    let parsed = match extension {
        Some("json" | "jsonc") => parse_to_serde_value(&content, &ParseOptions::default())
            .map_err(|error| error.to_string()),
        Some("toml") => toml::from_str(&content).map_err(|error| error.to_string()),
        _ => return Err(Error::UnsupportedConfigFormat(config_path.to_path_buf())),
    };
    parsed.map_err(|message| Error::InvalidConfig {
        path: config_path.to_path_buf(),
        message,
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

    use std::fs;

    use super::{
        Error, ModuleType, WranglerMigrations, WranglerRule, WranglerStorage, app_host,
        applied_rules, collect_bindings, deploy_config_path, is_valid_app_name, load_config,
        normalize_relative_path, pattern_within,
    };

    type TestResult = Result<(), Box<dyn std::error::Error>>;

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
    fn collects_named_bindings_from_wrangler_like_shapes() -> TestResult {
        let values = serde_json::from_str::<BTreeMap<String, Value>>(
            r#"{
                "vectorize": [{"binding": "INDEX", "index_name": "index"}],
                "durable_objects": {"bindings": [{"name": "ROOMS", "class_name": "Room"}]},
                "queues": {"producers": [{"binding": "EVENTS", "queue": "events"}]}
            }"#,
        )?;
        let bindings = collect_bindings(&values);
        assert_eq!(bindings.len(), 3);
        assert!(
            bindings
                .iter()
                .any(|binding| { binding.name == "INDEX" && binding.kind == "vectorize" })
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

    #[test]
    fn resolves_storage_bindings_with_local_wrangler_fallbacks() -> TestResult {
        let directory = tempfile::tempdir()?;
        let config = directory.path().join("wrangler.jsonc");
        fs::write(
            &config,
            r#"{
                "name": "app",
                "main": "worker.js",
                "kv_namespaces": [{ "binding": "SETTINGS", "id": "abc", "preview_id": "p" }, { "binding": "SESSION" }],
                "r2_buckets": [{ "binding": "FILES", "bucket_name": "files" }, { "binding": "UPLOADS" }],
                "queues": { "producers": [{ "binding": "EVENTS", "queue": "events" }] },
                "d1_databases": [
                    { "binding": "DB", "database_name": "app", "database_id": "db-id", "migrations_dir": "db/migrations", "migrations_table": "applied" },
                    { "binding": "LOCAL" }
                ]
            }"#,
        )?;

        let loaded = load_config(&config)?;

        assert_eq!(
            loaded.storage,
            vec![
                WranglerStorage::Kv {
                    name: "SETTINGS".to_owned(),
                    id: "abc".to_owned(),
                },
                WranglerStorage::Kv {
                    name: "SESSION".to_owned(),
                    id: "SESSION".to_owned(),
                },
                WranglerStorage::D1 {
                    name: "DB".to_owned(),
                    id: "db-id".to_owned(),
                    migrations: WranglerMigrations {
                        directory: directory.path().join("db/migrations"),
                        pattern: "*.sql".to_owned(),
                        table: "applied".to_owned(),
                    },
                },
                WranglerStorage::D1 {
                    name: "LOCAL".to_owned(),
                    id: "LOCAL".to_owned(),
                    migrations: WranglerMigrations {
                        directory: directory.path().join("migrations"),
                        pattern: "*.sql".to_owned(),
                        table: "d1_migrations".to_owned(),
                    },
                },
                WranglerStorage::R2 {
                    name: "FILES".to_owned(),
                    id: "files".to_owned(),
                },
                WranglerStorage::R2 {
                    name: "UPLOADS".to_owned(),
                    id: "UPLOADS".to_owned(),
                },
            ]
        );
        assert_eq!(loaded.bindings.len(), 1);
        assert_eq!(loaded.bindings[0].kind, "queues");
        Ok(())
    }

    #[test]
    fn applies_default_rules_without_those_wrangler_drops() {
        let rule = |module_type, glob: &str, fallthrough| WranglerRule {
            module_type,
            globs: vec![glob.to_owned()],
            fallthrough,
        };
        let applied = applied_rules(vec![
            rule(ModuleType::Text, "**/*.md", false),
            rule(ModuleType::Data, "**/*.txt", true),
            rule(ModuleType::Text, "**/*.csv", false),
        ]);

        assert_eq!(
            applied
                .iter()
                .map(|rule| (rule.module_type, rule.globs[0].as_str()))
                .collect::<Vec<_>>(),
            [
                (ModuleType::Text, "**/*.md"),
                (ModuleType::Data, "**/*.txt"),
                (ModuleType::Data, "**/*.bin"),
                (ModuleType::CompiledWasm, "**/*.wasm"),
            ]
        );
    }

    #[test]
    fn names_the_app_after_the_top_level_worker() -> TestResult {
        let directory = tempfile::tempdir()?;
        let config = directory.path().join("wrangler.json");
        fs::write(
            &config,
            r#"{ "name": "app-production", "topLevelName": "app", "main": "index.js" }"#,
        )?;

        assert_eq!(load_config(&config)?.name, "app");
        Ok(())
    }

    #[test]
    fn follows_the_deploy_config_from_a_nested_directory() -> TestResult {
        let directory = tempfile::tempdir()?;
        let deploy = directory.path().join(".wrangler/deploy");
        fs::create_dir_all(&deploy)?;
        fs::create_dir_all(directory.path().join("dist/app"))?;
        fs::create_dir_all(directory.path().join("src"))?;
        let generated = directory.path().join("dist/app/wrangler.json");
        fs::write(&generated, r#"{ "name": "app", "main": "index.js" }"#)?;
        fs::write(
            deploy.join("config.json"),
            r#"{ "configPath": "../../dist/app/wrangler.json", "auxiliaryWorkers": [] }"#,
        )?;

        assert_eq!(
            deploy_config_path(&directory.path().join("src"))?,
            deploy.join("../../dist/app/wrangler.json")
        );
        Ok(())
    }

    #[test]
    fn reports_a_missing_deploy_config_or_generated_config() -> TestResult {
        let directory = tempfile::tempdir()?;
        assert!(matches!(
            deploy_config_path(directory.path()),
            Err(Error::DeployConfigNotFound(path)) if path == directory.path()
        ));

        let deploy = directory.path().join(".wrangler/deploy");
        fs::create_dir_all(&deploy)?;
        fs::write(
            deploy.join("config.json"),
            r#"{ "configPath": "missing.json" }"#,
        )?;
        assert!(matches!(
            deploy_config_path(directory.path()),
            Err(Error::ConfigNotFound(path)) if path == deploy.join("missing.json")
        ));
        Ok(())
    }

    #[test]
    fn rejects_a_storage_binding_without_a_name() -> TestResult {
        let directory = tempfile::tempdir()?;
        let config = directory.path().join("wrangler.json");
        fs::write(
            &config,
            r#"{ "name": "app", "main": "worker.js", "d1_databases": [{ "database_id": "db" }] }"#,
        )?;

        let Err(Error::InvalidStorageBinding { kind, .. }) = load_config(&config) else {
            return Err("a nameless binding was accepted".into());
        };
        assert_eq!(kind, "d1_databases");
        Ok(())
    }

    #[test]
    fn requires_migrations_patterns_within_their_directory() {
        assert_eq!(
            pattern_within("./drizzle", "drizzle/*/migration.sql").as_deref(),
            Ok("*/migration.sql")
        );
        assert_eq!(pattern_within(".", "sql/*.sql").as_deref(), Ok("sql/*.sql"));
        assert!(pattern_within("db", "other/*.sql").is_err());
    }

    #[test]
    fn normalizes_relative_paths_as_wrangler_does() {
        assert_eq!(normalize_relative_path("./migrations/"), "migrations");
        assert_eq!(normalize_relative_path("db\\migrations"), "db/migrations");
        assert_eq!(normalize_relative_path("a/../b"), "b");
        assert_eq!(normalize_relative_path("../../db"), "../../db");
        assert_eq!(normalize_relative_path("./"), ".");
    }
}
