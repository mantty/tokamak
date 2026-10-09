use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use tokamak_cli::{Platform, PlatformPackManifest};

use super::packs::PlatformPack;
use super::tokamak_config::{TokamakConfig, app_name_problem, slug};
use super::vite::VitePlugin;
use super::wrangler_config::{self, WranglerConfig};
use super::{paths, plugins, project, settings, worker};

pub(crate) struct BuildRequest {
    pub(crate) platforms: Vec<Platform>,
    pub(crate) project_dir: PathBuf,
    pub(crate) build_dir: Option<PathBuf>,
    pub(crate) platform_pack_dir: Option<PathBuf>,
    pub(crate) top: settings::TopOptions,
    pub(crate) platform_options: settings::PlatformOptions,
    /// `--build`; `TOKAMAK_BUILD` applies without it.
    pub(crate) build_command: Option<String>,
    pub(crate) skip_project_build: bool,
}

pub(crate) struct BuildSummary {
    pub(crate) platform: Platform,
    pub(crate) bundle_dir: PathBuf,
}

pub(crate) struct DevelopmentRequest<'a> {
    pub(crate) platform: Platform,
    /// The canonical project directory.
    pub(crate) project: &'a Path,
    pub(crate) pack: &'a PlatformPack,
    pub(crate) tokamak: &'a TokamakConfig,
    pub(crate) worker_name: &'a str,
    pub(crate) endpoint: &'a str,
    pub(crate) session_token: &'a str,
    pub(crate) device_id: &'a str,
    pub(crate) top: &'a settings::TopOptions,
    pub(crate) platform_options: &'a settings::PlatformOptions,
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
    /// The compiled Worker's package directory.
    worker: &'a Path,
    plugins: &'a [plugins::Plugin],
    version: &'a str,
}

/// A platform whose pack is loaded and whose settings are resolved.
struct PlatformBuild {
    platform: Platform,
    pack: PlatformPack,
    settings: settings::PlatformSettings,
    app: App,
}

/// The app on one platform: its display name, the name's slug, and its
/// application identifier.
struct App {
    name: String,
    slug: String,
    identifier: String,
}

struct BuildMetadata<'a> {
    app: &'a App,
    manifest: &'a PlatformPackManifest,
    version: Option<&'a str>,
    development: Option<(&'a str, &'a str)>,
    device_id: Option<&'a str>,
    /// The entry points of the optional runtime parts the app links, which
    /// its executable exports.
    exported_symbols: &'a [&'a str],
}

pub(crate) fn run(request: &BuildRequest) -> Result<Vec<BuildSummary>> {
    validate_request(request)?;
    let project = fs::canonicalize(&request.project_dir)?;
    let build_dir = project.join(request.build_dir.as_deref().unwrap_or(Path::new("build")));
    fs::create_dir_all(&build_dir)?;
    let build_dir = fs::canonicalize(build_dir)?;
    let packs = request
        .platforms
        .iter()
        .map(|platform| PlatformPack::load(*platform, request.platform_pack_dir.as_deref()))
        .collect::<Result<Vec<_>>>()?;
    let current_dir = env::current_dir()?;
    let plugin = VitePlugin::new(build_dir.join(".tokamak").join("vite"));
    for (platform, pack) in request.platforms.iter().zip(&packs) {
        check_settings(
            &request.top,
            &request.platform_options,
            *platform,
            &pack.manifest,
        )?;
    }
    if !request.skip_project_build {
        let command = match request.build_command.clone() {
            Some(command) => Some(command),
            None => settings::process_environment("TOKAMAK_BUILD")?,
        };
        plugin.clear()?;
        project::build(&project, command.as_deref(), plugin.environment())?;
    }
    let tokamak = plugin.config("the build")?.parse()?;
    let sources = settings::Sources::new(
        &request.top,
        &request.platform_options,
        Some(&tokamak),
        &current_dir,
    );
    let version = required_version(&sources)?;
    let wrangler = wrangler_config::load_config(
        &wrangler_config::deploy_config_path(&project)
            .context("find the Wrangler configuration the build generated")?,
    )?;
    project::check_output(&wrangler)?;
    warn_unsupported_bindings(&wrangler);
    let plugins = plugins::discover(&request.project_dir)?;
    let builds = request
        .platforms
        .iter()
        .zip(packs)
        .map(|(platform, pack)| {
            let settings = settings::resolve(&sources, *platform, &pack.manifest)?;
            plugins::check(&plugins, &pack.manifest)?;
            Ok(PlatformBuild {
                platform: *platform,
                app: resolve_app(&settings, &wrangler.name, *platform)?,
                settings,
                pack,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let worker = worker::compile(&build_dir.join(".tokamak").join("worker"), &wrangler)
        .context("compile the Worker")?;
    let context = BuildContext {
        build_dir: &build_dir,
        wrangler: &wrangler,
        worker: &worker,
        plugins: &plugins,
        version: &version,
    };

    builds
        .iter()
        .map(|build| build_platform(request, build, &context))
        .collect()
}

/// Check the options and environment variables for `platform` before the
/// project's command runs; the `tokamak()` options are read after it.
pub(crate) fn check_settings(
    top: &settings::TopOptions,
    platform_options: &settings::PlatformOptions,
    platform: Platform,
    manifest: &PlatformPackManifest,
) -> Result<()> {
    let current_dir = env::current_dir()?;
    let sources = settings::Sources::new(top, platform_options, None, &current_dir);
    settings::resolve(&sources, platform, manifest)?;
    Ok(())
}

pub(crate) fn run_development(request: &DevelopmentRequest<'_>) -> Result<DevelopmentSummary> {
    let current_dir = env::current_dir()?;
    let sources = settings::Sources::new(
        request.top,
        request.platform_options,
        Some(request.tokamak),
        &current_dir,
    );
    let manifest = &request.pack.manifest;
    let platform_settings = settings::resolve(&sources, request.platform, manifest)?;
    let plugins = plugins::discover(request.project)?;
    let app = resolve_app(&platform_settings, request.worker_name, request.platform)?;
    let version = settings::version(&sources)?;
    let build_dir = request.project.join("build");
    let (input, project) = prepare_platform_input(request.project, &build_dir, request.platform)?;
    plugins::stage(&plugins, manifest, &input.join("plugins"))
        .context("stage native plugin inputs")?;
    fs::write(input.join("app/.tokamak-development"), b"")?;
    write_build_metadata(
        &input,
        &project,
        &BuildMetadata {
            app: &app,
            manifest,
            version: version.as_deref(),
            development: Some((request.endpoint, request.session_token)),
            device_id: Some(request.device_id),
            exported_symbols: &[tokamak::DEVELOPMENT_ENTRY_POINT],
        },
    )
    .context("write development metadata")?;

    let bundle_dir = output_path(&build_dir, request.platform, &app.slug);
    request
        .pack
        .build(&input, &bundle_dir, &platform_settings.pack_environment)
        .with_context(|| {
            format!(
                "build {} development shell using platform-pack entrypoint",
                request.platform.display_name()
            )
        })?;
    Ok(DevelopmentSummary {
        platform: request.platform,
        bundle_dir,
        app_slug: app.slug,
        identifier: app.identifier,
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
    build: &PlatformBuild,
    context: &BuildContext<'_>,
) -> Result<BuildSummary> {
    let PlatformBuild {
        platform,
        pack,
        settings: platform_settings,
        app,
    } = build;
    let platform = *platform;
    let (input, project) =
        prepare_platform_input(&request.project_dir, context.build_dir, platform)?;

    worker::package(&input.join("app"), context.worker, context.wrangler)
        .context("prepare the tokamak application package")?;
    plugins::stage(context.plugins, &pack.manifest, &input.join("plugins"))
        .context("stage native plugin inputs")?;
    write_build_metadata(
        &input,
        &project,
        &BuildMetadata {
            app,
            manifest: &pack.manifest,
            version: Some(context.version),
            development: None,
            device_id: None,
            exported_symbols: exported_symbols(context.wrangler),
        },
    )
    .context("write platform build metadata")?;

    let output = output_path(context.build_dir, platform, &app.slug);
    pack.build(&input, &output, &platform_settings.pack_environment)
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

/// The app on `platform`, named by `settings` or else by the Worker name,
/// which must then be its own slug.
fn resolve_app(
    settings: &settings::PlatformSettings,
    worker_name: &str,
    platform: Platform,
) -> Result<App> {
    let (name, app_slug) = match settings.name.as_deref() {
        Some(name) => (name.to_owned(), slug(name)),
        None if slug(worker_name) == worker_name && app_name_problem(worker_name).is_none() => {
            (worker_name.to_owned(), worker_name.to_owned())
        }
        None => bail!(
            "{} has no name, and the Worker name {worker_name:?} is not a valid app name \
             (lowercase letters and digits joined by single hyphens, at most 63 characters); \
             set --name, TOKAMAK_NAME, or `name` in the tokamak() options",
            platform.display_name()
        ),
    };
    let identifier = resolve_identifier(settings.identifier.clone(), &app_slug, platform)?;
    Ok(App {
        name,
        slug: app_slug,
        identifier,
    })
}

fn resolve_identifier(
    identifier: Option<String>,
    app_slug: &str,
    platform: Platform,
) -> Result<String> {
    let identifier = identifier.unwrap_or_else(|| default_identifier(app_slug, platform));
    validate_platform_identifier(platform, identifier)
}

fn required_version(sources: &settings::Sources<'_>) -> Result<String> {
    settings::version(sources)?.ok_or_else(|| {
        anyhow::anyhow!(
            "tokamak version is required for `tok build`; set --version, TOKAMAK_VERSION, or `version` in the tokamak() options"
        )
    })
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

fn prepare_platform_input(
    project_dir: &Path,
    build_dir: &Path,
    platform: Platform,
) -> Result<(PathBuf, PathBuf)> {
    let project = fs::canonicalize(project_dir)
        .with_context(|| format!("resolve project directory: {}", project_dir.display()))?;
    let staging = build_dir.join(".tokamak").join(platform.directory_name());
    let input = staging.join("input");
    paths::reset_path(&input)
        .with_context(|| format!("reset build input directory: {}", input.display()))?;
    fs::create_dir_all(input.join("metadata")).with_context(|| {
        format!(
            "create build input directory: {}",
            input.join("metadata").display()
        )
    })?;
    fs::create_dir_all(input.join("app"))?;
    Ok((input, project))
}

fn output_path(build_dir: &Path, platform: Platform, app_slug: &str) -> PathBuf {
    build_dir
        .join(platform.directory_name())
        .join(platform.output_name(app_slug))
}

fn write_build_metadata(input: &Path, project: &Path, metadata: &BuildMetadata<'_>) -> Result<()> {
    let app = metadata.app;
    let metadata_dir = input.join("metadata");
    let values = [
        ("app-name", app.name.clone()),
        ("app-slug", app.slug.clone()),
        ("identifier", app.identifier.clone()),
        ("host", format!("{}.tokamak.local", app.slug)),
        (
            "platform",
            metadata.manifest.target.platform().namespace().to_owned(),
        ),
        ("target", metadata.manifest.target.to_string()),
        ("project-dir", project.display().to_string()),
        (
            "exported-symbols",
            metadata
                .exported_symbols
                .iter()
                .flat_map(|symbol| [*symbol, "\n"])
                .collect(),
        ),
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

/// The entry points of the optional runtime parts an app with `wrangler`'s
/// bindings links.
fn exported_symbols(wrangler: &WranglerConfig) -> &'static [&'static str] {
    if wrangler.storage.is_empty() {
        &[]
    } else {
        &[tokamak::STORAGE_ENTRY_POINT]
    }
}

fn warn_unsupported_bindings(wrangler: &WranglerConfig) {
    if wrangler.bindings.is_empty() {
        return;
    }
    let mut warning = String::from(
        "\nWARNING: this app declares bindings that the packaged app does not provide.\n\n\
         The build will continue. Avoid these bindings when running on tokamak,\n\
         or guard their use with the appropriate platform or feature flag.\n\n\
         Unsupported bindings:\n\n",
    );
    for binding in &wrangler.bindings {
        let _ = writeln!(
            &mut warning,
            "  - {} ({}): the packaged app does not provide {}",
            binding.name, binding.kind, binding.feature
        );
    }
    eprintln!("{warning}");
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;

    use super::{exported_symbols, resolve_app, resolve_identifier};
    use crate::settings::PlatformSettings;
    use crate::wrangler_config;
    use tokamak_cli::Platform;

    #[test]
    fn exports_the_storage_entry_point_while_any_storage_binding_is_declared()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("wrangler.json");
        for (bindings, symbols) in [
            ("", &[][..]),
            (
                r#", "kv_namespaces": [{ "binding": "KV", "id": "kv" }]"#,
                &["tokamak_storage"][..],
            ),
            (
                r#", "d1_databases": [{ "binding": "DB", "database_id": "db" }]"#,
                &["tokamak_storage"][..],
            ),
            (
                r#", "r2_buckets": [{ "binding": "FILES", "bucket_name": "files" }]"#,
                &["tokamak_storage"][..],
            ),
        ] {
            fs::write(
                &path,
                format!(r#"{{ "name": "app", "main": "worker.js"{bindings} }}"#),
            )?;
            assert_eq!(
                exported_symbols(&wrangler_config::load_config(&path)?),
                symbols
            );
        }
        Ok(())
    }

    #[test]
    fn resolves_names_with_the_worker_name_as_fallback() -> Result<(), Box<dyn std::error::Error>> {
        let named = |name: Option<&str>| PlatformSettings {
            name: name.map(str::to_owned),
            identifier: None,
            pack_environment: BTreeMap::new(),
        };
        let app = resolve_app(&named(Some("Myapp Pro")), "worker_name", Platform::Macos)?;
        assert_eq!(
            (
                app.name.as_str(),
                app.slug.as_str(),
                app.identifier.as_str()
            ),
            ("Myapp Pro", "myapp-pro", "com.tokamak.myapp-pro")
        );
        let app = resolve_app(&named(None), "worker-name", Platform::Macos)?;
        assert_eq!(
            (app.name.as_str(), app.slug.as_str()),
            ("worker-name", "worker-name")
        );
        for worker_name in [
            "",
            "worker_name",
            "Upper",
            "-leading",
            "trailing-",
            "double--hyphen",
            &"a".repeat(64),
        ] {
            assert!(
                resolve_app(&named(None), worker_name, Platform::Macos).is_err(),
                "accepted {worker_name}"
            );
        }
        Ok(())
    }

    #[test]
    fn resolves_identifiers_with_platform_defaults() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            resolve_identifier(
                Some("com.example.ios".to_owned()),
                "demo-app",
                Platform::Ios
            )?,
            "com.example.ios"
        );
        assert_eq!(
            resolve_identifier(None, "demo-app", Platform::Android)?,
            "com.tokamak.demo_app"
        );
        assert_eq!(
            resolve_identifier(None, "demo-app", Platform::Ios)?,
            "com.tokamak.demo-app"
        );
        assert!(
            resolve_identifier(Some("com..app".to_owned()), "demo-app", Platform::Ios).is_err()
        );
        Ok(())
    }
}
