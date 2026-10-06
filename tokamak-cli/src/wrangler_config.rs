//! Minimal Wrangler configuration loading for tokamak packaging.

use std::collections::BTreeMap;
use std::fmt::Display;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokamak::{HtmlHandling, NotFoundHandling};

/// Where a build records the configuration it generated for deployment.
const DEPLOY_CONFIG: &str = ".wrangler/deploy/config.json";

/// Resolved subset of a Wrangler config that tokamak consumes.
#[derive(Debug)]
pub(crate) struct WranglerConfig {
    /// Absolute path to the config file that was parsed.
    pub(crate) path: PathBuf,
    /// Top-level Worker name used as the tokamak application identity.
    pub(crate) name: String,
    /// Worker entrypoint, resolved relative to the config file directory.
    pub(crate) main: PathBuf,
    /// Static asset configuration, when the Worker declares assets.
    pub(crate) assets: Option<WranglerAssets>,
    /// Text and JSON environment bindings declared in `vars`.
    pub(crate) vars: BTreeMap<String, Value>,
    /// The module rules Wrangler applies: those declared in `rules`, then its
    /// default rules, without the rules it drops.
    pub(crate) rules: Vec<WranglerRule>,
    /// Named Cloudflare bindings, other than storage, declared by the configuration.
    pub(crate) bindings: Vec<WranglerBinding>,
    /// Storage bindings declared by the configuration.
    pub(crate) storage: Vec<WranglerStorage>,
}

/// A storage binding declared in a Wrangler configuration, with the store
/// it names resolved as local Wrangler resolves it.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum WranglerStorage {
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
    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Kv { name, .. } | Self::D1 { name, .. } | Self::R2 { name, .. } => name,
        }
    }

    /// Identifier of the store the binding names.
    pub(crate) fn store(&self) -> &str {
        match self {
            Self::Kv { id, .. } | Self::D1 { id, .. } | Self::R2 { id, .. } => id,
        }
    }

    /// The Wrangler configuration key that declares the binding.
    pub(crate) const fn kind(&self) -> &'static str {
        match self {
            Self::Kv { .. } => "kv_namespaces",
            Self::D1 { .. } => "d1_databases",
            Self::R2 { .. } => "r2_buckets",
        }
    }
}

/// A D1 binding's migrations settings.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct WranglerMigrations {
    /// `migrations_dir`, resolved against the configuration file that
    /// declares it.
    pub(crate) directory: PathBuf,
    /// `migrations_pattern`, relative to `directory`.
    pub(crate) pattern: String,
    /// `migrations_table`.
    pub(crate) table: String,
}

/// A named binding declared in a Wrangler configuration.
#[derive(Debug)]
pub(crate) struct WranglerBinding {
    /// Binding name exposed to the Worker.
    pub(crate) name: String,
    /// Wrangler configuration key that declares the binding.
    pub(crate) kind: String,
}

/// A Wrangler rule selecting additional Worker modules.
#[derive(Debug, Deserialize)]
pub(crate) struct WranglerRule {
    /// Module type applied to matching files.
    #[serde(rename = "type")]
    pub(crate) module_type: WranglerModuleType,
    /// POSIX glob patterns evaluated relative to the directory of
    /// [`WranglerConfig::main`].
    pub(crate) globs: Vec<String>,
    /// Whether later matching rules may also apply.
    #[serde(default)]
    fallthrough: bool,
}

/// A module type in Wrangler's module rules, each named as Wrangler names it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub(crate) enum WranglerModuleType {
    ESModule,
    CommonJS,
    CompiledWasm,
    Text,
    Data,
}

/// Static asset subset of a Wrangler config that tokamak consumes.
#[derive(Debug)]
pub(crate) struct WranglerAssets {
    /// Static asset directory, resolved relative to the config file directory.
    pub(crate) directory: PathBuf,
    /// Worker binding name. Defaults to `ASSETS`.
    pub(crate) binding: String,
    /// Cloudflare-style HTML path handling mode.
    pub(crate) html_handling: HtmlHandling,
    /// Cloudflare-style asset miss handling mode.
    pub(crate) not_found_handling: NotFoundHandling,
}

/// The configuration a build generated for deployment, found as Wrangler
/// finds it: through the `configPath` of the first `.wrangler/deploy/config.json`
/// in `start` or its parent directories, relative to that file's directory.
pub(crate) fn deploy_config_path(start: &Path) -> Result<PathBuf> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct DeployConfig {
        config_path: PathBuf,
    }

    let pointer = std::path::absolute(start)?
        .ancestors()
        .map(|directory| directory.join(DEPLOY_CONFIG))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            anyhow!(
                "no {DEPLOY_CONFIG} found in {} or its parent directories",
                start.display()
            )
        })?;
    let deploy: DeployConfig = parse_config(&pointer)?;
    let path = pointer
        .parent()
        .unwrap_or(Path::new("."))
        .join(deploy.config_path);
    if !path.is_file() {
        bail!("wrangler config not found starting from {}", path.display());
    }
    Ok(path)
}

/// Load a generated Wrangler configuration file: fail when it cannot be read
/// or parsed, or omits a field tokamak needs to package a Worker.
pub(crate) fn load_config(config_path: &Path) -> Result<WranglerConfig> {
    let config_path = std::path::absolute(config_path)?;
    let raw: RawWranglerConfig = parse_config(&config_path)?;
    let config_dir = config_path.parent().unwrap_or(Path::new("."));
    let main = raw
        .main
        .ok_or_else(|| missing_field(&config_path, "main"))?;
    let name = raw
        .top_level_name
        .or(raw.name)
        .ok_or_else(|| missing_field(&config_path, "name"))?;
    let assets = raw
        .assets
        .map(|assets| resolve_assets(&config_path, config_dir, assets))
        .transpose()?;
    let storage = collect_storage(&config_path, config_dir, &raw.other)?;

    Ok(WranglerConfig {
        main: config_dir.join(main),
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
        name,
        path: config_path,
    })
}

fn missing_field(config_path: &Path, field: &str) -> anyhow::Error {
    anyhow!(
        "wrangler config {} is missing required field {field}",
        config_path.display()
    )
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
    let directory = assets
        .directory
        .ok_or_else(|| missing_field(config_path, "assets.directory"))?;
    let binding = assets.binding.unwrap_or_else(|| "ASSETS".to_owned());
    if binding.is_empty() {
        bail!("invalid asset configuration: assets.binding must not be empty");
    }

    Ok(WranglerAssets {
        directory: config_dir.join(directory),
        binding,
        html_handling: asset_setting("html_handling", assets.html_handling)?,
        not_found_handling: asset_setting("not_found_handling", assets.not_found_handling)?,
    })
}

/// The `assets.{field}` setting `value`, or its default.
fn asset_setting<T: DeserializeOwned + Default>(field: &str, value: Option<String>) -> Result<T> {
    let Some(value) = value else {
        return Ok(T::default());
    };
    serde_json::from_value(Value::String(value.clone())).map_err(|_| {
        anyhow!("invalid asset configuration: unsupported assets.{field} value '{value}'")
    })
}

/// `rules` followed by Wrangler's default rules, without the rules Wrangler
/// drops: those after a rule of the same type without `fallthrough`.
fn applied_rules(rules: Vec<WranglerRule>) -> Vec<WranglerRule> {
    let defaults = [
        (
            WranglerModuleType::Text,
            &["**/*.txt", "**/*.html", "**/*.sql"][..],
        ),
        (WranglerModuleType::Data, &["**/*.bin"]),
        (
            WranglerModuleType::CompiledWasm,
            &["**/*.wasm", "**/*.wasm?module"],
        ),
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
        bail!("invalid module rule: rules.globs must not be empty");
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
    resolve: impl Fn(T) -> Result<WranglerStorage>,
) -> Result<Vec<WranglerStorage>> {
    let invalid = |error: &dyn Display| {
        anyhow!(
            "invalid {kind} binding in {}: {error}",
            config_path.display()
        )
    };
    let Some(entries) = values.get(kind) else {
        return Ok(Vec::new());
    };
    Vec::<T>::deserialize(entries)
        .map_err(|error| invalid(&error))?
        .into_iter()
        .map(|raw| resolve(raw).map_err(|error| invalid(&error)))
        .collect()
}

fn binding_name(binding: Option<String>) -> Result<String> {
    binding
        .filter(|binding| !binding.is_empty())
        .context("every binding needs a non-empty \"binding\" name")
}

fn resolve_kv(raw: RawKvNamespace) -> Result<WranglerStorage> {
    let name = binding_name(raw.binding)?;
    Ok(WranglerStorage::Kv {
        id: raw.id.unwrap_or_else(|| name.clone()),
        name,
    })
}

fn resolve_r2(raw: RawR2Bucket) -> Result<WranglerStorage> {
    let name = binding_name(raw.binding)?;
    Ok(WranglerStorage::R2 {
        id: raw.bucket_name.unwrap_or_else(|| name.clone()),
        name,
    })
}

fn resolve_d1(config_dir: &Path, raw: RawD1Database) -> Result<WranglerStorage> {
    let name = binding_name(raw.binding)?;
    let pattern = match (&raw.migrations_dir, raw.migrations_pattern) {
        (_, None) => "*.sql".to_owned(),
        (Some(directory), Some(pattern)) => pattern_within(directory, &pattern)?,
        (None, Some(pattern)) => bail!("migrations_pattern \"{pattern}\" needs migrations_dir"),
    };
    let directory = raw
        .migrations_dir
        .unwrap_or_else(|| "migrations".to_owned());
    Ok(WranglerStorage::D1 {
        id: raw.database_id.unwrap_or_else(|| name.clone()),
        name,
        migrations: WranglerMigrations {
            directory: config_dir.join(directory),
            pattern,
            table: raw
                .migrations_table
                .unwrap_or_else(|| "d1_migrations".to_owned()),
        },
    })
}

/// `pattern`, which Wrangler requires to lie within `directory`, relative to
/// `directory`.
fn pattern_within(directory: &str, pattern: &str) -> Result<String> {
    let directory = normalize_relative_path(directory);
    let pattern = normalize_relative_path(pattern);
    if directory == "." {
        return Ok(pattern);
    }
    pattern
        .strip_prefix(&format!("{directory}/"))
        .map(str::to_owned)
        .with_context(|| {
            format!("migrations_pattern \"{pattern}\" must start with \"{directory}/\"")
        })
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
    let content =
        fs::read(config_path).with_context(|| format!("read {}", config_path.display()))?;
    serde_json::from_slice(&content)
        .map_err(|error| anyhow!("invalid wrangler config {}: {error}", config_path.display()))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;

    use anyhow::{Context, Result, bail};
    use serde_json::{Value, json};

    use super::{
        HtmlHandling, NotFoundHandling, WranglerMigrations, WranglerModuleType, WranglerRule,
        WranglerStorage, applied_rules, collect_bindings, deploy_config_path, load_config,
        normalize_relative_path, pattern_within,
    };

    /// The message loading `path` fails with.
    fn load_error(path: &Path) -> Result<String> {
        match load_config(path) {
            Ok(_) => bail!("{} loaded", path.display()),
            Err(error) => Ok(format!("{error:#}")),
        }
    }

    #[test]
    fn collects_named_bindings_from_wrangler_like_shapes() -> Result<()> {
        let values = serde_json::from_str::<BTreeMap<String, Value>>(
            r#"{
                "vectorize": [{"binding": "INDEX", "index_name": "index"}],
                "durable_objects": {"bindings": [{"name": "ROOMS", "class_name": "Room"}]},
                "queues": {"producers": [{"binding": "EVENTS", "queue": "events"}]}
            }"#,
        )?;
        let bindings = collect_bindings(&values);
        assert_eq!(
            bindings
                .iter()
                .map(|binding| (binding.kind.as_str(), binding.name.as_str()))
                .collect::<Vec<_>>(),
            [
                ("durable_objects", "ROOMS"),
                ("queues", "EVENTS"),
                ("vectorize", "INDEX"),
            ]
        );
        Ok(())
    }

    #[test]
    fn resolves_storage_bindings_with_local_wrangler_fallbacks() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let config = directory.path().join("wrangler.json");
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
            rule(WranglerModuleType::Text, "**/*.md", false),
            rule(WranglerModuleType::Data, "**/*.txt", true),
            rule(WranglerModuleType::Text, "**/*.csv", false),
        ]);

        assert_eq!(
            applied
                .iter()
                .map(|rule| (rule.module_type, rule.globs[0].as_str()))
                .collect::<Vec<_>>(),
            [
                (WranglerModuleType::Text, "**/*.md"),
                (WranglerModuleType::Data, "**/*.txt"),
                (WranglerModuleType::Data, "**/*.bin"),
                (WranglerModuleType::CompiledWasm, "**/*.wasm"),
            ]
        );
    }

    #[test]
    fn parses_module_rules() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let config = directory.path().join("wrangler.json");
        fs::write(
            &config,
            r#"{
                "name": "demo-app",
                "main": "worker/entry.mjs",
                "rules": [
                    { "type": "Text", "globs": ["**/*.md"] },
                    { "type": "Data", "globs": ["**/*.dat"], "fallthrough": true }
                ]
            }"#,
        )?;

        let rules = load_config(&config)?.rules;

        assert_eq!(
            rules[..2]
                .iter()
                .map(|rule| (rule.module_type, rule.globs.join(","), rule.fallthrough))
                .collect::<Vec<_>>(),
            [
                (WranglerModuleType::Text, "**/*.md".to_owned(), false),
                (WranglerModuleType::Data, "**/*.dat".to_owned(), true),
            ]
        );
        Ok(())
    }

    #[test]
    fn names_the_app_after_the_top_level_worker() -> Result<()> {
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
    fn follows_the_deploy_config_from_a_nested_directory() -> Result<()> {
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
    fn reports_a_missing_deploy_config_or_generated_config() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let Err(error) = deploy_config_path(directory.path()) else {
            bail!("found a deploy config");
        };
        assert_eq!(
            error.to_string(),
            format!(
                "no .wrangler/deploy/config.json found in {} or its parent directories",
                directory.path().display()
            )
        );

        let deploy = directory.path().join(".wrangler/deploy");
        fs::create_dir_all(&deploy)?;
        fs::write(
            deploy.join("config.json"),
            r#"{ "configPath": "missing.json" }"#,
        )?;
        let Err(error) = deploy_config_path(directory.path()) else {
            bail!("found a missing generated config");
        };
        assert!(error.to_string().ends_with("missing.json"), "{error}");
        Ok(())
    }

    #[test]
    fn rejects_missing_required_fields() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let config = directory.path().join("wrangler.json");
        for (source, field) in [
            ("{}", "main"),
            (r#"{ "main": "worker.mjs" }"#, "name"),
            (
                r#"{ "name": "demo-app", "main": "worker.mjs", "assets": {} }"#,
                "assets.directory",
            ),
        ] {
            fs::write(&config, source)?;
            let message = load_error(&config)?;
            assert!(
                message.ends_with(&format!("is missing required field {field}")),
                "{message}"
            );
        }
        Ok(())
    }

    #[test]
    fn rejects_invalid_values() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let config = directory.path().join("wrangler.json");
        for (source, expected) in [
            (r#"{ "main": "#, "invalid wrangler config"),
            (
                r#"{ "name": "demo-app", "main": "worker.mjs", "assets": { "directory": "public", "html_handling": "surprising" } }"#,
                "invalid asset configuration",
            ),
            (
                r#"{ "name": "demo-app", "main": "worker.mjs", "assets": { "directory": "public", "not_found_handling": "surprising" } }"#,
                "invalid asset configuration",
            ),
            (
                r#"{ "name": "demo-app", "main": "worker.mjs", "assets": { "directory": "public", "binding": "" } }"#,
                "invalid asset configuration",
            ),
            (
                r#"{ "name": "demo-app", "main": "worker.mjs", "d1_databases": [{ "database_id": "db" }] }"#,
                "invalid d1_databases binding",
            ),
        ] {
            fs::write(&config, source)?;
            let message = load_error(&config)?;
            assert!(message.starts_with(expected), "{message}");
        }
        Ok(())
    }

    #[test]
    fn parses_values_and_resolves_paths() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let config = root.join("wrangler.json");
        fs::write(
            &config,
            r#"{
                "name": "demo-app",
                "main": "build/server/entry.mjs",
                "compatibility_flags": ["nodejs_compat"],
                "vars": { "TEXT": "value", "JSON": { "enabled": true } },
                "assets": {
                    "directory": "build/client",
                    "binding": "STATIC",
                    "html_handling": "drop-trailing-slash",
                    "not_found_handling": "single-page-application"
                }
            }"#,
        )?;

        let config = load_config(&config)?;

        assert_eq!(config.name, "demo-app");
        assert_eq!(config.main, root.join("build/server/entry.mjs"));
        let assets = config.assets.context("assets should be parsed")?;
        assert_eq!(assets.directory, root.join("build/client"));
        assert_eq!(assets.binding, "STATIC");
        assert_eq!(assets.html_handling, HtmlHandling::DropTrailingSlash);
        assert_eq!(
            assets.not_found_handling,
            NotFoundHandling::SinglePageApplication
        );
        assert_eq!(config.vars.get("TEXT"), Some(&json!("value")));
        assert_eq!(config.vars.get("JSON"), Some(&json!({ "enabled": true })));
        Ok(())
    }

    #[test]
    fn binds_assets_as_assets_by_default() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let config = directory.path().join("wrangler.json");
        fs::write(
            &config,
            r#"{
                "name": "demo-app",
                "main": "index.js",
                "assets": { "directory": "public", "html_handling": "none", "not_found_handling": "404-page" }
            }"#,
        )?;

        let assets = load_config(&config)?
            .assets
            .context("assets should be parsed")?;
        assert_eq!(assets.binding, "ASSETS");
        assert_eq!(assets.html_handling, HtmlHandling::None);
        assert_eq!(assets.not_found_handling, NotFoundHandling::Page404);
        Ok(())
    }

    #[test]
    fn requires_migrations_patterns_within_their_directory() -> Result<()> {
        assert_eq!(
            pattern_within("./drizzle", "drizzle/*/migration.sql")?,
            "*/migration.sql"
        );
        assert_eq!(pattern_within(".", "sql/*.sql")?, "sql/*.sql");
        assert!(pattern_within("db", "other/*.sql").is_err());
        Ok(())
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
