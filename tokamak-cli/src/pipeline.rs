use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use tokamak::{
    TokamakConfig, WranglerConfig, load_tokamak_config, load_wrangler_config_for_env,
    resolve_tokamak_config_path, resolve_wrangler_config_path, slug,
};
use tokamak_cli::{MANIFEST_FILE, Platform, PlatformPackManifest, Target, load_manifest};

use super::{cache, plugins, support, variables, worker};

pub(crate) struct BuildRequest {
    pub(crate) platforms: Vec<Platform>,
    pub(crate) project_dir: PathBuf,
    pub(crate) build_dir: Option<PathBuf>,
    pub(crate) platform_pack_dir: Option<PathBuf>,
    pub(crate) tokamak_config_path: PathBuf,
    pub(crate) wrangler_config_path: Option<PathBuf>,
    pub(crate) wrangler_env: Option<String>,
    pub(crate) set: Vec<variables::SetVariable>,
    pub(crate) skip_project_build: bool,
}

pub(crate) struct BuildSummary {
    pub(crate) platform: Platform,
    pub(crate) bundle_dir: PathBuf,
}

pub(crate) struct DevelopmentRequest {
    pub(crate) platform: Platform,
    pub(crate) project_dir: PathBuf,
    pub(crate) platform_pack_dir: Option<PathBuf>,
    pub(crate) tokamak_config_path: PathBuf,
    pub(crate) wrangler_config_path: Option<PathBuf>,
    pub(crate) endpoint: String,
    pub(crate) session_token: String,
    pub(crate) device_id: Option<String>,
    pub(crate) set: Vec<variables::SetVariable>,
}

pub(crate) struct DevelopmentSummary {
    pub(crate) platform: Platform,
    pub(crate) bundle_dir: PathBuf,
    pub(crate) app_slug: String,
    pub(crate) identifier: String,
}

struct BuildContext<'a> {
    build_dir: &'a Path,
    wrangler: &'a WranglerConfig,
    tokamak: &'a TokamakConfig,
    plugins: &'a [plugins::Plugin],
    version: &'a str,
    environment: &'a std::collections::BTreeMap<String, std::ffi::OsString>,
}

struct BuildMetadata<'a> {
    app: (&'a str, &'a str),
    identifier: &'a str,
    manifest: &'a PlatformPackManifest,
    version: Option<&'a str>,
    development: Option<(&'a str, &'a str)>,
    device_id: Option<&'a str>,
}

pub(crate) fn run(request: &BuildRequest) -> Result<Vec<BuildSummary>> {
    validate_request(request)?;
    let project = fs::canonicalize(&request.project_dir)?;
    let build_dir = project.join(request.build_dir.as_deref().unwrap_or(Path::new("build")));
    fs::create_dir_all(&build_dir)?;
    let build_dir = fs::canonicalize(build_dir)?;
    let environment = variables::environment(&request.set)?;
    let tokamak = load_project_config(&request.tokamak_config_path)?;
    let version = required_version(&tokamak)?;
    let mut before = None;
    if !request.skip_project_build && !cache::project_is_current(&project, &build_dir)? {
        before = Some(cache::project_files(&project, &build_dir)?);
        support::run_project_build(&project)?;
    }
    let wrangler = load_wrangler(
        &request.project_dir,
        request.wrangler_config_path.as_deref(),
        request.wrangler_env.as_deref(),
    )?;
    support::validate_project_build(&wrangler)?;
    if let Some(before) = before.as_ref() {
        cache::record_project_build(&project, &build_dir, &wrangler, before)?;
    }
    let plugins = plugins::discover(&request.project_dir)?;
    let context = BuildContext {
        build_dir: &build_dir,
        wrangler: &wrangler,
        tokamak: &tokamak,
        plugins: &plugins,
        version: &version,
        environment: &environment,
    };

    request
        .platforms
        .iter()
        .map(|platform| build_platform(request, *platform, &context))
        .collect()
}

fn load_wrangler(
    project_dir: &Path,
    config_path: Option<&Path>,
    environment: Option<&str>,
) -> Result<WranglerConfig> {
    let config_base = if config_path.is_some() {
        env::current_dir()?
    } else {
        fs::canonicalize(project_dir)?
    };
    let config_path = resolve_wrangler_config_path(&config_base, config_path)?;
    Ok(load_wrangler_config_for_env(&config_path, environment)?)
}

pub(crate) fn run_development(request: &DevelopmentRequest) -> Result<DevelopmentSummary> {
    if !request.project_dir.is_dir() {
        bail!(
            "project directory does not exist: {}",
            request.project_dir.display()
        );
    }
    let environment = variables::environment(&request.set)?;
    let tokamak = load_project_config(&request.tokamak_config_path)?;
    let wrangler = load_wrangler(
        &request.project_dir,
        request.wrangler_config_path.as_deref(),
        None,
    )?;
    let plugins = plugins::discover(&request.project_dir)?;
    let (app_name, app_slug) = resolve_app(&tokamak, &wrangler.name, request.platform);
    let identifier = resolve_identifier(&tokamak, &app_slug, request.platform)?;
    let version = resolve_version(&tokamak)?;
    let (input, pack_root, manifest, project) = prepare_platform_input(
        &request.project_dir,
        &request.project_dir.join("build"),
        request.platform,
        request.platform_pack_dir.as_deref(),
        &tokamak,
    )?;
    plugins::stage(&plugins, request.platform, &input.join("plugins"))
        .context("stage native plugin inputs")?;
    fs::write(input.join("app/.tokamak-development"), b"")?;
    write_build_metadata(
        &input,
        &project,
        &BuildMetadata {
            app: (&app_name, &app_slug),
            identifier: &identifier,
            manifest: &manifest,
            version: version.as_deref(),
            development: Some((&request.endpoint, &request.session_token)),
            device_id: request.device_id.as_deref(),
        },
    )
    .context("write development metadata")?;

    let bundle_dir = output_path(&project.join("build"), request.platform, &app_slug);
    support::run_entrypoint(
        &pack_root,
        &input,
        &bundle_dir,
        manifest.target,
        &environment,
    )
    .with_context(|| {
        format!(
            "build {} development shell using platform-pack entrypoint",
            request.platform.display_name()
        )
    })?;
    Ok(DevelopmentSummary {
        platform: request.platform,
        bundle_dir,
        app_slug,
        identifier,
    })
}

fn validate_request(request: &BuildRequest) -> Result<()> {
    if request.platforms.is_empty() {
        bail!("at least one build platform is required");
    }
    if !request.project_dir.is_dir() {
        bail!(
            "project directory does not exist: {}",
            request.project_dir.display()
        );
    }
    if request.platform_pack_dir.is_some() && request.platforms.len() > 1 {
        bail!("--platform-pack can only be used with a single platform");
    }
    Ok(())
}

fn build_platform(
    request: &BuildRequest,
    platform: Platform,
    context: &BuildContext<'_>,
) -> Result<BuildSummary> {
    let (input, pack_root, manifest, project) = prepare_platform_input(
        &request.project_dir,
        context.build_dir,
        platform,
        request.platform_pack_dir.as_deref(),
        context.tokamak,
    )?;

    let worker_cache = context
        .build_dir
        .join(".tokamak")
        .join(platform.directory_name())
        .join("worker");
    worker::prepare_quickjs_app(
        &input.join("app"),
        &worker_cache,
        &pack_root,
        &manifest,
        context.wrangler,
    )
    .context("prepare the tokamak application package")?;
    plugins::stage(context.plugins, platform, &input.join("plugins"))
        .context("stage native plugin inputs")?;
    let (app_name, app_slug) = resolve_app(context.tokamak, &context.wrangler.name, platform);
    let identifier = resolve_identifier(context.tokamak, &app_slug, platform)?;
    write_build_metadata(
        &input,
        &project,
        &BuildMetadata {
            app: (&app_name, &app_slug),
            identifier: &identifier,
            manifest: &manifest,
            version: Some(context.version),
            development: None,
            device_id: None,
        },
    )
    .context("write platform build metadata")?;

    if matches!(
        platform,
        Platform::Ios | Platform::IosSimulator | Platform::Macos
    ) {
        let environment = input.join("app/worker-environment.json");
        let key = input.join("metadata/input-key");
        fs::write(
            &key,
            cache::hash_tree(&input, |path| path == environment || path == key)?,
        )?;
    }

    let output = output_path(context.build_dir, platform, &app_slug);
    support::run_entrypoint(
        &pack_root,
        &input,
        &output,
        manifest.target,
        context.environment,
    )
    .with_context(|| {
        format!(
            "build {} using platform-pack entrypoint",
            platform.display_name()
        )
    })?;
    Ok(BuildSummary {
        platform,
        bundle_dir: output,
    })
}

pub(crate) fn resolve_app(
    config: &TokamakConfig,
    worker_name: &str,
    platform: Platform,
) -> (String, String) {
    match config.name.for_platform(platform.directory_name()) {
        Some(name) => (name.clone(), slug(name)),
        None => (worker_name.to_owned(), worker_name.to_owned()),
    }
}

pub(crate) fn resolve_identifier(
    config: &TokamakConfig,
    app_slug: &str,
    platform: Platform,
) -> Result<String> {
    let value = if let Some(value) = environment_value(platform_identifier_env(platform))? {
        value
    } else if let Some(value) = environment_value("TOKAMAK_IDENTIFIER")? {
        value
    } else if let Some(identifier) = config.identifier.for_platform(platform.directory_name()) {
        identifier.clone()
    } else {
        default_identifier(app_slug, platform)
    };
    validate_platform_identifier(platform, value)
}

pub(crate) fn resolve_version(config: &TokamakConfig) -> Result<Option<String>> {
    if let Some(value) = environment_value("TOKAMAK_VERSION")? {
        return Ok(Some(validate_version("TOKAMAK_VERSION", value)?));
    }
    config
        .version
        .as_ref()
        .map(|value| validate_version("version", value.clone()))
        .transpose()
}

fn required_version(config: &TokamakConfig) -> Result<String> {
    resolve_version(config)?.ok_or_else(|| {
        anyhow::anyhow!(
            "tokamak version is required for `tok build`; set `version` in tokamak.jsonc or TOKAMAK_VERSION"
        )
    })
}

fn environment_value(name: &str) -> Result<Option<String>> {
    match env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => bail!("{name} must contain valid UTF-8"),
    }
}

fn platform_identifier_env(platform: Platform) -> &'static str {
    match platform {
        Platform::Android => "TOKAMAK_ANDROID_IDENTIFIER",
        Platform::Ios | Platform::IosSimulator => "TOKAMAK_IOS_IDENTIFIER",
        Platform::Macos => "TOKAMAK_MACOS_IDENTIFIER",
        Platform::Windows => "TOKAMAK_WINDOWS_IDENTIFIER",
    }
}

fn default_identifier(app_slug: &str, platform: Platform) -> String {
    if platform == Platform::Android {
        let mut name = app_slug.replace('-', "_");
        if name
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_digit())
        {
            name.insert_str(0, "app_");
        }
        format!("com.tokamak.{name}")
    } else {
        format!("com.tokamak.{app_slug}")
    }
}

fn validate_platform_identifier(platform: Platform, identifier: String) -> Result<String> {
    let valid = match platform {
        Platform::Android => {
            let parts = identifier.split('.').collect::<Vec<_>>();
            parts.len() >= 2
                && parts.iter().all(|part| {
                    part.chars()
                        .next()
                        .is_some_and(|character| character.is_ascii_alphabetic())
                        && part
                            .chars()
                            .all(|character| character.is_ascii_alphanumeric() || character == '_')
                })
        }
        Platform::Ios | Platform::IosSimulator | Platform::Macos => {
            !identifier.starts_with('.')
                && !identifier.ends_with('.')
                && !identifier.contains("..")
                && identifier
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || ".-".contains(character))
        }
        Platform::Windows => {
            !identifier.is_empty()
                && identifier
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || ".-_".contains(character))
        }
    };
    if valid {
        Ok(identifier)
    } else {
        bail!(
            "invalid {} identifier `{identifier}`",
            platform.display_name()
        )
    }
}

fn validate_version(field: &str, value: String) -> Result<String> {
    if value.trim().is_empty()
        || value != value.trim()
        || value.chars().any(char::is_control)
        || value.contains('\'')
    {
        bail!(
            "{field} must be a non-empty version without whitespace, quotes, or control characters"
        )
    }
    Ok(value)
}

fn prepare_platform_input(
    project_dir: &Path,
    build_dir: &Path,
    platform: Platform,
    platform_pack_dir: Option<&Path>,
    tokamak: &TokamakConfig,
) -> Result<(PathBuf, PathBuf, PlatformPackManifest, PathBuf)> {
    let manifest_path = fs::canonicalize(resolve_manifest(platform, platform_pack_dir)?)?;
    let manifest = load_manifest(&manifest_path)
        .with_context(|| format!("invalid platform pack: {}", manifest_path.display()))?;
    manifest
        .validate_cli_version(env!("CARGO_PKG_VERSION"))
        .with_context(|| format!("incompatible platform pack: {}", manifest_path.display()))?;
    support::validate_target(&manifest, platform)?;
    let pack_root = manifest_path
        .parent()
        .context("platform-pack manifest must have a parent directory")?
        .to_path_buf();
    let project = fs::canonicalize(project_dir)
        .with_context(|| format!("resolve project directory: {}", project_dir.display()))?;
    let staging = build_dir.join(".tokamak").join(platform.directory_name());
    let input = staging.join("input");
    support::reset_path(&input)
        .with_context(|| format!("reset build input directory: {}", input.display()))?;
    fs::create_dir_all(input.join("metadata")).with_context(|| {
        format!(
            "create build input directory: {}",
            input.join("metadata").display()
        )
    })?;
    fs::create_dir_all(input.join("app"))?;
    support::stage_platform_artifacts(&input, &pack_root, &manifest)
        .context("stage platform-pack platform artifacts")?;
    support::stage_platform_icons(&input, tokamak, platform)
        .context("stage Tokamak application assets")?;
    Ok((input, pack_root, manifest, project))
}

pub(crate) fn load_project_config(config_path: &Path) -> Result<TokamakConfig> {
    let current_dir = env::current_dir()?;
    let path = resolve_tokamak_config_path(&current_dir, Some(config_path))?;
    let Some(path) = path else {
        return Ok(TokamakConfig::default());
    };
    let loaded = load_tokamak_config(&path)
        .with_context(|| format!("load Tokamak config {}", path.display()))?;
    for warning in &loaded.warnings {
        eprintln!("warning: {warning}");
    }
    Ok(loaded.config)
}

fn output_path(build_dir: &Path, platform: Platform, app_slug: &str) -> PathBuf {
    build_dir
        .join(platform.directory_name())
        .join(platform.output_name(app_slug))
}

fn write_build_metadata(input: &Path, project: &Path, metadata: &BuildMetadata<'_>) -> Result<()> {
    let (app_name, app_slug) = metadata.app;
    let metadata_dir = input.join("metadata");
    let values = [
        ("app-name", app_name.to_owned()),
        ("app-slug", app_slug.to_owned()),
        ("identifier", metadata.identifier.to_owned()),
        ("host", format!("{app_slug}.tokamak.local")),
        (
            "platform",
            metadata
                .manifest
                .target
                .platform()
                .directory_name()
                .to_owned(),
        ),
        ("target", metadata.manifest.target.to_string()),
        ("project-dir", project.display().to_string()),
    ];
    for (name, value) in values {
        fs::write(metadata_dir.join(name), value)?;
    }
    if let Some(version) = metadata.version {
        fs::write(metadata_dir.join("version"), version)?;
    }
    if let Some((endpoint, session_token)) = metadata.development {
        fs::write(metadata_dir.join("dev-endpoint"), endpoint)?;
        fs::write(metadata_dir.join("dev-session-token"), session_token)?;
    }
    if let Some(device_id) = metadata.device_id {
        fs::write(metadata_dir.join("device-id"), device_id)?;
    }
    Ok(())
}

const PLATFORM_PACK_PATH_ENV: &str = "TOKAMAK_PLATFORM_PACK_PATH";

fn resolve_manifest(platform: Platform, explicit_dir: Option<&Path>) -> Result<PathBuf> {
    if let Some(directory) = explicit_dir {
        if directory.is_file() {
            bail!(
                "--platform-pack must point to a platform-pack directory, not a manifest file: {}",
                directory.display()
            );
        }
        if !directory.is_dir() {
            bail!("platform-pack directory not found: {}", directory.display());
        }
        let manifest = directory.join(MANIFEST_FILE);
        if manifest.is_file() {
            return Ok(manifest);
        }
        bail!("platform-pack manifest not found: {}", manifest.display());
    }

    let target = platform.default_target().map_err(anyhow::Error::from)?;
    if let Some(roots) = env::var_os(PLATFORM_PACK_PATH_ENV).filter(|roots| !roots.is_empty()) {
        return env::split_paths(&roots)
            .find_map(|root| manifest_in(&root, target))
            .with_context(|| {
                format!(
                    "{PLATFORM_PACK_PATH_ENV} does not contain a platform pack for {target}; npm installs @tokamakdev/platform-{target} with @tokamakdev/tok on hosts that can build it"
                )
            });
    }

    if let Some(manifest) = bundled_manifest(target).or_else(|| installed_manifest(target)) {
        return Ok(manifest);
    }

    bail!(
        "no platform pack found for {target}; install @tokamakdev/tok with npm, run the tokamak installer, pass --platform-pack, or build one with `cargo run -p xtask -- platform-pack --target {target}`"
    )
}

fn manifest_in(root: &Path, target: Target) -> Option<PathBuf> {
    let manifest = root.join(target.to_string()).join(MANIFEST_FILE);
    manifest.is_file().then_some(manifest)
}

/// The manifest under the installer's per-user data directory.
fn installed_manifest(target: Target) -> Option<PathBuf> {
    let root = env::home_dir()?.join(".local/share/tokamak/platform-packs");
    manifest_in(&root, target)
}

fn bundled_manifest(target: Target) -> Option<PathBuf> {
    let exe = env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    [
        exe_dir.join("platform-packs"),
        exe_dir.join("../share/tokamak/platform-packs"),
        exe_dir.join("../Resources/platform-packs"),
    ]
    .into_iter()
    .find_map(|root| manifest_in(&root, target))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{resolve_app, resolve_identifier, resolve_manifest};
    use tokamak::{PlatformValues, TokamakConfig};
    use tokamak_cli::{MANIFEST_FILE, Platform};

    #[test]
    fn resolves_configured_platform_names_and_worker_fallback() {
        let config = TokamakConfig {
            name: PlatformValues {
                default: Some("My App".to_owned()),
                ios: Some("Myapp Pro".to_owned()),
                ..PlatformValues::default()
            },
            ..TokamakConfig::default()
        };

        assert_eq!(
            resolve_app(&config, "worker-name", Platform::Ios),
            ("Myapp Pro".to_owned(), "myapp-pro".to_owned())
        );
        assert_eq!(
            resolve_app(&config, "worker-name", Platform::Macos),
            ("My App".to_owned(), "my-app".to_owned())
        );
        assert_eq!(
            resolve_app(&TokamakConfig::default(), "worker-name", Platform::Windows),
            ("worker-name".to_owned(), "worker-name".to_owned())
        );
    }

    #[test]
    fn resolves_configured_identifiers_and_platform_defaults()
    -> Result<(), Box<dyn std::error::Error>> {
        let config = TokamakConfig {
            identifier: PlatformValues {
                default: Some("com.example.app".to_owned()),
                ios: Some("com.example.ios".to_owned()),
                ..PlatformValues::default()
            },
            ..TokamakConfig::default()
        };

        assert_eq!(
            resolve_identifier(&config, "demo-app", Platform::Ios)?,
            "com.example.ios"
        );
        assert_eq!(
            resolve_identifier(&config, "demo-app", Platform::Macos)?,
            "com.example.app"
        );
        Ok(())
    }

    #[test]
    fn preserves_platform_identifier_fallbacks() -> Result<(), Box<dyn std::error::Error>> {
        let config = TokamakConfig::default();

        assert_eq!(
            resolve_identifier(&config, "demo-app", Platform::Android)?,
            "com.tokamak.demo_app"
        );
        assert_eq!(
            resolve_identifier(&config, "demo-app", Platform::Ios)?,
            "com.tokamak.demo-app"
        );
        Ok(())
    }

    #[test]
    fn resolves_an_explicit_platform_pack_directory() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let manifest = directory.path().join(MANIFEST_FILE);
        fs::write(&manifest, "manifest")?;

        assert_eq!(
            resolve_manifest(Platform::Macos, Some(directory.path()))?,
            manifest
        );
        Ok(())
    }

    #[test]
    fn rejects_an_explicit_manifest_file() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let manifest = directory.path().join(MANIFEST_FILE);
        fs::write(&manifest, "manifest")?;

        let Err(error) = resolve_manifest(Platform::Macos, Some(&manifest)) else {
            return Err("a manifest file was accepted as a platform pack".into());
        };
        assert!(
            error
                .to_string()
                .contains("must point to a platform-pack directory")
        );
        Ok(())
    }
}
