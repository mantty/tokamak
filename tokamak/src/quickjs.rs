#![deny(missing_docs)]

//! `QuickJS` runtime integration for tokamak.

#[cfg(feature = "native")]
use std::collections::BTreeMap;
#[cfg(feature = "native")]
use std::path::PathBuf;
#[cfg(feature = "native")]
use std::sync::Arc;

#[cfg(feature = "native")]
use crate::fs::Bundle as VfsBundle;
#[cfg(feature = "native")]
use serde::{Deserialize, Serialize};
#[cfg(feature = "native")]
use serde_json::Value;
use thiserror::Error;

/// Runtime result type.
pub type Result<T> = std::result::Result<T, Error>;

/// `QuickJS` runtime failures.
#[derive(Debug, Error)]
pub enum Error {
    /// Runtime configuration could not be serialized or decoded.
    #[cfg(feature = "native")]
    #[error(transparent)]
    Configuration(#[from] serde_json::Error),
    /// The JavaScript engine rejected an operation.
    #[error("QuickJS operation failed: {0}")]
    Engine(String),
    /// Native IO failed.
    #[cfg(feature = "native")]
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The TLS gateway rejected an operation.
    #[cfg(feature = "native")]
    #[error("TLS operation failed: {0}")]
    Tls(String),
    /// Runtime startup failed.
    #[cfg(feature = "native")]
    #[error("QuickJS startup failed: {0}")]
    Startup(String),
}

/// Static asset service paths.
#[cfg(feature = "native")]
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Assets {
    /// Asset routing manifest.
    pub manifest: PathBuf,
    /// Root directory containing static assets.
    pub root: PathBuf,
}

/// A packaged Worker whose modules are loaded independently by `QuickJS`.
#[cfg(feature = "native")]
#[derive(Clone, Debug)]
pub struct WorkerBundle {
    pub(crate) entry: String,
    pub(crate) modules: PathBuf,
    pub(crate) vfs_bundle: VfsBundle,
    pub(crate) legacy: Option<Arc<Vec<u8>>>,
}

#[cfg(feature = "native")]
impl WorkerBundle {
    /// Describe a split Worker module directory and its read-only `/bundle`.
    #[must_use]
    pub fn from_modules(
        entry: impl Into<String>,
        modules: impl Into<PathBuf>,
        bundle: impl Into<PathBuf>,
    ) -> Self {
        let bundle = bundle.into();
        Self {
            entry: entry.into(),
            modules: modules.into(),
            vfs_bundle: VfsBundle::new(bundle),
            legacy: None,
        }
    }

    /// Describe a legacy single-bytecode Worker and its read-only `/bundle`.
    #[must_use]
    pub fn from_bytecode(bytecode: Vec<u8>, bundle: impl Into<PathBuf>) -> Self {
        let bundle = bundle.into();
        Self {
            entry: "tokamak-worker.mjs".to_owned(),
            modules: PathBuf::new(),
            vfs_bundle: VfsBundle::new(bundle),
            legacy: Some(Arc::new(bytecode)),
        }
    }
}

/// Configuration passed to packaged `QuickJS` requests.
#[cfg(feature = "native")]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeConfig {
    /// Optional static asset service.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assets: Option<Assets>,
    /// Directory containing the app-private Worker cache.
    pub cache: PathBuf,
    /// Text and JSON Worker environment bindings.
    pub environment: BTreeMap<String, Value>,
}

/// Compile a bundled Worker module to `QuickJS` bytecode.
///
/// The input must be a self-contained ES module without unresolved imports.
///
/// # Errors
///
/// Returns an error when `QuickJS` cannot compile or serialize the module.
pub fn compile_worker(source: &[u8]) -> Result<Vec<u8>> {
    compile_module("tokamak-worker.mjs", source)
}

/// Compile one named Worker module to `QuickJS` bytecode.
///
/// The name is retained in the bytecode and is used to resolve its relative
/// imports when the module is loaded.
///
/// # Errors
///
/// Returns an error when `QuickJS` cannot compile or serialize the module.
pub fn compile_module(name: &str, source: &[u8]) -> Result<Vec<u8>> {
    crate::compiler::compile_module(name, source).map_err(Error::Engine)
}

#[cfg(test)]
mod tests {
    use super::compile_worker;
    use rquickjs::{Context, Module, Runtime};

    #[test]
    fn compiles_and_loads_a_module_bytecode_blob() -> Result<(), Box<dyn std::error::Error>> {
        let bytecode = compile_worker(b"export const value = 42;")?;
        let runtime = Runtime::new()?;
        let context = Context::full(&runtime)?;

        context.with(|ctx| -> Result<(), rquickjs::Error> {
            let module = unsafe { Module::load(ctx.clone(), &bytecode) }?;
            let (module, evaluation) = module.eval()?;
            evaluation.finish::<()>()?;
            let value: i32 = module.get("value")?;
            assert_eq!(value, 42);
            Ok(())
        })?;
        Ok(())
    }
}
