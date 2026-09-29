//! Packaged directory layout and Worker bytecode formats.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const WORKER_BUNDLE: &str = "worker.bundle";
const WORKER_MANIFEST: &str = "worker-manifest.json";
const WORKER_MODULES: &str = "worker-modules";
const WORKER_ENVIRONMENT: &str = "worker-environment.json";
const ASSET_MANIFEST: &str = "asset-manifest.json";
const ASSETS: &str = "assets";
const BUNDLE: &str = "bundle";
const D1_MIGRATIONS: &str = "d1-migrations";
const WORKER_BUNDLE_HEADER: &[u8] = b"TOKAMAK-QJS-GZIP\x01";

/// Failures reading or writing packaged Worker bytecode and manifests.
#[derive(Debug, Error)]
pub enum Error {
    /// Operating-system IO failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// JSON encoding or decoding failed.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Result type for package layout and bytecode operations.
pub type Result<T> = std::result::Result<T, Error>;

/// The packaged contents of a tokamak application.
///
/// `tokamak-cli` writes this layout and `tokamak` reads it. Both ask for paths
/// rather than naming files.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackageLayout {
    root: PathBuf,
}

impl PackageLayout {
    /// Describe the layout rooted at a packaged app directory.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The packaged app directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The `QuickJS` Worker bytecode.
    #[must_use]
    pub fn worker_bundle(&self) -> PathBuf {
        self.root.join(WORKER_BUNDLE)
    }

    /// The manifest describing the split `QuickJS` Worker modules.
    #[must_use]
    pub fn worker_manifest(&self) -> PathBuf {
        self.root.join(WORKER_MANIFEST)
    }

    /// The directory containing split `QuickJS` Worker modules.
    #[must_use]
    pub fn worker_modules(&self) -> PathBuf {
        self.root.join(WORKER_MODULES)
    }

    /// The normalized Worker environment bindings.
    #[must_use]
    pub fn worker_environment(&self) -> PathBuf {
        self.root.join(WORKER_ENVIRONMENT)
    }

    /// The static asset routing manifest.
    #[must_use]
    pub fn asset_manifest(&self) -> PathBuf {
        self.root.join(ASSET_MANIFEST)
    }

    /// The static asset directory.
    #[must_use]
    pub fn assets(&self) -> PathBuf {
        self.root.join(ASSETS)
    }

    /// The read-only Worker `/bundle` directory.
    #[must_use]
    pub fn bundle(&self) -> PathBuf {
        self.root.join(BUNDLE)
    }

    /// The migrations packaged for the D1 binding named `binding`.
    #[must_use]
    pub fn d1_migrations(&self, binding: &str) -> PathBuf {
        self.root.join(D1_MIGRATIONS).join(binding)
    }

    /// Whether the packaged app serves static assets.
    #[must_use]
    pub fn serves_assets(&self) -> bool {
        self.asset_manifest().is_file()
    }
}

/// The entry module and on-disk module directory for a packaged Worker.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkerManifest {
    /// Module name used to start the Worker.
    pub entry: String,
}

/// Compress `QuickJS` bytecode for storage in a packaged app.
///
/// # Errors
///
/// Returns an error when the gzip encoder cannot write the bytecode.
pub fn compress_worker_bundle(bytecode: &[u8]) -> Result<Vec<u8>> {
    compress_worker_bytecode(bytecode, WORKER_BUNDLE_HEADER)
}

/// Decode a packaged `QuickJS` bundle.
///
/// Legacy uncompressed bundles are returned unchanged.
///
/// # Errors
///
/// Returns an error when a compressed bundle is invalid or cannot be decoded.
pub fn decompress_worker_bundle(bundle: &[u8]) -> Result<Vec<u8>> {
    if !bundle.starts_with(WORKER_BUNDLE_HEADER) {
        return Ok(bundle.to_vec());
    }
    decompress_gzip(&bundle[WORKER_BUNDLE_HEADER.len()..])
}

/// Compress one split Worker module for storage in a packaged app.
///
/// The result is either a standard gzip stream or the original bytecode when
/// compression would make the module larger. This keeps small modules cheap
/// while allowing the runtime to decode each module independently.
///
/// # Errors
///
/// Returns an error when the gzip encoder cannot write the bytecode.
pub fn compress_worker_module(bytecode: &[u8]) -> Result<Vec<u8>> {
    compress_worker_bytecode(bytecode, &[])
}

fn compress_worker_bytecode(bytecode: &[u8], prefix: &[u8]) -> Result<Vec<u8>> {
    let mut compressor = GzEncoder::new(prefix.to_vec(), Compression::best());
    compressor.write_all(bytecode)?;
    let compressed = compressor.finish()?;
    Ok(if compressed.len() < bytecode.len() {
        compressed
    } else {
        bytecode.to_vec()
    })
}

/// Decode one independently stored Worker module.
///
/// # Errors
///
/// Returns an error when a gzip-compressed module is invalid.
pub fn decompress_worker_module(module: &[u8]) -> Result<Vec<u8>> {
    if !module.starts_with(&[0x1f, 0x8b]) {
        return Ok(module.to_vec());
    }
    decompress_gzip(module)
}

fn decompress_gzip(compressed: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = GzDecoder::new(compressed);
    let mut bytecode = Vec::new();
    decoder.read_to_end(&mut bytecode)?;
    Ok(bytecode)
}

/// Write the split Worker manifest.
///
/// # Errors
///
/// Returns an error when the manifest cannot be serialized or written.
pub fn write_worker_manifest(layout: &PackageLayout, manifest: &WorkerManifest) -> Result<()> {
    std::fs::write(
        layout.worker_manifest(),
        serde_json::to_vec_pretty(manifest)?,
    )?;
    Ok(())
}

/// Read the split Worker manifest.
///
/// # Errors
///
/// Returns an error when the manifest cannot be read or decoded.
pub fn read_worker_manifest(layout: &PackageLayout) -> Result<WorkerManifest> {
    Ok(serde_json::from_slice(&std::fs::read(
        layout.worker_manifest(),
    )?)?)
}

#[cfg(test)]
mod tests {
    use super::{
        PackageLayout, WORKER_BUNDLE_HEADER, WorkerManifest, compress_worker_bundle,
        compress_worker_module, decompress_worker_bundle, decompress_worker_module,
    };

    #[test]
    fn resolves_every_path_under_the_app_root() {
        let layout = PackageLayout::new("/apps/example");
        assert_eq!(
            layout.worker_bundle(),
            std::path::Path::new("/apps/example/worker.bundle")
        );
        assert_eq!(
            layout.worker_manifest(),
            std::path::Path::new("/apps/example/worker-manifest.json")
        );
        assert_eq!(
            layout.worker_modules(),
            std::path::Path::new("/apps/example/worker-modules")
        );
        assert_eq!(
            layout.worker_environment(),
            std::path::Path::new("/apps/example/worker-environment.json")
        );
        assert_eq!(
            layout.asset_manifest(),
            std::path::Path::new("/apps/example/asset-manifest.json")
        );
        assert_eq!(
            layout.assets(),
            std::path::Path::new("/apps/example/assets")
        );
        assert_eq!(
            layout.bundle(),
            std::path::Path::new("/apps/example/bundle")
        );
        assert_eq!(
            layout.d1_migrations("DB"),
            std::path::Path::new("/apps/example/d1-migrations/DB")
        );
    }

    #[test]
    fn reports_no_assets_without_a_manifest() {
        assert!(!PackageLayout::new("/apps/missing").serves_assets());
    }

    #[test]
    fn round_trips_compressed_worker_bytecode() -> Result<(), Box<dyn std::error::Error>> {
        let bytecode = b"quickjs bytecode".repeat(128);
        let compressed = compress_worker_bundle(&bytecode)?;

        assert!(compressed.starts_with(WORKER_BUNDLE_HEADER));
        assert!(compressed.len() < bytecode.len());
        assert_eq!(decompress_worker_bundle(&compressed)?, bytecode);
        Ok(())
    }

    #[test]
    fn rejects_corrupt_compressed_worker_bytecode() -> Result<(), Box<dyn std::error::Error>> {
        let mut compressed = compress_worker_bundle(&b"quickjs bytecode".repeat(128))?;
        let last = compressed
            .last_mut()
            .ok_or("compressed worker bundle was empty")?;
        *last ^= 1;

        assert!(decompress_worker_bundle(&compressed).is_err());
        Ok(())
    }

    #[test]
    fn accepts_legacy_uncompressed_worker_bytecode() -> Result<(), Box<dyn std::error::Error>> {
        let bytecode = b"legacy bytecode";

        assert_eq!(decompress_worker_bundle(bytecode)?, bytecode);
        Ok(())
    }

    #[test]
    fn compresses_split_modules_independently() -> Result<(), Box<dyn std::error::Error>> {
        let bytecode = b"quickjs bytecode".repeat(128);
        let compressed = compress_worker_module(&bytecode)?;
        assert_eq!(decompress_worker_module(&compressed)?, bytecode);
        Ok(())
    }

    #[test]
    fn round_trips_worker_manifest() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let layout = PackageLayout::new(directory.path());
        let manifest = WorkerManifest {
            entry: "entry.js".to_owned(),
        };
        super::write_worker_manifest(&layout, &manifest)?;

        assert_eq!(super::read_worker_manifest(&layout)?, manifest);
        Ok(())
    }
}
