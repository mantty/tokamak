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

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use clap::Args;
use tokamak_cli::{
    Platform, PlatformPackManifest, SHARED_PLATFORM_KEYS, VariableKind, is_valid_key,
};

use super::tokamak_config::{TokamakConfig, app_name_problem, value_problem};

/// Keys every platform shares with the top level, with their kind and description.
const SHARED_OPTIONS: [(&str, VariableKind, &str); 3] = [
    ("name", VariableKind::String, "Display name"),
    ("identifier", VariableKind::String, "Application identifier"),
    ("icon", VariableKind::Path, "Icon in the platform's format"),
];

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

/// Values from `--<platform>-<key>` options, by platform namespace then key.
#[derive(Clone, Debug, Default)]
pub(crate) struct PlatformOptions(BTreeMap<String, BTreeMap<String, String>>);

impl PlatformOptions {
    fn value(&self, namespace: &str, key: &str) -> Result<Option<String>> {
        let value = self.0.get(namespace).and_then(|values| values.get(key));
        value
            .map(|value| valid(&format!("--{namespace}-{key}"), value.clone()))
            .transpose()
    }

    fn keys(&self, namespace: &str) -> impl Iterator<Item = &String> {
        self.0.get(namespace).into_iter().flat_map(BTreeMap::keys)
    }
}

/// Where settings come from.
pub(crate) struct Sources<'a> {
    pub(crate) top: &'a TopOptions,
    pub(crate) platform: &'a PlatformOptions,
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
        platform: &'a PlatformOptions,
        config: Option<&'a TokamakConfig>,
        current_dir: &'a Path,
    ) -> Self {
        Self {
            top,
            platform,
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

/// Split `--<platform>-<key> <value>` and `--<platform>-<key>=<value>` options
/// out of `arguments`, which stop at `--`. Other arguments keep their order. A
/// separate value may not start with `-`, as with other options.
pub(crate) fn split_platform_options(
    arguments: Vec<OsString>,
) -> Result<(Vec<OsString>, PlatformOptions)> {
    let mut remaining = Vec::new();
    let mut options = PlatformOptions::default();
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        if argument == "--" {
            remaining.push(argument);
            remaining.extend(arguments);
            break;
        }
        let text = argument.to_string_lossy();
        let Some((namespace, key, value)) = platform_option(&text) else {
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
        let values = options.0.entry(namespace.to_owned()).or_default();
        if values.insert(key, value).is_some() {
            bail!("{option} is given more than once");
        }
    }
    Ok((remaining, options))
}

/// The namespace, key, and inline value of a platform option.
fn platform_option(argument: &str) -> Option<(&'static str, String, Option<String>)> {
    let option = argument.strip_prefix("--")?;
    let (name, value) = match option.split_once('=') {
        Some((name, value)) => (name, Some(value.to_owned())),
        None => (option, None),
    };
    Platform::ALL.iter().find_map(|platform| {
        let namespace = platform.namespace();
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
    if let Some(value) = sources.platform.value(namespace, key)? {
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
    if let Some(value) = sources.platform.value(namespace, key)? {
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
    if let Some(key) = sources
        .platform
        .keys(namespace)
        .find(|key| !SHARED_PLATFORM_KEYS.contains(&key.as_str()) && !declared(key))
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
    let options = SHARED_OPTIONS
        .iter()
        .map(|(key, _, _)| *key)
        .chain(manifest.variables.keys().map(String::as_str))
        .map(|key| format!("--{namespace}-{key}"))
        .collect::<Vec<_>>();
    format!("{} accepts {}", platform.display_name(), options.join(", "))
}

/// `TOKAMAK_<PLATFORM>_<KEY>`, or `TOKAMAK_<KEY>` for a top-level key.
fn environment_name(namespace: Option<&str>, key: &str) -> String {
    let name = match namespace {
        Some(namespace) => format!("tokamak-{namespace}-{key}"),
        None => format!("tokamak-{key}"),
    };
    name.replace('-', "_").to_ascii_uppercase()
}

/// A value from an option or environment variable, held to the configuration's rules.
fn valid(source: &str, value: String) -> Result<String> {
    match value_problem(&value) {
        Some(problem) => bail!("{source} {problem}"),
        None => Ok(value),
    }
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
    let rows = SHARED_OPTIONS
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
    use std::path::Path;

    use anyhow::Result;
    use serde_json::{Value, json};
    use tokamak_cli::SHARED_PLATFORM_KEYS;

    use crate::tokamak_config::{TokamakConfig, parse_config};
    use tokamak_cli::{PackVariable, Platform, PlatformPackManifest, Target, VariableKind};

    use super::{
        PlatformOptions, PlatformSettings, SHARED_OPTIONS, Sources, TopOptions, environment_name,
        platform_help, resolve, split_platform_options, version,
    };

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    /// `relative` in `directory`, as a pack receives a path setting.
    fn joined(directory: &str, relative: &str) -> OsString {
        Path::new(directory).join(relative).into_os_string()
    }

    fn platform_options(values: &[&str]) -> Result<PlatformOptions> {
        Ok(split_platform_options(arguments(values))?.1)
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
        platform: PlatformOptions,
        config: Option<TokamakConfig>,
        environment: BTreeMap<String, String>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                top: TopOptions::default(),
                platform: PlatformOptions::default(),
                config: None,
                environment: BTreeMap::new(),
            }
        }

        fn with<T>(&self, run: impl FnOnce(&Sources<'_>) -> Result<T>) -> Result<T> {
            let environment = |name: &str| Ok(self.environment.get(name).cloned());
            run(&Sources {
                top: &self.top,
                platform: &self.platform,
                config: self.config.as_ref(),
                environment: &environment,
                current_dir: Path::new("/work"),
            })
        }

        fn resolve(&self, platform: Platform) -> Result<PlatformSettings> {
            let manifest = manifest(platform.default_target()?);
            self.with(|sources| resolve(sources, platform, &manifest))
        }
    }

    #[test]
    fn splits_platform_options_from_other_arguments() -> Result<()> {
        let (remaining, options) = split_platform_options(arguments(&[
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
        assert_eq!(
            options.value("ios", "plist")?.as_deref(),
            Some("Info.plist")
        );
        assert_eq!(
            options.value("android", "manifest")?.as_deref(),
            Some("AndroidManifest.xml")
        );
        assert_eq!(options.value("ios", "team-id")?, None);
        Ok(())
    }

    #[test]
    fn keeps_equals_signs_and_leading_hyphens_in_inline_values() -> Result<()> {
        let options = platform_options(&["--ios-team-id=a=b", "--ios-plist=-Info.plist"])?;
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
            let error = split_platform_options(arguments(values)).map(|_| ());
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
        let error = split_platform_options(vec![invalid]).map(|_| ());
        assert!(error.is_err_and(|error| error.to_string() == "--ios-plist must be valid UTF-8"));

        let invalid = OsString::from_vec(b"\xff".to_vec());
        let error =
            split_platform_options(vec![OsString::from("--ios-plist"), invalid]).map(|_| ());
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
    fn shared_platform_options_cover_the_shared_keys() {
        let keys = SHARED_OPTIONS.map(|(key, _, _)| key);
        assert_eq!(keys, SHARED_PLATFORM_KEYS);
    }

    #[test]
    fn shared_keys_prefer_options_then_environment_then_configuration() -> Result<()> {
        let mut fixture = Fixture::new();
        fixture.config = configured(&json!({
            "identifier": "com.config.top",
            "ios": { "identifier": "com.config.ios" },
        }))?;
        assert_eq!(
            fixture.resolve(Platform::Ios)?.identifier.as_deref(),
            Some("com.config.ios")
        );
        assert_eq!(
            fixture.resolve(Platform::Macos)?.identifier.as_deref(),
            Some("com.config.top")
        );

        fixture
            .environment
            .insert("TOKAMAK_IDENTIFIER".to_owned(), "com.env.top".to_owned());
        assert_eq!(
            fixture.resolve(Platform::Ios)?.identifier.as_deref(),
            Some("com.env.top")
        );
        fixture.environment.insert(
            "TOKAMAK_IOS_IDENTIFIER".to_owned(),
            "com.env.ios".to_owned(),
        );
        assert_eq!(
            fixture
                .resolve(Platform::IosSimulator)?
                .identifier
                .as_deref(),
            Some("com.env.ios")
        );

        fixture.top.identifier = Some("com.option.top".to_owned());
        assert_eq!(
            fixture.resolve(Platform::Ios)?.identifier.as_deref(),
            Some("com.option.top")
        );
        fixture.platform = platform_options(&["--ios-identifier", "com.option.ios"])?;
        assert_eq!(
            fixture.resolve(Platform::Ios)?.identifier.as_deref(),
            Some("com.option.ios")
        );
        assert_eq!(
            fixture.resolve(Platform::Macos)?.identifier.as_deref(),
            Some("com.option.top")
        );
        Ok(())
    }

    #[test]
    fn passes_the_icon_to_the_pack_as_a_path() -> Result<()> {
        let mut fixture = Fixture::new();
        let icon =
            |settings: PlatformSettings, name: &str| settings.pack_environment.get(name).cloned();
        assert_eq!(
            icon(fixture.resolve(Platform::Macos)?, "TOKAMAK_MACOS_ICON"),
            None
        );
        fixture.config = configured(&json!({ "icon": "../AppIcon.icon" }))?;
        assert_eq!(
            icon(fixture.resolve(Platform::Macos)?, "TOKAMAK_MACOS_ICON"),
            Some(joined("/config", "../AppIcon.icon"))
        );
        fixture
            .environment
            .insert("TOKAMAK_ICON".to_owned(), "icons/App.icon".to_owned());
        assert_eq!(
            icon(fixture.resolve(Platform::Macos)?, "TOKAMAK_MACOS_ICON"),
            Some(joined("/work", "icons/App.icon"))
        );
        fixture.top.icon = Some("top/App.icon".to_owned());
        fixture.platform = platform_options(&["--macos-icon", "macos/App.icon"])?;
        assert_eq!(
            icon(fixture.resolve(Platform::Macos)?, "TOKAMAK_MACOS_ICON"),
            Some(joined("/work", "macos/App.icon"))
        );
        assert_eq!(
            icon(fixture.resolve(Platform::IosSimulator)?, "TOKAMAK_IOS_ICON"),
            Some(joined("/work", "top/App.icon"))
        );
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
        assert_eq!(
            environment(fixture.resolve(Platform::Ios)?),
            BTreeMap::from([
                (
                    "TOKAMAK_IOS_PLIST".to_owned(),
                    joined("/config", "native/Info.plist")
                ),
                ("TOKAMAK_IOS_TEAM_ID".to_owned(), OsString::from("CONFIG")),
            ])
        );

        fixture
            .environment
            .insert("TOKAMAK_IOS_PLIST".to_owned(), "env/Info.plist".to_owned());
        fixture.platform = platform_options(&["--ios-team-id", "OPTION"])?;
        assert_eq!(
            environment(fixture.resolve(Platform::IosSimulator)?),
            BTreeMap::from([
                (
                    "TOKAMAK_IOS_PLIST".to_owned(),
                    joined("/work", "env/Info.plist")
                ),
                ("TOKAMAK_IOS_TEAM_ID".to_owned(), OsString::from("OPTION")),
            ])
        );
        assert!(environment(fixture.resolve(Platform::Macos)?).is_empty());
        Ok(())
    }

    #[test]
    fn rejects_keys_the_platform_pack_does_not_declare() -> Result<()> {
        let mut fixture = Fixture::new();
        fixture.platform = platform_options(&["--ios-manifest", "AndroidManifest.xml"])?;
        assert!(fixture.resolve(Platform::Ios).is_err_and(|error| {
            error.to_string()
                == "unknown option --ios-manifest; iOS accepts --ios-name, --ios-identifier, \
                    --ios-icon, --ios-plist, --ios-team-id"
        }));
        assert!(fixture.resolve(Platform::Macos).is_ok());

        fixture.platform = PlatformOptions::default();
        fixture.config = configured(&json!({ "macos": { "plsit": "Info.plist" } }))?;
        assert!(fixture.resolve(Platform::Macos).is_err_and(|error| {
            error
                .to_string()
                .starts_with("unknown key macos.plsit in the tokamak() options; macOS accepts")
        }));
        assert!(fixture.resolve(Platform::Ios).is_ok());
        Ok(())
    }

    #[test]
    fn top_level_keys_prefer_options_then_environment_then_configuration() -> Result<()> {
        let mut fixture = Fixture::new();
        fixture.config = configured(&json!({ "version": "1.0.0" }))?;
        assert_eq!(fixture.with(version)?.as_deref(), Some("1.0.0"));
        fixture
            .environment
            .insert("TOKAMAK_VERSION".to_owned(), "2.0.0".to_owned());
        assert_eq!(fixture.with(version)?.as_deref(), Some("2.0.0"));
        fixture.top.version = Some("3.0.0".to_owned());
        assert_eq!(fixture.with(version)?.as_deref(), Some("3.0.0"));
        fixture.top.version = Some("1.0'beta\"".to_owned());
        assert_eq!(fixture.with(version)?.as_deref(), Some("1.0'beta\""));
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
