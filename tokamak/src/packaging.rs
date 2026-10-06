//! Packaged directory layout and the formats of the files in it.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::compiler::{SourceText, compile_module};

/// Failures reading or writing a packaged app.
#[derive(Debug, Error)]
pub enum Error {
    /// Operating-system IO failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// JSON encoding or decoding failed.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// An ES module did not compile.
    #[error("compile Worker module {name}: {message}")]
    Compile {
        /// The module's name.
        name: String,
        /// Why it did not compile.
        message: String,
    },
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

    /// The manifest describing the split `QuickJS` Worker modules.
    #[must_use]
    pub fn worker_manifest(&self) -> PathBuf {
        self.root.join("worker-manifest.json")
    }

    /// The directory containing split `QuickJS` Worker modules.
    #[must_use]
    pub fn worker_modules(&self) -> PathBuf {
        self.root.join("worker-modules")
    }

    /// The normalized Worker environment bindings.
    #[must_use]
    pub fn worker_environment(&self) -> PathBuf {
        self.root.join("worker-environment.json")
    }

    /// The static asset routing manifest.
    #[must_use]
    pub fn asset_manifest(&self) -> PathBuf {
        self.root.join("asset-manifest.json")
    }

    /// The static asset directory.
    #[must_use]
    pub fn assets(&self) -> PathBuf {
        self.root.join("assets")
    }

    /// The read-only Worker `/bundle` directory.
    #[must_use]
    pub fn bundle(&self) -> PathBuf {
        self.root.join("bundle")
    }

    /// The migrations packaged for the D1 binding named `binding`.
    #[must_use]
    pub fn d1_migrations(&self, binding: &str) -> PathBuf {
        self.root.join("d1-migrations").join(binding)
    }

    /// Whether the packaged app serves static assets.
    #[must_use]
    pub fn serves_assets(&self) -> bool {
        self.asset_manifest().is_file()
    }

    /// The bytecode of the ES module `name`.
    pub(crate) fn worker_module(&self, name: &str) -> PathBuf {
        self.worker_modules().join(format!("{name}.qjs"))
    }
}

/// A packaged Worker module's type.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ModuleType {
    /// JavaScript ES module, packaged as bytecode.
    #[serde(rename = "ESModule")]
    EsModule,
    /// Text, imported as a string.
    Text,
    /// Binary data, imported as an `ArrayBuffer`.
    Data,
}

/// A packaged Worker's modules: ES modules as bytecode in
/// [`PackageLayout::worker_modules`], Text and Data modules as files in
/// [`PackageLayout::bundle`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkerManifest {
    /// Module name used to start the Worker.
    pub entry: String,
    /// Every module, by name, with its type.
    pub modules: BTreeMap<String, ModuleType>,
}

#[cfg(all(test, feature = "native"))]
impl WorkerManifest {
    /// A Worker of the ES modules `entry` and `others`.
    pub(crate) fn es_modules(entry: &str, others: &[&str]) -> Self {
        Self {
            entry: entry.to_owned(),
            modules: std::iter::once(&entry)
                .chain(others)
                .map(|name| ((*name).to_owned(), ModuleType::EsModule))
                .collect(),
        }
    }
}

/// A packaged app's static assets, in [`PackageLayout::assets`], and how
/// requests resolve to them.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetManifest {
    /// The name of the Worker's binding to the assets.
    pub binding: String,
    /// The content type of each asset, by its path in the assets directory.
    pub files: BTreeMap<String, String>,
    /// The paths HTML assets are served at.
    pub html_handling: HtmlHandling,
    /// What serves a path no asset serves.
    pub not_found_handling: NotFoundHandling,
}

/// Cloudflare's `assets.html_handling`: the paths HTML assets are served at.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HtmlHandling {
    /// `page/index.html` at `/page/` and `page.html` at `/page`.
    #[default]
    AutoTrailingSlash,
    /// Both at `/page/`.
    ForceTrailingSlash,
    /// Both at `/page`.
    DropTrailingSlash,
    /// Each asset at its own path only.
    None,
}

/// Cloudflare's `assets.not_found_handling`: what serves a path no asset
/// serves.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NotFoundHandling {
    /// Nothing.
    #[default]
    None,
    /// `/index.html`, with status 200.
    SinglePageApplication,
    /// The nearest `404.html` in the path's directory or above it, with
    /// status 404.
    #[serde(rename = "404-page")]
    Page404,
}

/// Package the Worker `manifest` describes, from its module files in `root`:
/// ES modules compiled to bytecode, Text and Data modules copied into its
/// `/bundle`.
///
/// # Errors
///
/// Returns an error when a module cannot be read, compiled or written.
pub fn write_worker(layout: &PackageLayout, root: &Path, manifest: &WorkerManifest) -> Result<()> {
    std::fs::create_dir_all(layout.bundle())?;
    for (name, module_type) in &manifest.modules {
        let source = std::fs::read(root.join(name))?;
        match module_type {
            ModuleType::EsModule => {
                write_file(
                    &layout.worker_module(name),
                    &compile_worker_module(name, &source)?,
                )?;
            }
            ModuleType::Text | ModuleType::Data => {
                write_file(&layout.bundle().join(name), &source)?;
            }
        }
    }
    write_file(
        &layout.worker_manifest(),
        &serde_json::to_vec_pretty(manifest)?,
    )
}

/// Read the packaged Worker's manifest.
///
/// # Errors
///
/// Returns an error when the manifest cannot be read or decoded.
pub fn read_worker_manifest(layout: &PackageLayout) -> Result<WorkerManifest> {
    read_json(&layout.worker_manifest())
}

/// The bytecode of the packaged ES module `name`.
///
/// # Errors
///
/// Returns an error when the module cannot be read or decoded.
pub fn read_worker_module(layout: &PackageLayout, name: &str) -> Result<Vec<u8>> {
    let module = std::fs::read(layout.worker_module(name))?;
    if !module.starts_with(&[0x1f, 0x8b]) {
        return Ok(module);
    }
    let mut bytecode = Vec::new();
    GzDecoder::new(module.as_slice()).read_to_end(&mut bytecode)?;
    Ok(bytecode)
}

/// Write the packaged app's asset manifest.
///
/// # Errors
///
/// Returns an error when the manifest cannot be encoded or written.
pub fn write_asset_manifest(layout: &PackageLayout, manifest: &AssetManifest) -> Result<()> {
    write_file(
        &layout.asset_manifest(),
        &serde_json::to_vec_pretty(manifest)?,
    )
}

/// Read the packaged app's asset manifest.
///
/// # Errors
///
/// Returns an error when the manifest cannot be read or decoded.
pub fn read_asset_manifest(layout: &PackageLayout) -> Result<AssetManifest> {
    read_json(&layout.asset_manifest())
}

/// `name`'s bytecode, gzip-compressed unless that makes it larger, so each
/// module decodes independently and small modules stay cheap.
fn compile_worker_module(name: &str, source: &[u8]) -> Result<Vec<u8>> {
    let bytecode =
        compile_module(name, source, SourceText::Embedded).map_err(|message| Error::Compile {
            name: name.to_owned(),
            message,
        })?;
    let mut compressor = GzEncoder::new(Vec::new(), Compression::best());
    compressor.write_all(&bytecode)?;
    let compressed = compressor.finish()?;
    Ok(if compressed.len() < bytecode.len() {
        compressed
    } else {
        bytecode
    })
}

fn write_file(path: &Path, contents: &[u8]) -> Result<()> {
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)?;
    }
    std::fs::write(path, contents)?;
    Ok(())
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;

    use super::{
        AssetManifest, HtmlHandling, ModuleType, NotFoundHandling, PackageLayout, WorkerManifest,
        read_asset_manifest, read_worker_manifest, read_worker_module, write_asset_manifest,
        write_worker,
    };
    use crate::compiler::{SourceText, compile_module};

    #[test]
    fn resolves_every_path_under_the_app_root() {
        let layout = PackageLayout::new("/apps/example");
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
    fn packages_es_modules_as_bytecode_and_copies_text_and_data_modules()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().join("worker");
        let large = format!("export default {:?};", "quickjs bytecode".repeat(128));
        fs::create_dir_all(root.join("assets"))?;
        fs::write(root.join("entry.js"), "export default 1;")?;
        fs::write(root.join("assets/large.js"), &large)?;
        fs::write(root.join("assets/page.html"), "<p>page</p>")?;
        let manifest = WorkerManifest {
            entry: "entry.js".to_owned(),
            modules: BTreeMap::from([
                ("entry.js".to_owned(), ModuleType::EsModule),
                ("assets/large.js".to_owned(), ModuleType::EsModule),
                ("assets/page.html".to_owned(), ModuleType::Text),
            ]),
        };
        let layout = PackageLayout::new(directory.path().join("app"));

        write_worker(&layout, &root, &manifest)?;

        assert_eq!(read_worker_manifest(&layout)?, manifest);
        for (name, source) in [
            ("entry.js", "export default 1;"),
            ("assets/large.js", &large),
        ] {
            let bytecode = compile_module(name, source.as_bytes(), SourceText::Embedded)?;
            assert_eq!(read_worker_module(&layout, name)?, bytecode);
        }
        assert!(
            fs::metadata(layout.worker_modules().join("assets/large.js.qjs"))?.len()
                < fs::metadata(root.join("assets/large.js"))?.len()
        );
        assert_eq!(
            fs::read_to_string(layout.bundle().join("assets/page.html"))?,
            "<p>page</p>"
        );
        Ok(())
    }

    #[test]
    fn reports_the_module_that_does_not_compile() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        fs::write(directory.path().join("entry.js"), "export default {")?;
        let manifest = WorkerManifest {
            entry: "entry.js".to_owned(),
            modules: BTreeMap::from([("entry.js".to_owned(), ModuleType::EsModule)]),
        };

        let error = write_worker(
            &PackageLayout::new(directory.path().join("app")),
            directory.path(),
            &manifest,
        )
        .err()
        .ok_or("the module compiled")?;

        assert!(
            error
                .to_string()
                .starts_with("compile Worker module entry.js: "),
            "{error}"
        );
        Ok(())
    }

    #[test]
    fn writes_the_asset_manifest_in_wrangler_terms() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let layout = PackageLayout::new(directory.path());
        let manifest = AssetManifest {
            binding: "STATIC".to_owned(),
            files: BTreeMap::from([("index.html".to_owned(), "text/html".to_owned())]),
            html_handling: HtmlHandling::DropTrailingSlash,
            not_found_handling: NotFoundHandling::Page404,
        };

        write_asset_manifest(&layout, &manifest)?;

        let json: serde_json::Value = serde_json::from_slice(&fs::read(layout.asset_manifest())?)?;
        assert_eq!(json["htmlHandling"], "drop-trailing-slash");
        assert_eq!(json["notFoundHandling"], "404-page");
        assert_eq!(read_asset_manifest(&layout)?, manifest);
        assert!(layout.serves_assets());
        Ok(())
    }
}
