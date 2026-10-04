//! Application lifecycle and packaged-worker startup.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::certificates::{Certificates, Renewal};
use crate::dev_proxy::{DevProxy, DevProxyConfig};
use crate::dispatcher::Dispatcher;
use crate::env_vars::{StorageBinding, load as load_environment};
use crate::gateway::{self, GatewayConfig};
use crate::lifecycle_events::{Event, Events};
use crate::linked::StorageRuntime;
use crate::packaging::{PackageLayout, decompress_worker_bundle, read_worker_manifest};
use crate::quickjs::{Assets, RuntimeConfig, WorkerBundle};

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
}

/// A running tokamak service.
///
/// Dropping the runtime stops request handling and background renewal.
#[derive(Debug)]
pub struct Runtime {
    host: String,
    certificates: Arc<Certificates>,
    _renewal: Renewal,
    events: Events,
    gateway: gateway::Runtime,
}

impl Runtime {
    /// Start the gateway and JavaScript runtime for a packaged app.
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
        let worker = packaged_worker(&config.app)?;
        validate_worker(&worker)?;
        let handler = Dispatcher::new(worker, quickjs_config(&config)?)?;
        finish_start(events, config.host, certificates, handler)
    }

    /// Start a runtime that forwards requests to a host development server.
    ///
    /// The host connection is supplied by the development supervisor. The
    /// runtime does not start or inspect the host framework process.
    ///
    /// # Errors
    ///
    /// Returns an error when certificates, the proxy endpoint, or the gateway
    /// cannot start.
    pub fn start_development(
        config: DevelopmentConfig,
        listener: impl Fn(Event) + Send + Sync + 'static,
    ) -> Result<Self> {
        let events = Events::new(listener);
        events.emit(Event::Starting);
        let certificates = Arc::new(Certificates::start(config.state_dir, config.host.clone())?);
        let handler = DevProxy::new(&config.proxy)?;
        finish_start(events, config.host, certificates, handler)
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

    /// Post JSON `body` to the Worker's `/tokamak/<name>` endpoint and return
    /// the response body.
    ///
    /// # Errors
    ///
    /// Returns an error unless the Worker responds 200 within `timeout`.
    pub fn call(&self, name: &str, body: &str, timeout: Duration) -> Result<Vec<u8>> {
        Ok(self.gateway.call(name, body, timeout)?)
    }

    /// Certificate material for the shell's TLS challenge callbacks.
    #[must_use]
    pub fn certificates(&self) -> Arc<Certificates> {
        Arc::clone(&self.certificates)
    }

    /// Stop new request dispatch and quiesce gateway connections.
    pub fn suspend(&self) {
        self.gateway.suspend();
        self.events.emit(Event::Suspended);
    }

    /// Resume request dispatch, renewing certificates that fell due.
    ///
    /// # Errors
    ///
    /// Returns an error when renewal fails or the gateway rejects the
    /// transition.
    pub fn resume(&self) -> Result<()> {
        self.certificates.refresh()?;
        self.gateway.resume();
        self.events.emit(Event::Resumed);
        Ok(())
    }
}

/// Start the gateway serving `handler`, then certificate renewal.
fn finish_start(
    events: Events,
    host: String,
    certificates: Arc<Certificates>,
    handler: Arc<dyn gateway::Handler>,
) -> Result<Runtime> {
    let config = gateway_config(&certificates, &host);
    let gateway = gateway::Runtime::start(handler, config, events.clone())?;
    let renewal = certificates.start_renewal(events.clone());
    events.emit(Event::Listening {
        port: gateway.port(),
    });
    Ok(Runtime {
        host,
        certificates,
        _renewal: renewal,
        events,
        gateway,
    })
}

fn packaged_worker(app: &PackageLayout) -> Result<WorkerBundle> {
    if app.worker_manifest().is_file() {
        Ok(WorkerBundle::from_modules(
            read_worker_manifest(app)?,
            app.worker_modules(),
            app.bundle(),
        ))
    } else {
        let bytecode = decompress_worker_bundle(&std::fs::read(app.worker_bundle())?)?;
        Ok(WorkerBundle::from_bytecode(bytecode, app.bundle()))
    }
}

fn validate_worker(worker: &WorkerBundle) -> Result<()> {
    let entry_module = worker.modules.join(format!("{}.qjs", worker.entry));
    let message = match &worker.legacy {
        _ if worker.entry.is_empty() => "Worker entry module is empty".to_owned(),
        Some(bytecode) if bytecode.is_empty() => "Worker bytecode is empty".to_owned(),
        None if !entry_module.is_file() => {
            format!("Worker entry module is missing: {}", worker.entry)
        }
        _ => return Ok(()),
    };
    Err(crate::QuickJsError::Startup(message).into())
}

fn gateway_config(certificates: &Arc<Certificates>, host: &str) -> GatewayConfig {
    let certificates = Arc::clone(certificates);
    GatewayConfig {
        tls: Arc::new(move || {
            certificates
                .server_config()
                .map_err(|error| crate::QuickJsError::Tls(error.to_string()))
        }),
        host: host.to_owned(),
        port: 0,
        require_client_certificate: true,
    }
}

/// Set to "true" in a packaged app's Worker environment.
const RUNTIME_MARKER: &str = "TOKAMAK_RUNTIME";

fn quickjs_config(config: &Config) -> Result<RuntimeConfig> {
    let app = &config.app;
    let environment = load_environment(app)?;
    let storage = open_storage(config, &environment.storage)?;
    let mut vars = environment.vars;
    vars.insert(RUNTIME_MARKER.to_owned(), "true".into());
    Ok(RuntimeConfig {
        assets: app.serves_assets().then(|| Assets {
            manifest: app.asset_manifest(),
            root: app.assets(),
        }),
        cache: config.state_dir.join("cache"),
        environment: vars,
        storage,
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

    use super::{Config, gateway_config, quickjs_config};

    fn config(root: &std::path::Path) -> Config {
        Config {
            app: PackageLayout::new(root),
            state_dir: root.join("state"),
            storage_dir: root.join("storage"),
            host: "example.tokamak.local".to_owned(),
        }
    }
    use crate::certificates::Certificates;
    use crate::packaging::PackageLayout;
    use std::sync::Arc;

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    #[test]
    fn always_requires_a_client_certificate() -> TestResult {
        let directory = tempfile::tempdir()?;
        let certificates = Arc::new(Certificates::start(
            directory.path().to_path_buf(),
            "app.tokamak.local".to_owned(),
        )?);
        let app = PackageLayout::new(directory.path());
        write_environment(&app, &WorkerEnvironment::default())?;

        let config = gateway_config(&certificates, "app.tokamak.local");

        assert!(config.require_client_certificate);
        assert_eq!(config.port, 0);
        Ok(())
    }

    #[test]
    fn serves_assets_only_when_the_app_packages_them() -> TestResult {
        let directory = tempfile::tempdir()?;
        let app = PackageLayout::new(directory.path());
        write_environment(&app, &WorkerEnvironment::default())?;

        assert!(quickjs_config(&config(directory.path()))?.assets.is_none());

        std::fs::write(app.asset_manifest(), "{}")?;

        assert!(quickjs_config(&config(directory.path()))?.assets.is_some());
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
            quickjs_config(&config(directory.path()))?
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
        std::fs::create_dir_all(config.state_dir.join("cache"))?;

        let runtime = quickjs_config(&config)?;

        assert!(runtime.storage.is_none());
        assert!(!config.storage_dir.exists());
        assert!(!config.state_dir.join("storage").exists());
        assert!(config.state_dir.join("cache").is_dir());
        assert!(quickjs_config(&config)?.storage.is_none());
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

        let error = quickjs_config(&config).err().ok_or("storage opened")?;

        assert!(
            error.to_string().contains("linked without storage"),
            "{error}"
        );
        assert!(config.storage_dir.join("kv/session.sqlite").is_file());
        Ok(())
    }

    #[test]
    fn marks_a_packaged_worker_environment() -> TestResult {
        let directory = tempfile::tempdir()?;
        let app = PackageLayout::new(directory.path());
        write_environment(&app, &WorkerEnvironment::default())?;

        assert_eq!(
            quickjs_config(&config(directory.path()))?
                .environment
                .get("TOKAMAK_RUNTIME"),
            Some(&json!("true"))
        );
        Ok(())
    }

    #[test]
    fn describes_where_an_app_lives() {
        let config = config(std::path::Path::new("/apps/example"));

        assert_eq!(
            config.app.worker_bundle(),
            std::path::Path::new("/apps/example/worker.bundle")
        );
    }
}
