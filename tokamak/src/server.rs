//! Application lifecycle and packaged-worker startup.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::assets::Assets;
use crate::certificates::{Certificates, Renewal};
use crate::dispatcher::Dispatcher;
use crate::env_vars::{StorageBinding, load as load_environment};
use crate::gateway;
use crate::lifecycle_events::{Event, Events};
use crate::linked::StorageRuntime;
use crate::packaging::{PackageLayout, read_worker_manifest};
use crate::plugin_calls::{PluginCalls, PluginHandler};
use crate::quickjs::{RuntimeConfig, WorkerBundle};
use crate::worker_events::WorkerEvents;

use crate::Result;

/// What a runtime serves and where it keeps generated state.
#[derive(Clone, Debug)]
pub struct Config {
    /// Packaged application contents.
    pub app: PackageLayout,
    /// Writable per-app directory holding generated certificates.
    pub state_dir: PathBuf,
    /// Writable per-app directory holding the stores behind storage bindings.
    pub storage_dir: PathBuf,
    /// Stable HTTPS host the shell's `WebView` loads.
    pub host: String,
    /// Whether the app is in the foreground as the runtime starts.
    pub foreground: bool,
    /// The shell's plugins, which the Worker calls; without them, the Worker's
    /// plugin calls fail with `NotSupportedError`.
    pub plugins: Option<Arc<dyn PluginHandler>>,
}

/// Host development-server connection settings.
#[derive(Clone, Debug)]
pub struct DevProxyConfig {
    /// HTTP or HTTPS endpoint exposed by the host development supervisor.
    pub endpoint: String,
    /// Credential identifying the current development session.
    pub session_token: String,
}

/// Configuration for a runtime that forwards requests to a host development server.
#[derive(Clone, Debug)]
pub struct DevelopmentConfig {
    /// Writable per-app directory holding generated certificates.
    pub state_dir: PathBuf,
    /// Stable HTTPS host the shell's `WebView` loads.
    pub host: String,
    /// Host development-server connection.
    pub proxy: DevProxyConfig,
    /// Whether the app is in the foreground as the runtime starts.
    pub foreground: bool,
    /// The shell's plugins, which the development Worker calls.
    pub plugins: Option<Arc<dyn PluginHandler>>,
}

/// A running tokamak service.
///
/// Dropping the runtime stops request handling and background renewal.
#[derive(Debug)]
pub struct Runtime {
    host: String,
    certificates: Arc<Certificates>,
    _renewal: Renewal,
    events: Arc<WorkerEvents>,
    plugins: Arc<PluginCalls>,
    gateway: gateway::Runtime,
}

impl Runtime {
    /// Start the gateway and JavaScript runtime for a packaged app, and
    /// deliver the `start` event in the background.
    ///
    /// Blocks until the gateway is listening. `listener` receives every
    /// [`Event`], including those raised from background threads, so it must
    /// return promptly and must not panic.
    ///
    /// # Errors
    ///
    /// Returns an error when certificates, the packaged bundle, or `QuickJS`
    /// startup fail.
    pub fn start(config: Config, listener: impl Fn(Event) + Send + Sync + 'static) -> Result<Self> {
        let events = Events::new(listener);
        events.emit(Event::Starting);
        let certificates = Arc::new(Certificates::start(
            config.state_dir.clone(),
            config.host.clone(),
        )?);
        let worker = WorkerBundle::new(read_worker_manifest(&config.app)?, config.app.clone());
        validate_worker(&worker)?;
        let plugins = PluginCalls::new(config.plugins.clone(), config.foreground);
        let handler = Dispatcher::new(worker, quickjs_config(&config, Arc::clone(&plugins))?);
        finish_start(
            events,
            config.host,
            config.foreground,
            certificates,
            handler,
            plugins,
        )
    }

    /// Start a runtime that forwards requests to a host development server.
    ///
    /// The host connection is supplied by the development supervisor. The
    /// runtime does not start or inspect the host framework process.
    ///
    /// # Errors
    ///
    /// Returns an error when the app was linked without the development part,
    /// or when certificates, the proxy endpoint, or the gateway cannot start.
    pub fn start_development(
        config: DevelopmentConfig,
        listener: impl Fn(Event) + Send + Sync + 'static,
    ) -> Result<Self> {
        let development = crate::linked::development().ok_or_else(|| {
            io::Error::other(
                "the app starts in development, but its runtime was linked without development",
            )
        })?;
        let events = Events::new(listener);
        events.emit(Event::Starting);
        let certificates = Arc::new(Certificates::start(config.state_dir, config.host.clone())?);
        let plugins = PluginCalls::new(config.plugins, config.foreground);
        let handler = (development.handler)(&config.proxy, Arc::clone(&plugins))?;
        finish_start(
            events,
            config.host,
            config.foreground,
            certificates,
            handler,
            plugins,
        )
    }

    /// The host the `WebView` loads.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The loopback port the gateway bound.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.gateway.port()
    }

    /// Wait for the loopback gateway to respond and return its current port.
    ///
    /// # Errors
    ///
    /// Returns an error when the gateway does not recover within its timeout.
    pub fn restore_gateway(&self) -> Result<u16> {
        Ok(self.gateway.restore_gateway()?)
    }

    /// Deliver the event `name`, whose JSON is `event`, to the Worker's
    /// listeners and return the reply as JSON: the first value a listener
    /// returns other than `undefined`, or `null`.
    ///
    /// Blocks until the listeners and their `waitUntil` promises settle, or
    /// `timeout` passes. Events wait for `start`. `resume` and `suspend` are
    /// delivered one at a time, so a shell emits them from one thread to keep
    /// their order, each after [`Runtime::set_foreground`] reports a change.
    /// A packaged app runs no Worker for an event without listeners.
    ///
    /// # Errors
    ///
    /// Returns an error when `name` is `start`, the Worker cannot run, or
    /// `timeout` passes before every listener returns.
    pub fn emit(&self, name: &str, event: &str, timeout: Duration) -> Result<String> {
        self.events
            .emit(name, event, timeout)
            .map_err(crate::Error::Event)
    }

    /// Record whether the app is in the foreground, which the Worker's
    /// `getLifecycleStage()` reports from then on, and return whether that
    /// changed. A shell calls it as the platform reports each change, and
    /// emits `resume` or `suspend` when it returns true.
    pub fn set_foreground(&self, foreground: bool) -> bool {
        self.plugins.set_foreground(foreground)
    }

    /// Whether the app is in the foreground, as last recorded. The shells'
    /// plugin hosts read it to decide whether a prompt can show.
    #[must_use]
    pub fn is_foreground(&self) -> bool {
        self.plugins.is_foreground()
    }

    /// Pass a plugin's JSON `result` to the Worker's call or subscription
    /// `id`, as the shell's [`PluginHandler`] answers it. The runtime ignores
    /// results for calls and subscriptions that have ended.
    pub fn reply(&self, id: u64, result: &str) {
        self.plugins.reply(id, result);
    }

    /// Certificate material for the shell's TLS challenge callbacks.
    #[must_use]
    pub fn certificates(&self) -> Arc<Certificates> {
        Arc::clone(&self.certificates)
    }
}

/// Start the gateway serving `handler`, certificate renewal, and the Worker's
/// events.
fn finish_start(
    events: Events,
    host: String,
    foreground: bool,
    certificates: Arc<Certificates>,
    handler: Arc<dyn gateway::Handler>,
    plugins: Arc<PluginCalls>,
) -> Result<Runtime> {
    let gateway = gateway::Runtime::start(
        handler,
        Arc::clone(&certificates),
        host.clone(),
        events.clone(),
    )?;
    let renewal = certificates.start_renewal(events);
    Ok(Runtime {
        host,
        certificates,
        _renewal: renewal,
        events: WorkerEvents::start(Arc::clone(gateway.shared()), foreground),
        plugins,
        gateway,
    })
}

fn validate_worker(worker: &WorkerBundle) -> Result<()> {
    let message = if worker.entry.is_empty() {
        "Worker entry module is empty".to_owned()
    } else if !worker.app.worker_module(&worker.entry).is_file() {
        format!("Worker entry module is missing: {}", worker.entry)
    } else {
        return Ok(());
    };
    Err(crate::QuickJsError::Startup(message).into())
}

/// Set to "true" in a packaged app's Worker environment.
const RUNTIME_MARKER: &str = "TOKAMAK_RUNTIME";

fn quickjs_config(config: &Config, plugins: Arc<PluginCalls>) -> Result<RuntimeConfig> {
    let app = &config.app;
    let environment = load_environment(app)?;
    let storage = open_storage(config, &environment.storage)?;
    let mut vars = environment.vars;
    vars.insert(RUNTIME_MARKER.to_owned(), "true".into());
    Ok(RuntimeConfig {
        assets: app
            .serves_assets()
            .then(|| Assets::open(app))
            .transpose()?
            .map(Arc::new),
        cache: Arc::default(),
        environment: vars,
        storage,
        plugins,
    })
}

/// The stores the app's storage `bindings` name. An app without storage
/// bindings keeps no stores, so its storage directories are removed.
fn open_storage(
    config: &Config,
    bindings: &[StorageBinding],
) -> Result<Option<Arc<dyn StorageRuntime>>> {
    let scratch = config.state_dir.join("storage");
    if bindings.is_empty() {
        remove_directory(&config.storage_dir)?;
        remove_directory(&scratch)?;
        return Ok(None);
    }
    let entry = crate::linked::storage().ok_or_else(|| {
        io::Error::other(
            "the app declares storage bindings, but its runtime was linked without storage",
        )
    })?;
    Ok(Some((entry.open)(
        &config.storage_dir,
        &scratch,
        &config.app,
        bindings,
    )?))
}

fn remove_directory(directory: &Path) -> io::Result<()> {
    match fs::remove_dir_all(directory) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::env_vars::{StorageBinding, WorkerEnvironment, write as write_environment};
    use serde_json::json;

    use super::{Config, DevProxyConfig, DevelopmentConfig, Runtime, quickjs_config};

    fn config(root: &std::path::Path) -> Config {
        Config {
            app: PackageLayout::new(root),
            state_dir: root.join("state"),
            storage_dir: root.join("storage"),
            host: "example.tokamak.local".to_owned(),
            foreground: true,
            plugins: None,
        }
    }
    use crate::packaging::{
        AssetManifest, HtmlHandling, NotFoundHandling, PackageLayout, write_asset_manifest,
    };

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    fn plugins() -> std::sync::Arc<crate::plugin_calls::PluginCalls> {
        crate::plugin_calls::PluginCalls::new(None, true)
    }

    #[test]
    fn serves_assets_only_when_the_app_packages_them() -> TestResult {
        let directory = tempfile::tempdir()?;
        let app = PackageLayout::new(directory.path());
        write_environment(&app, &WorkerEnvironment::default())?;

        assert!(
            quickjs_config(&config(directory.path()), plugins())?
                .assets
                .is_none()
        );

        write_asset_manifest(
            &app,
            &AssetManifest {
                binding: "ASSETS".to_owned(),
                files: BTreeMap::new(),
                html_handling: HtmlHandling::default(),
                not_found_handling: NotFoundHandling::default(),
            },
        )?;

        assert!(
            quickjs_config(&config(directory.path()), plugins())?
                .assets
                .is_some()
        );
        Ok(())
    }

    #[test]
    fn loads_packaged_worker_vars() -> TestResult {
        let directory = tempfile::tempdir()?;
        let app = PackageLayout::new(directory.path());
        write_environment(
            &app,
            &WorkerEnvironment {
                vars: BTreeMap::from([("JSON".to_owned(), json!({ "enabled": true }))]),
                storage: Vec::new(),
            },
        )?;

        assert_eq!(
            quickjs_config(&config(directory.path()), plugins())?
                .environment
                .get("JSON"),
            Some(&json!({ "enabled": true }))
        );
        Ok(())
    }

    #[test]
    fn removes_the_stores_of_an_app_without_storage_bindings() -> TestResult {
        let directory = tempfile::tempdir()?;
        let app = PackageLayout::new(directory.path());
        write_environment(&app, &WorkerEnvironment::default())?;
        let config = config(directory.path());
        std::fs::create_dir_all(config.storage_dir.join("d1"))?;
        std::fs::write(config.storage_dir.join("d1/app.sqlite"), "")?;
        std::fs::create_dir_all(config.state_dir.join("storage/r2/files"))?;

        let runtime = quickjs_config(&config, plugins())?;

        assert!(runtime.storage.is_none());
        assert!(!config.storage_dir.exists());
        assert!(!config.state_dir.join("storage").exists());
        assert!(quickjs_config(&config, plugins())?.storage.is_none());
        Ok(())
    }

    // Unit test executables do not export the storage part's entry point, as
    // an app without storage bindings does not.
    #[test]
    fn refuses_storage_bindings_when_linked_without_storage() -> TestResult {
        let directory = tempfile::tempdir()?;
        let app = PackageLayout::new(directory.path());
        write_environment(
            &app,
            &WorkerEnvironment {
                vars: BTreeMap::new(),
                storage: vec![StorageBinding::Kv {
                    name: "SESSION".to_owned(),
                    id: "session".to_owned(),
                }],
            },
        )?;
        let config = config(directory.path());
        std::fs::create_dir_all(config.storage_dir.join("kv"))?;
        std::fs::write(config.storage_dir.join("kv/session.sqlite"), "")?;

        let error = quickjs_config(&config, plugins())
            .err()
            .ok_or("storage opened")?;

        assert!(
            error.to_string().contains("linked without storage"),
            "{error}"
        );
        assert!(config.storage_dir.join("kv/session.sqlite").is_file());
        Ok(())
    }

    // Unit test executables do not export the development part's entry point,
    // as an app built for release does not.
    #[test]
    fn refuses_development_when_linked_without_development() -> TestResult {
        let directory = tempfile::tempdir()?;

        let error = Runtime::start_development(
            DevelopmentConfig {
                state_dir: directory.path().join("state"),
                host: "example.tokamak.local".to_owned(),
                proxy: DevProxyConfig {
                    endpoint: "http://127.0.0.1:1".to_owned(),
                    session_token: "token".to_owned(),
                },
                foreground: true,
                plugins: None,
            },
            |_| {},
        )
        .err()
        .ok_or("the development runtime started")?;

        assert!(
            error.to_string().contains("linked without development"),
            "{error}"
        );
        assert!(!directory.path().join("state").exists());
        Ok(())
    }

    #[test]
    fn marks_a_packaged_worker_environment() -> TestResult {
        let directory = tempfile::tempdir()?;
        let app = PackageLayout::new(directory.path());
        write_environment(&app, &WorkerEnvironment::default())?;

        assert_eq!(
            quickjs_config(&config(directory.path()), plugins())?
                .environment
                .get("TOKAMAK_RUNTIME"),
            Some(&json!("true"))
        );
        Ok(())
    }
}
