//! Packaged main and Tokamak workers and their service bindings.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use tokamak::{PackageLayout, ServiceBinding};

use crate::wrangler_config::{self, WranglerBinding, WranglerConfig, WranglerService};
use crate::{project, worker};

/// Generated configurations for the application's workers.
pub(crate) struct WorkerGraph {
    pub(crate) main: WranglerConfig,
    tokamak: Option<WranglerConfig>,
}

/// A compiled worker and its invocation environment.
pub(crate) struct CompiledWorker<'a> {
    pub(crate) config: &'a WranglerConfig,
    name: &'static str,
    compiled: PathBuf,
    services: BTreeMap<String, ServiceBinding>,
}

impl WorkerGraph {
    /// Load the worker graph recorded by the project's build.
    pub(crate) fn load(project: &Path) -> Result<Self> {
        let deploy = wrangler_config::deploy_config(project)
            .context("find the Wrangler configurations the build generated")?;
        let main = wrangler_config::load_config(&deploy.main)?;
        let tokamak = match deploy.auxiliaries.as_slice() {
            [] => None,
            [path] => Some(wrangler_config::load_config(path)?),
            _ => bail!("the packaged app supports the main and Tokamak workers only"),
        };
        let mut graph = Self { main, tokamak };
        graph.validate_roles()?;
        let names: BTreeSet<_> = graph
            .workers()
            .map(|(_, config)| config.service_name.clone())
            .collect();
        for config in std::iter::once(&mut graph.main).chain(graph.tokamak.iter_mut()) {
            project::check_output(config)?;
            validate_bindings(config)?;
            mark_unavailable_services(config, &names)?;
        }
        Ok(graph)
    }

    /// Configurations keyed by their packaged service names.
    pub(crate) fn workers(&self) -> impl Iterator<Item = (&'static str, &WranglerConfig)> {
        std::iter::once(("main", &self.main))
            .chain(self.tokamak.iter().map(|config| ("tokamak", config)))
    }

    /// Compile each bundle independently of its late-bound environment.
    pub(crate) fn compile(&self, cache: &Path) -> Result<Vec<CompiledWorker<'_>>> {
        let names: BTreeMap<_, _> = self
            .workers()
            .map(|(name, config)| (config.service_name.as_str(), name))
            .collect();
        self.workers()
            .map(|(name, config)| {
                Ok(CompiledWorker {
                    config,
                    name,
                    compiled: worker::compile(&cache.join(name), config)
                        .with_context(|| format!("compile the {name} worker"))?,
                    services: service_bindings(config, &names),
                })
            })
            .collect()
    }

    fn validate_roles(&self) -> Result<()> {
        let binding = self
            .main
            .services
            .iter()
            .find(|binding| binding.binding == "TOKAMAK");
        match (&self.tokamak, binding) {
            (None, None) => Ok(()),
            (Some(native), Some(binding))
                if native.service_name != self.main.service_name
                    && binding.service == native.service_name
                    && binding.entrypoint() == "default" =>
            {
                let main = native
                    .services
                    .iter()
                    .find(|binding| binding.binding == "MAIN");
                if main.is_none_or(|binding| {
                    binding.service != self.main.service_name || binding.entrypoint() != "default"
                }) {
                    bail!(
                        "the Tokamak worker must bind MAIN to the main worker's default entrypoint"
                    );
                }
                Ok(())
            }
            _ => bail!(
                "TOKAMAK must bind the main worker to a distinct packaged auxiliary worker's default entrypoint"
            ),
        }
    }
}

/// The service bindings of `config` whose targets `names` maps to packaged
/// service names, keyed by binding name.
fn service_bindings(
    config: &WranglerConfig,
    names: &BTreeMap<&str, &'static str>,
) -> BTreeMap<String, ServiceBinding> {
    config
        .services
        .iter()
        .filter_map(|binding| {
            let target = names.get(binding.service.as_str())?;
            Some((
                binding.binding.clone(),
                ServiceBinding {
                    service: (*target).to_owned(),
                    entrypoint: binding.entrypoint().to_owned(),
                },
            ))
        })
        .collect()
}

impl CompiledWorker<'_> {
    /// Write the bundle and environment into its application subpackage.
    pub(crate) fn package(&self, app: &Path) -> Result<()> {
        let layout = PackageLayout::new(app);
        let layout = if self.name == "tokamak" {
            layout.tokamak_worker()
        } else {
            layout
        };
        worker::package(
            layout.root(),
            &self.compiled,
            self.config,
            self.services.clone(),
        )
    }
}

fn validate_bindings(config: &WranglerConfig) -> Result<()> {
    let mut names: BTreeSet<&str> = config.vars.keys().map(String::as_str).collect();
    names.insert("TOKAMAK_RUNTIME");
    let bindings = config
        .assets
        .iter()
        .map(|assets| assets.binding.as_str())
        .chain(
            config
                .storage
                .iter()
                .map(wrangler_config::WranglerStorage::name),
        )
        .chain(config.bindings.iter().map(|binding| binding.name.as_str()))
        .chain(
            config
                .services
                .iter()
                .map(|binding| binding.binding.as_str()),
        );
    for name in bindings {
        if name.is_empty() || !names.insert(name) {
            bail!(
                "worker {} has an empty or duplicate binding name: {name}",
                config.service_name
            );
        }
    }
    for binding in &config.services {
        if binding.service.is_empty() || binding.entrypoint().is_empty() {
            bail!(
                "service binding {} must name a worker and entrypoint",
                binding.binding
            );
        }
        if binding.entrypoint() == "TokamakEvents" {
            bail!(
                "service binding {} targets TokamakEvents, which only native events reach",
                binding.binding
            );
        }
    }
    Ok(())
}

fn mark_unavailable_services(config: &mut WranglerConfig, names: &BTreeSet<String>) -> Result<()> {
    for binding in &config.services {
        if names.contains(&binding.service) {
            validate_local_options(binding)?;
        } else {
            config.bindings.push(WranglerBinding {
                name: binding.binding.clone(),
                kind: "services",
                feature: "service bindings",
            });
        }
    }
    Ok(())
}

fn validate_local_options(binding: &WranglerService) -> Result<()> {
    for (name, value) in &binding.options {
        if name != "remote" || value != &serde_json::Value::Bool(false) {
            bail!(
                "packaged service binding {} does not support the {name} option",
                binding.binding
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use anyhow::Result;
    use serde_json::{Value, json};

    use super::WorkerGraph;

    fn project(main: &Value, native: Option<&Value>) -> Result<tempfile::TempDir> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        fs::create_dir_all(root.join(".wrangler/deploy"))?;
        for (name, config) in
            std::iter::once(("main", main)).chain(native.map(|config| ("native", config)))
        {
            fs::write(
                root.join(format!("{name}.json")),
                serde_json::to_vec(config)?,
            )?;
        }
        fs::write(root.join("index.js"), "export default {};")?;
        fs::write(
            root.join(".wrangler/deploy/config.json"),
            serde_json::to_vec(&json!({
                "configPath": "../../main.json",
                "auxiliaryWorkers": native.map(|_| json!({ "configPath": "../../native.json" }))
                    .into_iter().collect::<Vec<_>>()
            }))?,
        )?;
        Ok(directory)
    }

    #[test]
    fn unavailable_external_bindings_remain_visible_as_build_warnings() -> Result<()> {
        let project = project(
            &json!({
                "name": "main", "main": "index.js",
                "services": [{ "binding": "API", "service": "external", "remote": true }]
            }),
            None,
        )?;
        let graph = WorkerGraph::load(project.path())?;
        assert_eq!(
            graph
                .main
                .bindings
                .iter()
                .map(|binding| (binding.name.as_str(), binding.kind))
                .collect::<Vec<_>>(),
            [("API", "services")]
        );
        Ok(())
    }

    #[test]
    fn rejects_ambiguous_or_unrepresentable_service_graphs() -> Result<()> {
        let main = json!({ "name": "main", "main": "index.js",
            "services": [{ "binding": "TOKAMAK", "service": "native" }] });
        let native = json!({ "name": "native", "main": "index.js",
            "services": [{ "binding": "MAIN", "service": "main" }] });
        for (field, value, message) in [
            (
                "services",
                json!([
                    { "binding": "TOKAMAK", "service": "native" },
                    { "binding": "TOKAMAK_RUNTIME", "service": "native" }
                ]),
                "duplicate binding name: TOKAMAK_RUNTIME",
            ),
            (
                "vars",
                json!({ "TOKAMAK": true }),
                "duplicate binding name: TOKAMAK",
            ),
            (
                "services",
                json!([
                    { "binding": "TOKAMAK", "service": "native" },
                    { "binding": "TOKAMAK", "service": "native" }
                ]),
                "duplicate binding name: TOKAMAK",
            ),
            (
                "services",
                json!([{ "binding": "TOKAMAK", "service": "missing" }]),
                "TOKAMAK must bind",
            ),
            (
                "services",
                json!([{ "binding": "TOKAMAK", "service": "native", "remote": true }]),
                "remote option",
            ),
            (
                "services",
                json!([{ "binding": "TOKAMAK", "service": "native", "environment": "prod" }]),
                "environment option",
            ),
            (
                "services",
                json!([{ "binding": "TOKAMAK", "service": "native", "props": {} }]),
                "props option",
            ),
            (
                "services",
                json!([{ "binding": "TOKAMAK", "service": "native", "entrypoint": "Location" }]),
                "TOKAMAK must bind",
            ),
            (
                "services",
                json!([
                    { "binding": "TOKAMAK", "service": "native" },
                    { "binding": "EVENTS", "service": "native", "entrypoint": "TokamakEvents" }
                ]),
                "only native events reach",
            ),
        ] {
            let mut main = main.clone();
            main[field] = value;
            let project = project(&main, Some(&native))?;
            assert_rejected(project.path(), message)?;
        }
        Ok(())
    }

    /// Assert that the graph at `root` is rejected with a message containing `message`.
    fn assert_rejected(root: &std::path::Path, message: &str) -> Result<()> {
        let error = WorkerGraph::load(root)
            .err()
            .ok_or_else(|| anyhow::anyhow!("accepted invalid graph"))?;
        assert!(error.to_string().contains(message), "{error:#}");
        Ok(())
    }

    #[test]
    fn requires_distinct_bundles_and_both_generated_binding_targets() -> Result<()> {
        for (target, native, message) in [
            ("native", None, "TOKAMAK must bind"),
            (
                "main",
                Some(json!({ "name": "main", "main": "index.js" })),
                "TOKAMAK must bind",
            ),
            (
                "native",
                Some(json!({ "name": "native", "main": "index.js" })),
                "must bind MAIN",
            ),
            (
                "native",
                Some(json!({ "name": "native", "main": "index.js",
                "services": [{ "binding": "MAIN", "service": "missing" }] })),
                "must bind MAIN",
            ),
        ] {
            let project = project(
                &json!({ "name": "main", "main": "index.js",
                    "services": [{ "binding": "TOKAMAK", "service": target }]
                }),
                native.as_ref(),
            )?;
            assert_rejected(project.path(), message)?;
        }
        Ok(())
    }
}
