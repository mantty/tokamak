//! Application lifecycle and packaged-worker startup.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::certificates::{Certificates, Renewal};
use crate::dev_proxy::{DevProxy, DevProxyConfig};
use crate::dispatcher::Dispatcher;
use crate::env_vars::load as load_environment;
use crate::gateway::{self, GatewayCertificates, GatewayConfig};
use crate::lifecycle_events::{Event, Events};
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
    /// [`Event`], including those raised from background threads.
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
        let handler = Dispatcher::new(worker, quickjs_config(&config.app, &config.state_dir)?)?;
        let gateway = start_gateway(&certificates, &config.host, handler)?;
        Ok(finish_start(events, config.host, certificates, gateway))
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
        let gateway = start_gateway(&certificates, &config.host, handler)?;
        Ok(finish_start(events, config.host, certificates, gateway))
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

fn finish_start(
    events: Events,
    host: String,
    certificates: Arc<Certificates>,
    gateway: gateway::Runtime,
) -> Runtime {
    let renewal = certificates.start_renewal(events.clone());
    events.emit(Event::Listening {
        port: gateway.port(),
    });
    Runtime {
        host,
        certificates,
        _renewal: renewal,
        events,
        gateway,
    }
}

fn start_gateway(
    certificates: &Certificates,
    host: &str,
    handler: Arc<dyn gateway::Handler>,
) -> Result<gateway::Runtime> {
    Ok(gateway::Runtime::start(
        handler,
        gateway_config(certificates, host),
    )?)
}

fn packaged_worker(app: &PackageLayout) -> Result<WorkerBundle> {
    if app.worker_manifest().is_file() {
        let manifest = read_worker_manifest(app)?;
        Ok(WorkerBundle::from_modules(
            manifest.entry,
            app.worker_modules(),
            app.bundle(),
        ))
    } else {
        let bytecode = decompress_worker_bundle(&std::fs::read(app.worker_bundle())?)?;
        Ok(WorkerBundle::from_bytecode(bytecode, app.bundle()))
    }
}

fn validate_worker(worker: &WorkerBundle) -> Result<()> {
    if worker.entry.is_empty() {
        return Err(crate::QuickJsError::Startup("Worker entry module is empty".to_owned()).into());
    }
    if let Some(bytecode) = &worker.legacy {
        if bytecode.is_empty() {
            return Err(crate::QuickJsError::Startup("Worker bytecode is empty".to_owned()).into());
        }
    } else if !worker
        .modules
        .join(format!("{}.qjs", worker.entry))
        .is_file()
    {
        return Err(crate::QuickJsError::Startup(format!(
            "Worker entry module is missing: {}",
            worker.entry
        ))
        .into());
    }
    Ok(())
}

fn gateway_config(certificates: &Certificates, host: &str) -> GatewayConfig {
    GatewayConfig {
        certificates: GatewayCertificates {
            ca: certificates.authority_path(),
            certificate: certificates.server_certificate_path(),
            private_key: certificates.server_key_path(),
        },
        host: host.to_owned(),
        port: 0,
        require_client_certificate: true,
    }
}

fn quickjs_config(app: &PackageLayout, state_dir: &Path) -> Result<RuntimeConfig> {
    Ok(RuntimeConfig {
        assets: app.serves_assets().then(|| Assets {
            manifest: app.asset_manifest(),
            root: app.assets(),
        }),
        cache: state_dir.join("cache"),
        environment: load_environment(app)?.vars,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::env_vars::{WorkerEnvironment, write as write_environment};
    use serde_json::json;

    use super::{Config, gateway_config, quickjs_config};
    use crate::certificates::Certificates;
    use crate::packaging::PackageLayout;

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    #[test]
    fn always_requires_a_client_certificate() -> TestResult {
        let directory = tempfile::tempdir()?;
        let certificates = Certificates::start(
            directory.path().to_path_buf(),
            "app.tokamak.local".to_owned(),
        )?;
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

        assert!(quickjs_config(&app, directory.path())?.assets.is_none());

        std::fs::write(app.asset_manifest(), "{}")?;

        assert!(quickjs_config(&app, directory.path())?.assets.is_some());
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
            },
        )?;

        assert_eq!(
            quickjs_config(&app, directory.path())?
                .environment
                .get("JSON"),
            Some(&json!({ "enabled": true }))
        );
        Ok(())
    }

    #[test]
    fn describes_where_an_app_lives() {
        let config = Config {
            app: PackageLayout::new("/apps/example"),
            state_dir: "/state".into(),
            host: "example.tokamak.local".to_owned(),
        };

        assert_eq!(
            config.app.worker_bundle(),
            std::path::Path::new("/apps/example/worker.bundle")
        );
    }
}
