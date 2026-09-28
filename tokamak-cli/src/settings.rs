//! Application settings from command-line options, environment variables, and
//! the Tokamak configuration.
//!
//! A key has the same name in each source: `--ios-plist`, `TOKAMAK_IOS_PLIST`,
//! and `ios.plist`. Options take precedence over environment variables, which
//! take precedence over the configuration. Within a source, a platform's own
//! value takes precedence over a top-level one. The CLI validates the keys it
//! owns; every other key belongs to a platform pack, which declares it.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use tokamak::{SHARED_PLATFORM_KEYS, TokamakConfig, app_name_problem, is_valid_key};
use tokamak_cli::{Platform, PlatformPackManifest, VariableKind};

/// Keys every platform shares with the top level, with their kind and description.
const SHARED_OPTIONS: [(&str, VariableKind, &str); 3] = [
    ("name", VariableKind::String, "Display name"),
    ("identifier", VariableKind::String, "Application identifier"),
    ("icon", VariableKind::Path, "Icon in the platform's format"),
];

/// Top-level values from command-line options.
#[derive(Clone, Debug, Default)]
pub(crate) struct TopOptions {
    pub(crate) name: Option<String>,
    pub(crate) identifier: Option<String>,
    pub(crate) icon: Option<String>,
    pub(crate) version: Option<String>,
    pub(crate) build: Option<String>,
}

/// Values from `--<platform>-<key>` options, by platform namespace then key.
#[derive(Clone, Debug, Default)]
pub(crate) struct PlatformOptions(BTreeMap<String, BTreeMap<String, String>>);

impl PlatformOptions {
    fn get(&self, namespace: &str, key: &str) -> Option<&String> {
        self.0.get(namespace)?.get(key)
    }

    fn keys(&self, namespace: &str) -> impl Iterator<Item = &String> {
        self.0.get(namespace).into_iter().flat_map(BTreeMap::keys)
    }
}

/// Where settings come from.
pub(crate) struct Sources<'a> {
    pub(crate) top: &'a TopOptions,
    pub(crate) platform: &'a PlatformOptions,
    pub(crate) config: &'a TokamakConfig,
    /// Reads an environment variable.
    pub(crate) environment: &'a dyn Fn(&str) -> Result<Option<String>>,
    /// Base for relative paths from options and environment variables.
    pub(crate) current_dir: &'a Path,
}

/// Settings for one platform.
#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) struct PlatformSettings {
    pub(crate) name: Option<String>,
    pub(crate) identifier: Option<String>,
    pub(crate) icon: Option<PathBuf>,
    /// The platform pack's variables, as its entrypoint's environment.
    pub(crate) pack_environment: BTreeMap<String, OsString>,
}

/// Split `--<platform>-<key> <value>` and `--<platform>-<key>=<value>` options
/// out of `arguments`, which stop at `--`. Other arguments keep their order.
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
        let Some((namespace, key, value)) = platform_option(&argument) else {
            remaining.push(argument);
            continue;
        };
        if !is_valid_key(&key) {
            bail!(
                "--{namespace}-{key} is not a valid option; keys are lowercase words joined by hyphens"
            );
        }
        let value = match value {
            Some(value) => value,
            None => arguments
                .next()
                .filter(|value| value != "--")
                .and_then(|value| value.into_string().ok())
                .with_context(|| format!("--{namespace}-{key} requires a value"))?,
        };
        let values = options.0.entry(namespace.to_owned()).or_default();
        if values.insert(key.clone(), value).is_some() {
            bail!("--{namespace}-{key} is given more than once");
        }
    }
    Ok((remaining, options))
}

/// The namespace, key, and inline value of a platform option.
fn platform_option(argument: &OsString) -> Option<(&'static str, String, Option<String>)> {
    let option = argument.to_str()?.strip_prefix("--")?;
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

/// The app version: `--version`, `TOKAMAK_VERSION`, or `version`.
pub(crate) fn version(sources: &Sources<'_>) -> Result<Option<String>> {
    let value = top_value(sources, "version", sources.top.version.as_ref())?;
    Ok(value.or_else(|| sources.config.version.clone()))
}

/// The project build command: `--build`, `TOKAMAK_BUILD`, or `build`.
pub(crate) fn build_command(sources: &Sources<'_>) -> Result<Option<String>> {
    let value = top_value(sources, "build", sources.top.build.as_ref())?;
    Ok(value.or_else(|| sources.config.build.clone()))
}

/// Resolve `platform`'s settings, rejecting keys its pack does not declare.
pub(crate) fn resolve(
    sources: &Sources<'_>,
    platform: Platform,
    manifest: &PlatformPackManifest,
) -> Result<PlatformSettings> {
    let config_platform = platform.directory_name();
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
    let icon = shared_value(sources, platform, "icon", sources.top.icon.as_ref())?;
    Ok(PlatformSettings {
        name: name.or_else(|| sources.config.name.for_platform(config_platform).cloned()),
        identifier: identifier.or_else(|| {
            sources
                .config
                .identifier
                .for_platform(config_platform)
                .cloned()
        }),
        icon: icon
            .map(|icon| sources.current_dir.join(icon))
            .or_else(|| sources.config.icon.for_platform(config_platform).cloned()),
        pack_environment: pack_environment(sources, platform, manifest)?,
    })
}

/// A top-level key's option or environment value.
fn top_value(sources: &Sources<'_>, key: &str, option: Option<&String>) -> Result<Option<String>> {
    if let Some(value) = option {
        return valid(&format!("--{key}"), value.clone()).map(Some);
    }
    environment_value(sources, &environment_name(None, key))
}

/// A shared key's option or environment value: the platform's own, then the top-level one.
fn shared_value(
    sources: &Sources<'_>,
    platform: Platform,
    key: &str,
    top_option: Option<&String>,
) -> Result<Option<String>> {
    let namespace = platform.namespace();
    if let Some(value) = sources.platform.get(namespace, key) {
        return valid(&format!("--{namespace}-{key}"), value.clone()).map(Some);
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

/// The platform pack's variables, keyed by their environment variable names.
fn pack_environment(
    sources: &Sources<'_>,
    platform: Platform,
    manifest: &PlatformPackManifest,
) -> Result<BTreeMap<String, OsString>> {
    reject_undeclared_keys(sources, platform, manifest)?;
    let namespace = platform.namespace();
    let mut environment = BTreeMap::new();
    for (key, variable) in &manifest.variables {
        let name = environment_name(Some(namespace), key);
        let Some((value, directory)) = pack_value(sources, namespace, key, &name)? else {
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
) -> Result<Option<(String, PathBuf)>> {
    if let Some(value) = sources.platform.get(namespace, key) {
        let value = valid(&format!("--{namespace}-{key}"), value.clone())?;
        return Ok(Some((value, sources.current_dir.to_path_buf())));
    }
    if let Some(value) = environment_value(sources, environment_name)? {
        return Ok(Some((value, sources.current_dir.to_path_buf())));
    }
    let configured = sources
        .config
        .pack_values
        .get(namespace)
        .and_then(|values| values.get(key));
    Ok(configured.map(|configured| (configured.value.clone(), configured.directory.clone())))
}

fn reject_undeclared_keys(
    sources: &Sources<'_>,
    platform: Platform,
    manifest: &PlatformPackManifest,
) -> Result<()> {
    let namespace = platform.namespace();
    let declared = |key: &String| manifest.variables.contains_key(key);
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
    let configured = sources.config.pack_values.get(namespace);
    if let Some(key) = configured
        .into_iter()
        .flat_map(BTreeMap::keys)
        .find(|key| !declared(key))
    {
        let path = sources
            .config
            .path
            .as_deref()
            .unwrap_or(Path::new("tokamak.jsonc"));
        bail!(
            "unknown key {namespace}.{key} in {}; {}",
            path.display(),
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
    if value.trim().is_empty() || value != value.trim() || value.chars().any(char::is_control) {
        bail!(
            "{source} must be a non-empty value without surrounding whitespace or control characters"
        );
    }
    Ok(value)
}

/// `--help` text listing a platform's options, or why they cannot be listed.
pub(crate) fn platform_help(
    platform: Platform,
    manifest: std::result::Result<&PlatformPackManifest, String>,
) -> String {
    let namespace = platform.namespace();
    let mut help = format!(
        "{} options (also TOKAMAK_{}_<KEY>, or {namespace}.<key> in tokamak.jsonc):\n",
        platform.display_name(),
        namespace.to_ascii_uppercase()
    );
    let manifest = match manifest {
        Ok(manifest) => manifest,
        Err(error) => {
            let _ = writeln!(help, "  the platform pack could not be loaded: {error}");
            return help;
        }
    };
    let pack_options = manifest
        .variables
        .iter()
        .map(|(key, variable)| (key.as_str(), variable.kind, variable.description.as_str()));
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
    help
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    use anyhow::Result;
    use tokamak::{PackValue, PlatformValues, SHARED_PLATFORM_KEYS, TokamakConfig};
    use tokamak_cli::{PackVariable, Platform, PlatformPackManifest, Target, VariableKind};

    use super::{
        PlatformOptions, PlatformSettings, SHARED_OPTIONS, Sources, TopOptions, build_command,
        environment_name, platform_help, resolve, split_platform_options, version,
    };

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
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
            artifacts: Vec::new(),
            required_tools: Vec::new(),
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
        }
    }

    fn pack_values(
        platform: &str,
        values: &[(&str, &str, &str)],
    ) -> BTreeMap<String, BTreeMap<String, PackValue>> {
        let values = values
            .iter()
            .map(|(key, value, directory)| {
                let value = PackValue {
                    value: (*value).to_owned(),
                    directory: PathBuf::from(directory),
                };
                ((*key).to_owned(), value)
            })
            .collect();
        BTreeMap::from([(platform.to_owned(), values)])
    }

    struct Fixture {
        top: TopOptions,
        platform: PlatformOptions,
        config: TokamakConfig,
        environment: BTreeMap<String, String>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                top: TopOptions::default(),
                platform: PlatformOptions::default(),
                config: TokamakConfig::default(),
                environment: BTreeMap::new(),
            }
        }

        fn with<T>(&self, run: impl FnOnce(&Sources<'_>) -> Result<T>) -> Result<T> {
            let environment = |name: &str| Ok(self.environment.get(name).cloned());
            run(&Sources {
                top: &self.top,
                platform: &self.platform,
                config: &self.config,
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
            options.get("ios", "plist").map(String::as_str),
            Some("Info.plist")
        );
        assert_eq!(
            options.get("android", "manifest").map(String::as_str),
            Some("AndroidManifest.xml")
        );
        assert_eq!(options.get("ios", "team-id"), None);
        Ok(())
    }

    #[test]
    fn rejects_malformed_platform_options() {
        for (values, message) in [
            (&["--ios-plist"][..], "--ios-plist requires a value"),
            (&["--ios-plist", "--"][..], "--ios-plist requires a value"),
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
        fixture.config.identifier = PlatformValues {
            default: Some("com.config.top".to_owned()),
            ios: Some("com.config.ios".to_owned()),
            ..PlatformValues::default()
        };
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
    fn option_and_environment_icons_are_relative_to_the_current_directory() -> Result<()> {
        let mut fixture = Fixture::new();
        fixture.config.icon.default = Some(PathBuf::from("/config/AppIcon.icon"));
        assert_eq!(
            fixture.resolve(Platform::Macos)?.icon,
            Some(PathBuf::from("/config/AppIcon.icon"))
        );
        fixture
            .environment
            .insert("TOKAMAK_ICON".to_owned(), "icons/App.icon".to_owned());
        assert_eq!(
            fixture.resolve(Platform::Macos)?.icon,
            Some(PathBuf::from("/work/icons/App.icon"))
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
        fixture.config.pack_values = pack_values(
            "ios",
            &[
                ("plist", "native/Info.plist", "/config"),
                ("team-id", "CONFIG", "/config"),
            ],
        );
        let environment = |settings: PlatformSettings| settings.pack_environment;
        assert_eq!(
            environment(fixture.resolve(Platform::Ios)?),
            BTreeMap::from([
                (
                    "TOKAMAK_IOS_PLIST".to_owned(),
                    OsString::from("/config/native/Info.plist")
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
                    OsString::from("/work/env/Info.plist")
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
        fixture.config.path = Some(PathBuf::from("/config/tokamak.jsonc"));
        fixture.config.pack_values = pack_values("macos", &[("plsit", "Info.plist", "/config")]);
        assert!(fixture.resolve(Platform::Macos).is_err_and(|error| {
            error
                .to_string()
                .starts_with("unknown key macos.plsit in /config/tokamak.jsonc; macOS accepts")
        }));
        assert!(fixture.resolve(Platform::Ios).is_ok());
        Ok(())
    }

    #[test]
    fn top_level_keys_prefer_options_then_environment_then_configuration() -> Result<()> {
        let mut fixture = Fixture::new();
        fixture.config.version = Some("1.0.0".to_owned());
        fixture.config.build = Some("pnpm build".to_owned());
        assert_eq!(fixture.with(version)?.as_deref(), Some("1.0.0"));
        fixture
            .environment
            .insert("TOKAMAK_VERSION".to_owned(), "2.0.0".to_owned());
        fixture
            .environment
            .insert("TOKAMAK_BUILD".to_owned(), "turbo build".to_owned());
        assert_eq!(fixture.with(version)?.as_deref(), Some("2.0.0"));
        assert_eq!(fixture.with(build_command)?.as_deref(), Some("turbo build"));
        fixture.top.version = Some("3.0.0".to_owned());
        assert_eq!(fixture.with(version)?.as_deref(), Some("3.0.0"));
        Ok(())
    }

    #[test]
    fn lists_platform_options_in_help() {
        let help = platform_help(Platform::Ios, Ok(&manifest(Target::IosArm64)));
        assert!(
            help.starts_with(
                "iOS options (also TOKAMAK_IOS_<KEY>, or ios.<key> in tokamak.jsonc):\n"
            )
        );
        assert!(help.contains("\n  --ios-icon <PATH>         Icon in the platform's format\n"));
        assert!(help.contains("\n  --ios-plist <PATH>        User plist\n"));
        assert!(help.contains("\n  --ios-team-id <VALUE>     Signing team\n"));

        let help = platform_help(Platform::Android, Err("no platform pack found".to_owned()));
        assert!(
            help.ends_with("  the platform pack could not be loaded: no platform pack found\n")
        );
    }
}
