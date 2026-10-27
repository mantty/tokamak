//! Application settings from command-line options, environment variables, and
//! the tokamak Vite plugin's options.
//!
//! A setting has the same name in each source: `--<platform>-<key>`,
//! `TOKAMAK_<PLATFORM>_<KEY>`, and `<platform>.<key>`, or `--<key>`,
//! `TOKAMAK_<KEY>`, and `<key>` at the top level. Options take precedence over
//! environment variables, which take precedence over the configuration. Within
//! a source, a platform's own value takes precedence over a top-level one. The
//! CLI validates the keys it owns; every other key belongs to a platform pack,
//! which declares it.
//!
//! Plugins declare settings too, under their ID: `--<id>-<key>`,
//! `TOKAMAK_<ID>_<KEY>` and `<id>.<key>`, and for one platform
//! `--<id>-<platform>-<key>`, `TOKAMAK_<ID>_<PLATFORM>_<KEY>` and
//! `<id>.<platform>.<key>`, with the same precedence. Their options are clap
//! arguments built from the plugins' manifests, and their values are JSON of
//! the kind each setting declares.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Arg, ArgMatches, Args};
use serde_json::{Map, Value};
use tokamak_cli::{
    Platform, PlatformPackManifest, SHARED_PLATFORM_KEYS, VariableKind, is_valid_key,
};

use super::plugin_settings::{Kind, Setting, option_name, option_namespaces, option_text};
use super::plugins::Plugin;
use super::tokamak_config::{TokamakConfig, app_name_problem, valid};

/// Top-level settings from command-line options.
#[derive(Args, Clone, Debug, Default)]
pub(crate) struct TopOptions {
    /// Display name for every platform [`TOKAMAK_NAME`, `name`].
    #[arg(long)]
    pub(crate) name: Option<String>,
    /// Application identifier for every platform [`TOKAMAK_IDENTIFIER`, `identifier`].
    #[arg(long)]
    pub(crate) identifier: Option<String>,
    /// Icon for every platform [`TOKAMAK_ICON`, `icon`].
    #[arg(long, value_name = "PATH")]
    pub(crate) icon: Option<String>,
    /// App version [`TOKAMAK_VERSION`, `version`].
    #[arg(long)]
    pub(crate) version: Option<String>,
}

/// Values from `--<namespace>-<key>` options: platforms' text, by namespace
/// then key, and plugins' JSON, by option name without `--`.
#[derive(Clone, Debug, Default)]
pub(crate) struct NamespacedOptions {
    platforms: BTreeMap<String, BTreeMap<String, String>>,
    plugins: BTreeMap<String, Value>,
}

impl NamespacedOptions {
    fn value(&self, namespace: &str, key: &str) -> Result<Option<String>> {
        let value = self
            .platforms
            .get(namespace)
            .and_then(|values| values.get(key));
        value
            .map(|value| valid(&format!("--{namespace}-{key}"), value.clone()))
            .transpose()
    }

    fn keys(&self, namespace: &str) -> impl Iterator<Item = &String> {
        self.platforms
            .get(namespace)
            .into_iter()
            .flat_map(BTreeMap::keys)
    }

    /// These options with the values `matches` holds for the options
    /// [`plugin_arguments`] adds.
    pub(crate) fn with_plugin_options(mut self, matches: &ArgMatches, plugins: &[Plugin]) -> Self {
        for (plugin, key, setting) in declared_settings(plugins) {
            let names =
                option_namespaces().map(|namespace| option_name(&plugin.id, namespace, key));
            for name in names.filter(|name| matches.contains_id(name)) {
                let value = setting.kind.option_value(matches, &name);
                self.plugins.insert(name, value);
            }
        }
        self
    }
}

/// Where settings come from.
pub(crate) struct Sources<'a> {
    pub(crate) top: &'a TopOptions,
    pub(crate) options: &'a NamespacedOptions,
    /// The `tokamak()` options, once the project's command has reported them.
    pub(crate) config: Option<&'a TokamakConfig>,
    /// Reads an environment variable.
    pub(crate) environment: &'a dyn Fn(&str) -> Result<Option<String>>,
    /// Base for relative paths from options and environment variables.
    pub(crate) current_dir: &'a Path,
}

impl<'a> Sources<'a> {
    /// Sources that read the process environment.
    pub(crate) fn new(
        top: &'a TopOptions,
        options: &'a NamespacedOptions,
        config: Option<&'a TokamakConfig>,
        current_dir: &'a Path,
    ) -> Self {
        Self {
            top,
            options,
            config,
            environment: &process_environment,
            current_dir,
        }
    }
}

/// The value of the environment variable `name`, which must be UTF-8.
pub(crate) fn process_environment(name: &str) -> Result<Option<String>> {
    match env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => bail!("{name} must contain valid UTF-8"),
    }
}

/// Settings for one platform.
#[derive(Debug)]
pub(crate) struct PlatformSettings {
    pub(crate) name: Option<String>,
    pub(crate) identifier: Option<String>,
    /// The platform pack's variables and the icon path, as its entrypoint's
    /// environment.
    pub(crate) pack_environment: BTreeMap<String, OsString>,
}

/// Split platforms' `--<namespace>-<key> <value>` and
/// `--<namespace>-<key>=<value>` options out of `arguments`, which stop at
/// `--`. Other arguments keep their order. A separate value may not start
/// with `-`, as with other options.
pub(crate) fn split_namespaced_options(
    arguments: Vec<OsString>,
) -> Result<(Vec<OsString>, NamespacedOptions)> {
    let mut remaining = Vec::new();
    let mut options = NamespacedOptions::default();
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        if argument == "--" {
            remaining.push(argument);
            remaining.extend(arguments);
            break;
        }
        let text = argument.to_string_lossy();
        let Some((namespace, key, value)) = namespaced_option(&text) else {
            remaining.push(argument);
            continue;
        };
        let option = format!("--{namespace}-{key}");
        if matches!(text, Cow::Owned(_)) {
            bail!("{option} must be valid UTF-8");
        }
        if !is_valid_key(&key) {
            bail!("{option} is not a valid option; keys are lowercase words joined by hyphens");
        }
        let value = match value {
            Some(value) => value,
            None => separate_value(arguments.next(), &option)?,
        };
        let values = options.platforms.entry(namespace.to_owned()).or_default();
        if values.insert(key, value).is_some() {
            bail!("{option} is given more than once");
        }
    }
    Ok((remaining, options))
}

/// The namespace, key, and inline value of a platform's option.
fn namespaced_option(argument: &str) -> Option<(&'static str, String, Option<String>)> {
    let option = argument.strip_prefix("--")?;
    let (name, value) = match option.split_once('=') {
        Some((name, value)) => (name, Some(value.to_owned())),
        None => (option, None),
    };
    option_namespaces().flatten().find_map(|namespace| {
        let key = name.strip_prefix(namespace)?.strip_prefix('-')?;
        Some((namespace, key.to_owned(), value.clone()))
    })
}

fn separate_value(value: Option<OsString>, option: &str) -> Result<String> {
    let Some(value) = value.filter(|value| !value.to_string_lossy().starts_with('-')) else {
        bail!("{option} requires a value");
    };
    match value.into_string() {
        Ok(value) => Ok(value),
        Err(_) => bail!("{option} must be valid UTF-8"),
    }
}

/// The app version: `--version`, `TOKAMAK_VERSION`, or `version`.
pub(crate) fn version(sources: &Sources<'_>) -> Result<Option<String>> {
    let option = top_option(sources, "version", sources.top.version.as_ref())?;
    Ok(option.or_else(|| sources.config.and_then(|config| config.version.clone())))
}

/// Resolve `platform`'s settings, rejecting keys its pack does not declare.
pub(crate) fn resolve(
    sources: &Sources<'_>,
    platform: Platform,
    manifest: &PlatformPackManifest,
) -> Result<PlatformSettings> {
    let configured = sources
        .config
        .map(|config| config.for_platform(platform))
        .unwrap_or_default();
    let name = shared_value(sources, platform, "name", sources.top.name.as_ref())?;
    if let Some(problem) = name.as_deref().and_then(app_name_problem) {
        bail!("{} name {problem}", platform.display_name());
    }
    let identifier = shared_value(
        sources,
        platform,
        "identifier",
        sources.top.identifier.as_ref(),
    )?;
    let icon = shared_value(sources, platform, "icon", sources.top.icon.as_ref())?
        .map(|icon| sources.current_dir.join(icon))
        .or(configured.icon);
    let mut pack_environment = pack_environment(sources, platform, manifest, &configured.pack)?;
    if let Some(icon) = icon {
        let name = environment_name(Some(platform.namespace()), "icon");
        pack_environment.insert(name, icon.into_os_string());
    }
    Ok(PlatformSettings {
        name: name.or(configured.name),
        identifier: identifier.or(configured.identifier),
        pack_environment,
    })
}

/// A top-level key's option or environment value.
fn top_option(sources: &Sources<'_>, key: &str, option: Option<&String>) -> Result<Option<String>> {
    match option {
        Some(value) => valid(&format!("--{key}"), value.clone()).map(Some),
        None => environment_value(sources, &environment_name(None, key)),
    }
}

/// A shared key's option or environment value: the platform's own, then the top-level one.
fn shared_value(
    sources: &Sources<'_>,
    platform: Platform,
    key: &str,
    top_option: Option<&String>,
) -> Result<Option<String>> {
    let namespace = platform.namespace();
    if let Some(value) = sources.options.value(namespace, key)? {
        return Ok(Some(value));
    }
    if let Some(value) = top_option {
        return valid(&format!("--{key}"), value.clone()).map(Some);
    }
    match environment_value(sources, &environment_name(Some(namespace), key))? {
        Some(value) => Ok(Some(value)),
        None => environment_value(sources, &environment_name(None, key)),
    }
}

fn environment_value(sources: &Sources<'_>, name: &str) -> Result<Option<String>> {
    (sources.environment)(name)?
        .map(|value| valid(name, value))
        .transpose()
}

/// The platform pack's variables, keyed by their environment variable names;
/// `configured` holds the `tokamak()` options' values for the platform.
fn pack_environment(
    sources: &Sources<'_>,
    platform: Platform,
    manifest: &PlatformPackManifest,
    configured: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, OsString>> {
    reject_undeclared_keys(sources, platform, manifest, configured)?;
    let namespace = platform.namespace();
    let mut environment = BTreeMap::new();
    for (key, variable) in &manifest.variables {
        let name = environment_name(Some(namespace), key);
        let Some((value, directory)) = pack_value(sources, namespace, key, &name, configured)?
        else {
            continue;
        };
        let value = match variable.kind {
            VariableKind::String => OsString::from(value),
            VariableKind::Path => directory.join(value).into_os_string(),
        };
        environment.insert(name, value);
    }
    Ok(environment)
}

/// A pack variable's value and the directory a relative path is relative to.
fn pack_value(
    sources: &Sources<'_>,
    namespace: &str,
    key: &str,
    environment_name: &str,
    configured: &BTreeMap<String, String>,
) -> Result<Option<(String, PathBuf)>> {
    let from_current_dir = |value| (value, sources.current_dir.to_path_buf());
    if let Some(value) = sources.options.value(namespace, key)? {
        return Ok(Some(from_current_dir(value)));
    }
    if let Some(value) = environment_value(sources, environment_name)? {
        return Ok(Some(from_current_dir(value)));
    }
    let configured = configured.get(key).zip(sources.config);
    Ok(configured.map(|(value, config)| (value.clone(), config.root.clone())))
}

fn reject_undeclared_keys(
    sources: &Sources<'_>,
    platform: Platform,
    manifest: &PlatformPackManifest,
    configured: &BTreeMap<String, String>,
) -> Result<()> {
    let namespace = platform.namespace();
    let declared = |key: &str| manifest.variables.contains_key(key);
    let shared = |key: &str| {
        SHARED_PLATFORM_KEYS
            .iter()
            .any(|(shared, _, _)| *shared == key)
    };
    if let Some(key) = sources
        .options
        .keys(namespace)
        .find(|key| !shared(key) && !declared(key))
    {
        bail!(
            "unknown option --{namespace}-{key}; {}",
            accepted_options(platform, manifest)
        );
    }
    if let Some(key) = configured.keys().find(|key| !declared(key)) {
        bail!(
            "unknown key {namespace}.{key} in the tokamak() options; {}",
            accepted_options(platform, manifest)
        );
    }
    Ok(())
}

fn accepted_options(platform: Platform, manifest: &PlatformPackManifest) -> String {
    let namespace = platform.namespace();
    let options = SHARED_PLATFORM_KEYS
        .iter()
        .map(|(key, _, _)| *key)
        .chain(manifest.variables.keys().map(String::as_str))
        .map(|key| format!("--{namespace}-{key}"))
        .collect::<Vec<_>>();
    format!("{} accepts {}", platform.display_name(), options.join(", "))
}

/// Each plugin's settings for `platform`, by plugin ID then key, as JSON of
/// their kinds with paths made absolute and defaults applied, rejecting keys
/// and plugins that are not declared.
pub(crate) fn resolve_plugins(
    sources: &Sources<'_>,
    platform: Platform,
    plugins: &[Plugin],
) -> Result<BTreeMap<String, Map<String, Value>>> {
    if let Some(id) = sources
        .config
        .into_iter()
        .flat_map(|config| config.plugins.keys())
        .find(|id| !plugins.iter().any(|plugin| &plugin.id == *id))
    {
        bail!("unknown key {id} in the tokamak() options; no plugin of the app has that ID");
    }
    let namespace = platform.namespace();
    let mut resolved = BTreeMap::new();
    for plugin in plugins {
        reject_undeclared_plugin_keys(sources, plugin)?;
        let mut values = Map::new();
        for (key, setting) in &plugin.settings {
            let Some((value, directory)) =
                resolve_setting(sources, &plugin.id, namespace, key, setting)?
            else {
                continue;
            };
            let value = setting
                .kind
                .resolve_paths(value, &directory)
                .with_context(|| format!("plugin {} setting {key}", plugin.id))?;
            values.insert(key.clone(), value);
        }
        resolved.insert(plugin.id.clone(), values);
    }
    Ok(resolved)
}

/// Plugin `id`'s value of `key` on the platform `namespace`, and the directory
/// a relative path is relative to: the platform's own option, then the
/// plugin's, then the environment variables and configuration likewise, then
/// the setting's default.
fn resolve_setting(
    sources: &Sources<'_>,
    id: &str,
    namespace: &str,
    key: &str,
    setting: &Setting,
) -> Result<Option<(Value, PathBuf)>> {
    let in_current_dir = |value: Value| Some((value, sources.current_dir.to_path_buf()));
    let option = [Some(namespace), None].into_iter().find_map(|namespace| {
        sources
            .options
            .plugins
            .get(&option_name(id, namespace, key))
    });
    if let Some(value) = option {
        return Ok(in_current_dir(value.clone()));
    }
    let platform_namespace = format!("{id}-{namespace}");
    for name in [
        environment_name(Some(&platform_namespace), key),
        environment_name(Some(id), key),
    ] {
        if let Some(text) = (sources.environment)(&name)? {
            let value = setting
                .kind
                .parse(&text)
                .map_err(|problem| anyhow!("{name} {problem}"))?;
            return Ok(in_current_dir(value));
        }
    }
    if let Some(configured) = configured_plugin_value(sources, id, namespace, key, &setting.kind)? {
        return Ok(Some(configured));
    }
    let root = sources
        .config
        .map_or(sources.current_dir, |config| &config.root);
    Ok(setting
        .default
        .clone()
        .map(|value| (value, root.to_path_buf())))
}

/// Plugin `id`'s configured value of `key` on the platform `namespace`, of
/// `kind`, and the Vite root.
fn configured_plugin_value(
    sources: &Sources<'_>,
    id: &str,
    namespace: &str,
    key: &str,
    kind: &Kind,
) -> Result<Option<(Value, PathBuf)>> {
    let Some((configured, config)) = sources
        .config
        .and_then(|config| Some((config.plugins.get(id)?, config)))
    else {
        return Ok(None);
    };
    let platform_value = configured
        .platforms
        .get(namespace)
        .and_then(|values| values.get(key))
        .map(|value| (value, format!("{id}.{namespace}.{key}")));
    let Some((value, field)) =
        platform_value.or_else(|| Some((configured.values.get(key)?, format!("{id}.{key}"))))
    else {
        return Ok(None);
    };
    kind.check(value)
        .map_err(|problem| anyhow!("invalid tokamak() options: {field} {problem}"))?;
    Ok(Some((value.clone(), config.root.clone())))
}

/// Fails on a configured key the plugin does not declare, for every platform
/// or one. clap rejects options the plugin does not declare.
fn reject_undeclared_plugin_keys(sources: &Sources<'_>, plugin: &Plugin) -> Result<()> {
    let Some(configured) = sources
        .config
        .and_then(|config| config.plugins.get(&plugin.id))
    else {
        return Ok(());
    };
    let id = &plugin.id;
    let declared = |key: &&String| plugin.settings.contains_key(*key);
    let accepted = || {
        let keys = plugin.settings.keys().map(|key| format!("{id}.{key}"));
        let keys = keys.collect::<Vec<_>>();
        if keys.is_empty() {
            format!("plugin {id} has no settings")
        } else {
            format!("plugin {id} accepts {}", keys.join(", "))
        }
    };
    if let Some(key) = configured.values.keys().find(|key| !declared(key)) {
        bail!(
            "unknown key {id}.{key} in the tokamak() options; {}",
            accepted()
        );
    }
    for (namespace, values) in &configured.platforms {
        if let Some(key) = values.keys().find(|key| !declared(key)) {
            bail!(
                "unknown key {id}.{namespace}.{key} in the tokamak() options; {}",
                accepted()
            );
        }
    }
    Ok(())
}

/// Each plugin's settings, with the plugin.
fn declared_settings(plugins: &[Plugin]) -> impl Iterator<Item = (&Plugin, &String, &Setting)> {
    plugins.iter().flat_map(|plugin| {
        plugin
            .settings
            .iter()
            .map(move |(key, setting)| (plugin, key, setting))
    })
}

/// The options that set the plugins' settings, for clap to parse:
/// `--<id>-<key>`, listed in help with its default, and each platform's
/// `--<id>-<platform>-<key>`, hidden. Resolving a setting applies its default.
pub(crate) fn plugin_arguments(plugins: &[Plugin]) -> Vec<Arg> {
    declared_settings(plugins)
        .flat_map(|(plugin, key, setting)| {
            let id = &plugin.id;
            let default = setting
                .default
                .as_ref()
                .map(|default| format!(" [default: {}]", option_text(default)))
                .unwrap_or_default();
            let help = format!(
                "{}{default} [`{}`, `{id}.{key}`]",
                setting.description,
                environment_name(Some(id), key)
            );
            option_namespaces().map(move |namespace| {
                let argument = setting.kind.argument(option_name(id, namespace, key));
                match namespace {
                    None => argument.help(help.clone()).help_heading("Plugin options"),
                    Some(_) => argument.hide(true),
                }
            })
        })
        .collect()
}

/// `TOKAMAK_<PLATFORM>_<KEY>`, or `TOKAMAK_<KEY>` for a top-level key.
fn environment_name(namespace: Option<&str>, key: &str) -> String {
    let name = match namespace {
        Some(namespace) => format!("tokamak-{namespace}-{key}"),
        None => format!("tokamak-{key}"),
    };
    name.replace('-', "_").to_ascii_uppercase()
}

/// `--help` text on the options that set one platform's value of a plugin's
/// setting, if a plugin declares one.
pub(crate) fn plugins_help(plugins: &[Plugin]) -> Option<String> {
    plugins
        .iter()
        .any(|plugin| !plugin.settings.is_empty())
        .then(|| {
            "Plugin options set one platform's value as --<id>-<platform>-<key>, \
         TOKAMAK_<ID>_<PLATFORM>_<KEY>, or <id>.<platform>.<key> in the tokamak() options."
                .to_owned()
        })
}

/// `--help` text listing a platform's options, noting when its pack cannot be loaded.
pub(crate) fn platform_help(
    platform: Platform,
    manifest: std::result::Result<&PlatformPackManifest, String>,
) -> String {
    let namespace = platform.namespace();
    let mut help = format!(
        "{} options (also TOKAMAK_{}_<KEY>, or {namespace}.<key> in the tokamak() options):\n",
        platform.display_name(),
        namespace.to_ascii_uppercase()
    );
    let pack_options = manifest.as_ref().ok().into_iter().flat_map(|manifest| {
        manifest
            .variables
            .iter()
            .map(|(key, variable)| (key.as_str(), variable.kind, variable.description.as_str()))
    });
    let rows = SHARED_PLATFORM_KEYS
        .into_iter()
        .chain(pack_options)
        .map(|(key, kind, description)| {
            let value = match kind {
                VariableKind::String => "<VALUE>",
                VariableKind::Path => "<PATH>",
            };
            (format!("--{namespace}-{key} {value}"), description)
        })
        .collect::<Vec<_>>();
    let width = rows
        .iter()
        .map(|(option, _)| option.len())
        .max()
        .unwrap_or(0);
    for (option, description) in rows {
        let _ = writeln!(help, "  {option:width$}  {description}");
    }
    if let Err(error) = manifest {
        let _ = writeln!(
            help,
            "  The platform pack's options are unavailable: {error}"
        );
    }
    help
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::fmt::Debug;
    use std::path::Path;

    use anyhow::Result;
    use serde_json::{Value, json};

    use crate::tokamak_config::{TokamakConfig, parse_config};
    use tokamak_cli::{PackVariable, Platform, PlatformPackManifest, Target, VariableKind};

    use super::{
        NamespacedOptions, PlatformSettings, Plugin, Sources, TopOptions, environment_name,
        platform_help, plugin_arguments, plugins_help, resolve, resolve_plugins,
        split_namespaced_options, version,
    };

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    /// `relative` in `directory`, as a pack receives a path setting.
    fn joined(directory: &str, relative: &str) -> OsString {
        Path::new(directory).join(relative).into_os_string()
    }

    fn namespaced_options(values: &[&str]) -> Result<NamespacedOptions> {
        Ok(split_namespaced_options(arguments(values))?.1)
    }

    fn manifest(target: Target) -> PlatformPackManifest {
        let variable = |kind, description: &str| PackVariable {
            kind,
            description: description.to_owned(),
        };
        PlatformPackManifest {
            tokamak_version: "0.1.0".to_owned(),
            target,
            variables: BTreeMap::from([
                (
                    "plist".to_owned(),
                    variable(VariableKind::Path, "User plist"),
                ),
                (
                    "team-id".to_owned(),
                    variable(VariableKind::String, "Signing team"),
                ),
            ]),
            plugin_keys: BTreeMap::new(),
        }
    }

    /// `config` as the plugin reports it from the Vite root `/config`.
    fn configured(config: &Value) -> Result<Option<TokamakConfig>> {
        parse_config(Path::new("/config"), config).map(Some)
    }

    struct Fixture {
        top: TopOptions,
        options: NamespacedOptions,
        config: Option<TokamakConfig>,
        environment: BTreeMap<String, String>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                top: TopOptions::default(),
                options: NamespacedOptions::default(),
                config: None,
                environment: BTreeMap::new(),
            }
        }

        fn with<T>(&self, run: impl FnOnce(&Sources<'_>) -> Result<T>) -> Result<T> {
            let environment = |name: &str| Ok(self.environment.get(name).cloned());
            run(&Sources {
                top: &self.top,
                options: &self.options,
                config: self.config.as_ref(),
                environment: &environment,
                current_dir: Path::new("/work"),
            })
        }

        fn resolve(&self, platform: Platform) -> Result<PlatformSettings> {
            let manifest = manifest(platform.default_target()?);
            self.with(|sources| resolve(sources, platform, &manifest))
        }

        /// Asserts what `read` takes from `platform`'s settings.
        #[track_caller]
        fn assert_resolves<T: Debug + PartialEq>(
            &self,
            platform: Platform,
            read: impl FnOnce(PlatformSettings) -> T,
            expected: &T,
        ) -> Result<()> {
            assert_eq!(&read(self.resolve(platform)?), expected, "{platform:?}");
            Ok(())
        }

        /// Asserts that resolving `platform` fails with an error `accepts` holds for.
        #[track_caller]
        fn assert_rejects(&self, platform: Platform, accepts: impl FnOnce(&str) -> bool) {
            let error = self.resolve(platform).err().map(|error| error.to_string());
            assert!(
                error.as_deref().is_some_and(accepts),
                "{platform:?}: {error:?}"
            );
        }
    }

    #[test]
    fn splits_platform_options_from_other_arguments() -> Result<()> {
        let (remaining, options) = split_namespaced_options(arguments(&[
            "tok",
            "build",
            "ios",
            "--ios-plist",
            "Info.plist",
            "--project",
            "app",
            "--android-manifest=AndroidManifest.xml",
            "--iosx",
            "--",
            "--ios-team-id",
            "TEAM",
        ]))?;

        assert_eq!(
            remaining,
            arguments(&[
                "tok",
                "build",
                "ios",
                "--project",
                "app",
                "--iosx",
                "--",
                "--ios-team-id",
                "TEAM"
            ])
        );
        let assert_value = |namespace, key, expected| -> Result<()> {
            let value = options.value(namespace, key)?;
            assert_eq!(value.as_deref(), expected, "--{namespace}-{key}");
            Ok(())
        };
        assert_value("ios", "plist", Some("Info.plist"))?;
        assert_value("android", "manifest", Some("AndroidManifest.xml"))?;
        assert_value("ios", "team-id", None)?;
        Ok(())
    }

    #[test]
    fn keeps_equals_signs_and_leading_hyphens_in_inline_values() -> Result<()> {
        let options = namespaced_options(&["--ios-team-id=a=b", "--ios-plist=-Info.plist"])?;
        assert_eq!(options.value("ios", "team-id")?.as_deref(), Some("a=b"));
        assert_eq!(
            options.value("ios", "plist")?.as_deref(),
            Some("-Info.plist")
        );
        Ok(())
    }

    #[test]
    fn rejects_malformed_platform_options() {
        for (values, message) in [
            (&["--ios-plist"][..], "--ios-plist requires a value"),
            (&["--ios-plist", "--"][..], "--ios-plist requires a value"),
            (
                &["--ios-plist", "--help"][..],
                "--ios-plist requires a value",
            ),
            (&["--ios-", "x"][..], "--ios- is not a valid option"),
            (
                &["--ios-plist=a", "--ios-plist=b"][..],
                "--ios-plist is given more than once",
            ),
            (&["--ios-Plist=a"][..], "--ios-Plist is not a valid option"),
        ] {
            let error = split_namespaced_options(arguments(values)).map(|_| ());
            assert!(
                error
                    .as_ref()
                    .is_err_and(|error| error.to_string().starts_with(message)),
                "{values:?}: {error:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_platform_options_that_are_not_utf8() {
        use std::os::unix::ffi::OsStringExt;

        let invalid = OsString::from_vec(b"--ios-plist=\xff".to_vec());
        let error = split_namespaced_options(vec![invalid]).map(|_| ());
        assert!(error.is_err_and(|error| error.to_string() == "--ios-plist must be valid UTF-8"));

        let invalid = OsString::from_vec(b"\xff".to_vec());
        let error =
            split_namespaced_options(vec![OsString::from("--ios-plist"), invalid]).map(|_| ());
        assert!(error.is_err_and(|error| error.to_string() == "--ios-plist must be valid UTF-8"));
    }

    #[test]
    fn names_environment_variables_after_their_keys() {
        assert_eq!(
            environment_name(Some("ios"), "team-id"),
            "TOKAMAK_IOS_TEAM_ID"
        );
        assert_eq!(environment_name(None, "identifier"), "TOKAMAK_IDENTIFIER");
    }

    #[test]
    fn shared_keys_prefer_options_then_environment_then_configuration() -> Result<()> {
        let mut fixture = Fixture::new();
        fixture.config = configured(&json!({
            "identifier": "com.config.top",
            "ios": { "identifier": "com.config.ios" },
        }))?;
        let identifier = |settings: PlatformSettings| settings.identifier;
        fixture.assert_resolves(Platform::Ios, identifier, &Some("com.config.ios".into()))?;
        fixture.assert_resolves(Platform::Macos, identifier, &Some("com.config.top".into()))?;

        fixture
            .environment
            .insert("TOKAMAK_IDENTIFIER".to_owned(), "com.env.top".to_owned());
        fixture.assert_resolves(Platform::Ios, identifier, &Some("com.env.top".into()))?;
        fixture.environment.insert(
            "TOKAMAK_IOS_IDENTIFIER".to_owned(),
            "com.env.ios".to_owned(),
        );
        fixture.assert_resolves(
            Platform::IosSimulator,
            identifier,
            &Some("com.env.ios".into()),
        )?;

        fixture.top.identifier = Some("com.option.top".to_owned());
        fixture.assert_resolves(Platform::Ios, identifier, &Some("com.option.top".into()))?;
        fixture.options = namespaced_options(&["--ios-identifier", "com.option.ios"])?;
        fixture.assert_resolves(Platform::Ios, identifier, &Some("com.option.ios".into()))?;
        fixture.assert_resolves(Platform::Macos, identifier, &Some("com.option.top".into()))?;
        Ok(())
    }

    #[test]
    fn passes_the_icon_to_the_pack_as_a_path() -> Result<()> {
        let mut fixture = Fixture::new();
        let icon =
            |name| move |settings: PlatformSettings| settings.pack_environment.get(name).cloned();
        let macos_icon = icon("TOKAMAK_MACOS_ICON");
        fixture.assert_resolves(Platform::Macos, macos_icon, &None)?;
        fixture.config = configured(&json!({ "icon": "../AppIcon.icon" }))?;
        fixture.assert_resolves(
            Platform::Macos,
            macos_icon,
            &Some(joined("/config", "../AppIcon.icon")),
        )?;
        fixture
            .environment
            .insert("TOKAMAK_ICON".to_owned(), "icons/App.icon".to_owned());
        fixture.assert_resolves(
            Platform::Macos,
            macos_icon,
            &Some(joined("/work", "icons/App.icon")),
        )?;
        fixture.top.icon = Some("top/App.icon".to_owned());
        fixture.options = namespaced_options(&["--macos-icon", "macos/App.icon"])?;
        fixture.assert_resolves(
            Platform::Macos,
            macos_icon,
            &Some(joined("/work", "macos/App.icon")),
        )?;
        fixture.assert_resolves(
            Platform::IosSimulator,
            icon("TOKAMAK_IOS_ICON"),
            &Some(joined("/work", "top/App.icon")),
        )?;
        Ok(())
    }

    #[test]
    fn rejects_invalid_option_and_environment_names() {
        let mut fixture = Fixture::new();
        fixture.top.name = Some("!!!".to_owned());
        assert!(fixture.resolve(Platform::Macos).is_err_and(|error| {
            error.to_string() == "macOS name must contain an ASCII letter or digit"
        }));
        fixture.top.name = None;
        fixture
            .environment
            .insert("TOKAMAK_NAME".to_owned(), " App".to_owned());
        assert!(
            fixture
                .resolve(Platform::Macos)
                .is_err_and(|error| error.to_string().starts_with("TOKAMAK_NAME must be"))
        );
    }

    #[test]
    fn pack_variables_prefer_options_then_environment_then_configuration() -> Result<()> {
        let mut fixture = Fixture::new();
        fixture.config = configured(&json!({
            "ios": { "plist": "native/Info.plist", "team-id": "CONFIG" },
        }))?;
        let environment = |settings: PlatformSettings| settings.pack_environment;
        fixture.assert_resolves(
            Platform::Ios,
            environment,
            &BTreeMap::from([
                (
                    "TOKAMAK_IOS_PLIST".to_owned(),
                    joined("/config", "native/Info.plist"),
                ),
                ("TOKAMAK_IOS_TEAM_ID".to_owned(), OsString::from("CONFIG")),
            ]),
        )?;

        fixture
            .environment
            .insert("TOKAMAK_IOS_PLIST".to_owned(), "env/Info.plist".to_owned());
        fixture.options = namespaced_options(&["--ios-team-id", "OPTION"])?;
        fixture.assert_resolves(
            Platform::IosSimulator,
            environment,
            &BTreeMap::from([
                (
                    "TOKAMAK_IOS_PLIST".to_owned(),
                    joined("/work", "env/Info.plist"),
                ),
                ("TOKAMAK_IOS_TEAM_ID".to_owned(), OsString::from("OPTION")),
            ]),
        )?;
        fixture.assert_resolves(Platform::Macos, environment, &BTreeMap::new())?;
        Ok(())
    }

    /// `relative` in `directory`, as a plugin receives a path setting.
    fn path(directory: &str, relative: &str) -> String {
        Path::new(directory)
            .join(relative)
            .to_string_lossy()
            .into_owned()
    }

    /// A plugin `alerts` with a setting of each kind.
    fn alerts() -> Result<Vec<Plugin>> {
        Ok(vec![serde_json::from_value(json!({
            "schemaVersion": 1,
            "id": "alerts",
            "settings": {
                "sounds": { "kind": "path", "description": "Sounds" },
                "volume": { "kind": "integer", "description": "Volume" },
                "gain": { "kind": "number", "description": "Gain" },
                "label": { "kind": "string", "description": "Label" },
                "loud": { "kind": "boolean", "description": "Loud", "default": false },
                "tone": {
                    "kind": "enum",
                    "values": ["soft", "sharp"],
                    "description": "Tone",
                    "default": "soft",
                },
                "names": { "kind": "list", "of": "string", "description": "Names" },
                "purposes": { "kind": "map", "of": "string", "description": "Purposes" },
            },
            "build": "build.mjs",
        }))?])
    }

    /// The options tok reads from `values`: the platforms' split out, and the
    /// plugins' parsed by clap.
    fn parsed_options(values: &[&str], plugins: &[Plugin]) -> Result<NamespacedOptions> {
        let mut arguments = arguments(&["tok"]);
        arguments.extend(values.iter().map(OsString::from));
        let (remaining, options) = split_namespaced_options(arguments)?;
        let matches = clap::Command::new("tok")
            .args(plugin_arguments(plugins))
            .try_get_matches_from(remaining)?;
        Ok(options.with_plugin_options(&matches, plugins))
    }

    impl Fixture {
        /// The alerts plugin's settings on `platform`.
        fn plugin_settings(&self, platform: Platform) -> Result<Value> {
            let plugins = alerts()?;
            let mut resolved = self.with(|sources| resolve_plugins(sources, platform, &plugins))?;
            Ok(Value::Object(resolved.remove("alerts").unwrap_or_default()))
        }

        /// Asserts the alerts plugin's settings on `platform`.
        #[track_caller]
        fn assert_plugin_settings(&self, platform: Platform, expected: &Value) -> Result<()> {
            assert_eq!(&self.plugin_settings(platform)?, expected, "{platform:?}");
            Ok(())
        }
    }

    /// The alerts plugin's defaults, with `values`.
    fn with_defaults(values: Value) -> Value {
        let mut settings = json!({ "loud": false, "tone": "soft" });
        if let (Some(settings), Value::Object(values)) = (settings.as_object_mut(), values) {
            settings.extend(values);
        }
        settings
    }

    #[test]
    fn leaves_plugin_options_to_clap() -> Result<()> {
        let (remaining, options) = split_namespaced_options(arguments(&[
            "tok",
            "build",
            "--alerts-ios-sounds=ios",
            "--ios-plist",
            "Info.plist",
        ]))?;

        assert_eq!(
            remaining,
            arguments(&["tok", "build", "--alerts-ios-sounds=ios"])
        );
        assert_eq!(
            options.value("ios", "plist")?.as_deref(),
            Some("Info.plist")
        );
        Ok(())
    }

    #[test]
    fn reads_each_kind_of_plugin_setting_from_options() -> Result<()> {
        let mut fixture = Fixture::new();
        fixture.options = parsed_options(
            &[
                "--alerts-sounds",
                "sounds",
                "--alerts-volume",
                "-3",
                "--alerts-gain=0.5",
                "--alerts-label",
                "Alarm",
                "--alerts-loud",
                "--alerts-tone",
                "sharp",
                "--alerts-names",
                "a,b",
                "--alerts-names",
                "c",
                "--alerts-purposes",
                "Run=Tracks runs,Map=Shows a=b",
                "--alerts-purposes=Walk=Counts steps",
            ],
            &alerts()?,
        )?;

        assert_eq!(
            fixture.plugin_settings(Platform::Ios)?,
            json!({
                "sounds": path("/work", "sounds"),
                "volume": -3,
                "gain": 0.5,
                "label": "Alarm",
                "loud": true,
                "tone": "sharp",
                "names": ["a", "b", "c"],
                "purposes": { "Run": "Tracks runs", "Map": "Shows a=b", "Walk": "Counts steps" },
            })
        );
        Ok(())
    }

    #[test]
    fn reads_each_kind_of_plugin_setting_from_environment_variables() -> Result<()> {
        let mut fixture = Fixture::new();
        fixture.environment.extend(
            [
                ("TOKAMAK_ALERTS_SOUNDS", "sounds"),
                ("TOKAMAK_ALERTS_VOLUME", "3"),
                ("TOKAMAK_ALERTS_GAIN", "1e-1"),
                ("TOKAMAK_ALERTS_LABEL", "Alarm"),
                ("TOKAMAK_ALERTS_LOUD", "true"),
                ("TOKAMAK_ALERTS_TONE", "sharp"),
                ("TOKAMAK_ALERTS_NAMES", "a,b"),
                ("TOKAMAK_ALERTS_PURPOSES", "Run=Tracks runs,Map=Maps"),
            ]
            .map(|(name, value)| (name.to_owned(), value.to_owned())),
        );

        assert_eq!(
            fixture.plugin_settings(Platform::Android)?,
            json!({
                "sounds": path("/work", "sounds"),
                "volume": 3,
                "gain": 0.1,
                "label": "Alarm",
                "loud": true,
                "tone": "sharp",
                "names": ["a", "b"],
                "purposes": { "Run": "Tracks runs", "Map": "Maps" },
            })
        );
        Ok(())
    }

    #[test]
    fn reads_each_kind_of_plugin_setting_from_the_configuration() -> Result<()> {
        let mut fixture = Fixture::new();
        fixture.config = configured(&json!({
            "alerts": {
                "sounds": "sounds",
                "volume": 3,
                "gain": 1,
                "label": "Alarm",
                "loud": true,
                "names": ["a", "b,c"],
                "purposes": { "Run": "Tracks, counts" },
                "ios": { "tone": "sharp", "names": [] },
            },
        }))?;

        let shared = json!({
            "sounds": path("/config", "sounds"),
            "volume": 3,
            "gain": 1,
            "label": "Alarm",
            "loud": true,
            "tone": "soft",
            "names": ["a", "b,c"],
            "purposes": { "Run": "Tracks, counts" },
        });
        assert_eq!(fixture.plugin_settings(Platform::Macos)?, shared);
        let mut ios = shared;
        ios["tone"] = json!("sharp");
        ios["names"] = json!([]);
        assert_eq!(fixture.plugin_settings(Platform::IosSimulator)?, ios);
        Ok(())
    }

    #[test]
    fn applies_plugin_setting_defaults_last() -> Result<()> {
        let mut fixture = Fixture::new();
        fixture.assert_plugin_settings(Platform::Ios, &with_defaults(json!({})))?;

        fixture.config = configured(&json!({ "alerts": { "loud": true } }))?;
        fixture.options = parsed_options(&["--alerts-ios-loud=false"], &alerts()?)?;
        fixture.assert_plugin_settings(Platform::Ios, &with_defaults(json!({ "loud": false })))?;
        fixture
            .assert_plugin_settings(Platform::Android, &with_defaults(json!({ "loud": true })))?;
        Ok(())
    }

    #[test]
    fn applies_list_map_and_path_defaults_as_declared() -> Result<()> {
        let plugins: Vec<Plugin> = vec![serde_json::from_value(json!({
            "schemaVersion": 1,
            "id": "kit",
            "settings": {
                "names": { "kind": "list", "of": "string", "description": "Names", "default": ["a,b"] },
                "purposes": {
                    "kind": "map",
                    "of": "string",
                    "description": "Purposes",
                    "default": { "Run": "Tracks, counts" },
                },
                "folder": { "kind": "path", "description": "Folder", "default": "assets" },
            },
            "build": "build.mjs",
        }))?];
        let mut fixture = Fixture::new();
        fixture.options = parsed_options(&[], &plugins)?;
        let resolve = |fixture: &Fixture| -> Result<Value> {
            let mut resolved =
                fixture.with(|sources| resolve_plugins(sources, Platform::Ios, &plugins))?;
            Ok(Value::Object(resolved.remove("kit").unwrap_or_default()))
        };

        assert_eq!(
            resolve(&fixture)?,
            json!({
                "names": ["a,b"],
                "purposes": { "Run": "Tracks, counts" },
                "folder": path("/work", "assets"),
            })
        );
        fixture.config = configured(&json!({}))?;
        assert_eq!(
            resolve(&fixture)?["folder"],
            json!(path("/config", "assets"))
        );
        let help = clap::Command::new("tok")
            .args(plugin_arguments(&plugins))
            .render_help()
            .to_string();
        assert!(help.contains("[default: Run=Tracks, counts]"), "{help}");
        Ok(())
    }

    #[test]
    fn plugin_settings_prefer_options_then_environment_then_configuration() -> Result<()> {
        let mut fixture = Fixture::new();
        fixture.config = configured(&json!({
            "alerts": { "sounds": "sounds", "volume": 1, "ios": { "sounds": "ios-sounds" } },
        }))?;
        fixture.assert_plugin_settings(
            Platform::Ios,
            &with_defaults(json!({ "sounds": path("/config", "ios-sounds"), "volume": 1 })),
        )?;
        fixture.assert_plugin_settings(
            Platform::Android,
            &with_defaults(json!({ "sounds": path("/config", "sounds"), "volume": 1 })),
        )?;

        fixture.environment.extend([
            ("TOKAMAK_ALERTS_SOUNDS".to_owned(), "env".to_owned()),
            ("TOKAMAK_ALERTS_MACOS_VOLUME".to_owned(), "2".to_owned()),
        ]);
        fixture.options = parsed_options(&["--alerts-android-volume", "3"], &alerts()?)?;
        fixture.assert_plugin_settings(
            Platform::Macos,
            &with_defaults(json!({ "sounds": path("/work", "env"), "volume": 2 })),
        )?;
        fixture.assert_plugin_settings(
            Platform::Android,
            &with_defaults(json!({ "sounds": path("/work", "env"), "volume": 3 })),
        )?;
        Ok(())
    }

    #[test]
    fn plugin_options_take_precedence_over_platform_environment_variables() -> Result<()> {
        let mut fixture = Fixture::new();
        fixture
            .environment
            .insert("TOKAMAK_ALERTS_IOS_VOLUME".to_owned(), "1".to_owned());
        fixture.options = parsed_options(&["--alerts-volume", "2"], &alerts()?)?;

        assert_eq!(
            fixture.plugin_settings(Platform::Ios)?,
            with_defaults(json!({ "volume": 2 }))
        );
        Ok(())
    }

    #[test]
    fn rejects_plugin_options_of_another_kind_with_clap_s_messages() -> Result<()> {
        let plugins = alerts()?;
        for (values, message) in [
            (
                &["--alerts-tone", "shrap"][..],
                "a similar value exists: 'sharp'",
            ),
            (&["--alerts-volume", "loud"][..], "must be a whole number"),
            (
                &["--alerts-loud", "false"][..],
                "unexpected argument 'false'",
            ),
            (&["--alerts-loud=yes"][..], "[possible values: true, false]"),
            (&["--alerts-purposes", "Run"][..], "must be key=value"),
            (
                &["--alerts-sound", "x"][..],
                "a similar argument exists: '--alerts-sounds'",
            ),
            (
                &["--alerts-web-sounds", "x"][..],
                "unexpected argument '--alerts-web-sounds'",
            ),
        ] {
            let error = parsed_options(values, &plugins).err();
            let error = error.map(|error| error.to_string()).unwrap_or_default();
            assert!(error.contains(message), "{values:?}: {error}");
        }
        Ok(())
    }

    #[test]
    fn rejects_plugin_values_of_another_kind_naming_their_source() -> Result<()> {
        let cases = [
            (
                ("TOKAMAK_ALERTS_TONE", "shrap"),
                None,
                "TOKAMAK_ALERTS_TONE must be one of soft, sharp",
            ),
            (
                ("TOKAMAK_ALERTS_IOS_NAMES", " a"),
                None,
                "TOKAMAK_ALERTS_IOS_NAMES must be comma-separated values, each a non-empty value",
            ),
            (
                ("", ""),
                Some(json!({ "alerts": { "volume": "3" } })),
                "invalid tokamak() options: alerts.volume must be a whole number",
            ),
            (
                ("", ""),
                Some(json!({ "alerts": { "ios": { "purposes": ["Run"] } } })),
                "invalid tokamak() options: alerts.ios.purposes must be an object of values",
            ),
        ];
        for ((name, value), config, expected) in cases {
            let mut fixture = Fixture::new();
            fixture
                .environment
                .insert(name.to_owned(), value.to_owned());
            if let Some(config) = &config {
                fixture.config = configured(config)?;
            }

            let Err(error) = fixture.plugin_settings(Platform::Ios) else {
                anyhow::bail!("{expected}: accepted");
            };

            assert!(error.to_string().starts_with(expected), "{error}");
        }
        Ok(())
    }

    #[test]
    fn lists_plugin_settings_in_help_with_their_kinds_and_defaults() -> Result<()> {
        let help = clap::Command::new("tok")
            .args(plugin_arguments(&alerts()?))
            .render_help()
            .to_string();
        let lists = |line: &str| help.lines().any(|help| help.trim_start().starts_with(line));

        assert!(help.contains("Plugin options:"), "{help}");
        assert!(lists("--alerts-loud[=<BOOLEAN>]"), "{help}");
        assert!(lists("--alerts-volume <INTEGER>"), "{help}");
        assert!(lists("--alerts-purposes <KEY=VALUE>"), "{help}");
        assert!(
            help.contains("[`TOKAMAK_ALERTS_TONE`, `alerts.tone`]"),
            "{help}"
        );
        assert!(
            help.contains("Tone [default: soft] [`TOKAMAK_ALERTS_TONE`"),
            "{help}"
        );
        assert!(help.contains("Loud [default: false]"), "{help}");
        assert!(help.contains("[possible values: soft, sharp]"), "{help}");
        assert!(!help.contains("--alerts-ios-tone"), "{help}");
        assert!(plugins_help(&alerts()?).is_some());
        assert_eq!(plugins_help(&[]), None);
        Ok(())
    }

    #[test]
    fn rejects_configured_plugin_keys_and_plugins_that_are_not_declared() -> Result<()> {
        let cases = [
            (
                json!({ "alerts": { "ios": { "sound": "x" } } }),
                "unknown key alerts.ios.sound in the tokamak() options",
            ),
            (
                json!({ "location": { "sounds": "x" } }),
                "unknown key location in the tokamak() options; no plugin of the app has that ID",
            ),
            (
                json!({ "alerts": { "sound": "x" } }),
                "unknown key alerts.sound in the tokamak() options; plugin alerts accepts alerts.gain",
            ),
        ];
        for (config, expected) in cases {
            let mut fixture = Fixture::new();
            fixture.config = configured(&config)?;

            let Err(error) = fixture.plugin_settings(Platform::Ios) else {
                anyhow::bail!("{expected}: accepted");
            };

            assert!(error.to_string().starts_with(expected), "{error}");
        }
        Ok(())
    }

    #[test]
    fn rejects_keys_the_platform_pack_does_not_declare() -> Result<()> {
        let mut fixture = Fixture::new();
        fixture.options = namespaced_options(&["--ios-manifest", "AndroidManifest.xml"])?;
        fixture.assert_rejects(Platform::Ios, |error| {
            error
                == "unknown option --ios-manifest; iOS accepts --ios-name, --ios-identifier, \
                    --ios-icon, --ios-plist, --ios-team-id"
        });
        assert!(fixture.resolve(Platform::Macos).is_ok());

        fixture.options = NamespacedOptions::default();
        fixture.config = configured(&json!({ "macos": { "plsit": "Info.plist" } }))?;
        fixture.assert_rejects(Platform::Macos, |error| {
            error.starts_with("unknown key macos.plsit in the tokamak() options; macOS accepts")
        });
        assert!(fixture.resolve(Platform::Ios).is_ok());
        Ok(())
    }

    #[test]
    fn top_level_keys_prefer_options_then_environment_then_configuration() -> Result<()> {
        let assert_version = |fixture: &Fixture, expected: &str| -> Result<()> {
            assert_eq!(fixture.with(version)?.as_deref(), Some(expected));
            Ok(())
        };
        let mut fixture = Fixture::new();
        fixture.config = configured(&json!({ "version": "1.0.0" }))?;
        assert_version(&fixture, "1.0.0")?;
        fixture
            .environment
            .insert("TOKAMAK_VERSION".to_owned(), "2.0.0".to_owned());
        assert_version(&fixture, "2.0.0")?;
        fixture.top.version = Some("3.0.0".to_owned());
        assert_version(&fixture, "3.0.0")?;
        fixture.top.version = Some("1.0'beta\"".to_owned());
        assert_version(&fixture, "1.0'beta\"")?;
        fixture.top.version = None;
        fixture
            .environment
            .insert("TOKAMAK_VERSION".to_owned(), " 2".to_owned());
        assert!(
            fixture
                .with(version)
                .is_err_and(|error| error.to_string().starts_with("TOKAMAK_VERSION must be"))
        );
        Ok(())
    }

    #[test]
    fn lists_platform_options_in_help() {
        let help = platform_help(Platform::Ios, Ok(&manifest(Target::IosArm64)));
        assert!(help.starts_with(
            "iOS options (also TOKAMAK_IOS_<KEY>, or ios.<key> in the tokamak() options):\n"
        ));
        assert!(help.contains("\n  --ios-icon <PATH>         Icon in the platform's format\n"));
        assert!(help.contains("\n  --ios-plist <PATH>        User plist\n"));
        assert!(help.contains("\n  --ios-team-id <VALUE>     Signing team\n"));

        let help = platform_help(Platform::Android, Err("no platform pack found".to_owned()));
        assert!(help.contains("\n  --android-identifier <VALUE>  Application identifier\n"));
        assert!(
            help.ends_with(
                "  The platform pack's options are unavailable: no platform pack found\n"
            )
        );
    }
}
