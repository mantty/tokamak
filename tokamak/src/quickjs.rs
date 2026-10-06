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
use crate::packaging::{ModuleType, PackageLayout, WorkerManifest};
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
    /// A runtime call into the Worker failed.
    #[cfg(feature = "native")]
    #[error("{0}")]
    Call(String),
}

#[cfg(feature = "native")]
impl Error {
    /// A startup failure described by `message`.
    pub(crate) fn startup(message: &str) -> Self {
        Self::Startup(message.to_owned())
    }
}

/// A packaged Worker whose modules are loaded independently by `QuickJS`.
#[cfg(feature = "native")]
#[derive(Clone, Debug)]
pub struct WorkerBundle {
    pub(crate) entry: String,
    /// The type of each module, by name.
    pub(crate) module_types: Arc<BTreeMap<String, ModuleType>>,
    /// The packaged app holding the Worker.
    pub(crate) app: PackageLayout,
    /// The read-only `/bundle`, which holds the Text and Data modules.
    pub(crate) vfs_bundle: VfsBundle,
}

#[cfg(feature = "native")]
impl WorkerBundle {
    /// Describe the Worker `manifest` lists, packaged in `app`.
    #[must_use]
    pub fn new(manifest: WorkerManifest, app: PackageLayout) -> Self {
        Self {
            entry: manifest.entry,
            module_types: Arc::new(manifest.modules),
            vfs_bundle: VfsBundle::new(app.bundle()),
            app,
        }
    }
}

#[cfg(all(test, feature = "native"))]
impl WorkerBundle {
    /// The Worker of the ES module `source`, packaged in `directory`.
    pub(crate) fn of_source(
        source: impl AsRef<[u8]>,
        directory: &std::path::Path,
    ) -> std::result::Result<Self, crate::packaging::Error> {
        let manifest = WorkerManifest::es_modules("worker.mjs", &[]);
        let app = PackageLayout::new(directory.join("app"));
        std::fs::write(directory.join("worker.mjs"), source)?;
        crate::packaging::write_worker(&app, directory, &manifest)?;
        Ok(Self::new(manifest, app))
    }
}

/// Configuration passed to packaged `QuickJS` requests.
#[cfg(feature = "native")]
#[derive(Clone, Debug)]
pub(crate) struct RuntimeConfig {
    /// The app's static assets, when it has any.
    pub(crate) assets: Option<Arc<crate::assets::Assets>>,
    /// Directory containing the app-private Worker cache.
    pub(crate) cache: PathBuf,
    /// Text and JSON Worker environment bindings.
    pub(crate) environment: BTreeMap<String, Value>,
    /// The stores behind storage bindings, when the app has any.
    pub(crate) storage: Option<Arc<dyn crate::linked::StorageRuntime>>,
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
    crate::compiler::compile_module(name, source, crate::compiler::SourceText::Embedded)
        .map_err(Error::Engine)
}

#[cfg(test)]
mod tests {
    use super::compile_module;
    use rquickjs::{ArrayBuffer, Context, Module, Runtime};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct OwnedBytes {
        bytes: Vec<u8>,
        drops: Arc<AtomicUsize>,
    }

    // The vector owns its allocation until the ArrayBuffer releases this source.
    unsafe impl rquickjs::ArrayBufferSource for OwnedBytes {
        fn as_ptr(&self) -> *mut u8 {
            self.bytes.as_ptr().cast_mut()
        }
        fn len(&self) -> usize {
            self.bytes.len()
        }
    }

    impl Drop for OwnedBytes {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn transferred_external_buffers_release_their_owner_once()
    -> Result<(), Box<dyn std::error::Error>> {
        for method in ["transfer", "transferToFixedLength"] {
            for length in [0, 2, 4, 8] {
                let drops = Arc::new(AtomicUsize::new(0));
                let runtime = Runtime::new()?;
                let context = Context::full(&runtime)?;
                context.with(|ctx| -> rquickjs::Result<()> {
                    let source = OwnedBytes { bytes: vec![1, 2, 3, 4], drops: Arc::clone(&drops) };
                    ctx.globals().set("original", ArrayBuffer::from_source(ctx.clone(), source)?)?;
                    let data: Vec<u8> = ctx.eval(format!("globalThis.moved = original.{method}({length}); Array.from(new Uint8Array(moved))"))?;
                    let expected: Vec<_> = (0..length).map(|index| if index < 4 { index + 1 } else { 0 }).collect();
                    assert_eq!(data, expected);
                    assert_eq!(ctx.eval::<usize, _>("original.byteLength")?, 0);
                    ctx.globals().remove("original")?;
                    assert_eq!(drops.load(Ordering::Relaxed), usize::from(length != 4));
                    ctx.globals().remove("moved")?;
                    assert_eq!(drops.load(Ordering::Relaxed), 1);
                    Ok(())
                })?;
                runtime.run_gc();
                assert_eq!(drops.load(Ordering::Relaxed), 1);
            }
        }
        Ok(())
    }

    #[test]
    fn rust_vector_buffers_support_transfer_and_resize() -> Result<(), Box<dyn std::error::Error>> {
        let runtime = Runtime::new()?;
        let context = Context::full(&runtime)?;
        for method in ["transfer", "transferToFixedLength"] {
            for length in [0_u8, 2, 4, 8] {
                context.with(|ctx| -> rquickjs::Result<()> {
                    let mut bytes = Vec::with_capacity(31);
                    bytes.extend_from_slice(&[1_u8, 2, 3, 4]);
                    ctx.globals()
                        .set("bytes", rquickjs::TypedArray::new(ctx.clone(), bytes)?)?;
                    let transferred: Vec<u8> = ctx.eval(format!(
                        "Array.from(new Uint8Array(bytes.buffer.{method}({length})))"
                    ))?;
                    let expected: Vec<_> = (0..length)
                        .map(|index| if index < 4 { index + 1 } else { 0 })
                        .collect();
                    assert_eq!(transferred, expected);
                    assert_eq!(ctx.eval::<usize, _>("bytes.byteLength")?, 0);
                    ctx.globals().remove("bytes")?;
                    Ok(())
                })?;
                runtime.run_gc();
            }
        }
        Ok(())
    }

    #[test]
    fn compiles_and_loads_a_module_bytecode_blob() -> Result<(), Box<dyn std::error::Error>> {
        let bytecode = compile_module("value.mjs", b"export const value = 42;")?;
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
