use std::collections::BTreeMap;
use std::sync::Arc;

use super::{Config, Result, open_storage, service_config, validate_worker};
use crate::dispatcher::Dispatcher;
use crate::env_vars::WorkerEnvironment;
use crate::packaging::PackageLayout;
use crate::plugin_calls::PluginCalls;
use crate::quickjs::{Error, WorkerBundle};
use crate::services::{Registry, Service, Target};

/// The generated entrypoint through which the host delivers native events.
const EVENT_ENTRYPOINT: &str = "TokamakEvents";

struct Package {
    worker: WorkerBundle,
    environment: WorkerEnvironment,
}

impl Package {
    fn read(app: &PackageLayout) -> Result<Self> {
        let worker = WorkerBundle::new(crate::packaging::read_worker_manifest(app)?, app.clone());
        validate_worker(&worker)?;
        Ok(Self {
            worker,
            environment: crate::env_vars::load(app)?,
        })
    }
}

pub(super) fn load(config: &Config, plugins: &Arc<PluginCalls>) -> Result<Arc<Dispatcher>> {
    let mut packages = BTreeMap::from([("main", Package::read(&config.app)?)]);
    let native = config.app.tokamak_worker();
    let split = native.root().try_exists()?;
    if split {
        packages.insert("tokamak", Package::read(&native)?);
    }
    validate_bindings(&packages)?;
    let retained: Vec<_> = packages
        .values()
        .flat_map(|package| package.environment.storage.iter().cloned())
        .collect();
    let mut stores = open_storage(config, &retained)?;
    let mut services = BTreeMap::new();
    for (name, package) in packages {
        let bundle = package.worker;
        let environment = package.environment;
        let native = (name == "tokamak").then(|| Arc::clone(plugins));
        let storage = stores
            .as_mut()
            .filter(|_| !environment.storage.is_empty())
            .map(|stores| stores.bind(&bundle.app, &environment.storage))
            .transpose()?;
        let config = service_config(&bundle.app, &environment, storage, native)?;
        services.insert(
            name.into(),
            Service {
                bundle,
                config,
                bindings: environment
                    .services
                    .iter()
                    .map(|(binding, target)| {
                        (
                            binding.clone(),
                            Target::new(&target.service, &target.entrypoint),
                        )
                    })
                    .collect(),
            },
        );
    }
    let registry = Arc::new(Registry::new(services)?);
    let (worker, config) = registry.worker("main")?;
    if !split {
        return Ok(Dispatcher::new(worker, config));
    }
    let (native, native_config) = registry.worker("tokamak")?;
    Ok(Dispatcher::with_native(
        worker,
        config,
        Dispatcher::new(native, native_config),
    ))
}

fn validate_bindings(packages: &BTreeMap<&str, Package>) -> Result<()> {
    for (name, package) in packages {
        let env = &package.environment;
        let app = &package.worker.app;
        let assets = app
            .serves_assets()
            .then(|| crate::packaging::read_asset_manifest(app))
            .transpose()?;
        for (binding, target) in &env.services {
            let collision = env.vars.contains_key(binding)
                || env.storage.iter().any(|store| store.name() == binding)
                || assets
                    .as_ref()
                    .is_some_and(|assets| &assets.binding == binding)
                || binding == super::RUNTIME_MARKER;
            if collision {
                return Err(Error::Startup(format!(
                    "Service binding {binding} conflicts with another binding in {name}"
                ))
                .into());
            }
            if target.entrypoint == EVENT_ENTRYPOINT {
                return Err(Error::Startup(format!(
                    "Service binding {binding} targets {EVENT_ENTRYPOINT}, which only native events reach"
                ))
                .into());
            }
            if !packages.contains_key(target.service.as_str()) || target.entrypoint.is_empty() {
                return Err(Error::Startup(format!(
                    "Service binding {binding} has an invalid target {}:{}",
                    target.service, target.entrypoint
                ))
                .into());
            }
        }
    }
    Ok(())
}
