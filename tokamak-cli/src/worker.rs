//! Worker package preparation.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tokamak::{
    ModuleType, PackageLayout, WorkerEnvironment, WorkerManifest, compile_module,
    compress_worker_module, write_worker_environment, write_worker_manifest,
};
use walkdir::WalkDir;

use super::support::{self, copy_dir_contents, copy_file, glob_matches, slash_path};
use super::wrangler_config::{WranglerAssets, WranglerConfig};
use super::{cache, storage};

/// What a compiled Worker was compiled from, and what it is.
#[derive(Deserialize, Serialize)]
struct WorkerCache {
    inputs: String,
    outputs: String,
}

/// The Worker `wrangler` describes, compiled into a package directory under
/// `cache_dir`; an earlier compilation is reused while its inputs are unchanged.
pub(crate) fn compile(cache_dir: &Path, wrangler: &WranglerConfig) -> Result<PathBuf> {
    let (root, manifest) = collect_modules(wrangler)?;
    let files = manifest
        .modules
        .keys()
        .map(|name| root.join(name))
        .collect::<Vec<_>>();
    let inputs = format!(
        "{}:{}:{}",
        env!("CARGO_PKG_VERSION"),
        serde_json::to_string(&manifest)?,
        cache::hash_paths(&files)?
    );
    let marker = cache_dir.join("state.json");
    let compiled = cache_dir.join("compiled");
    let cached = fs::read(&marker)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<WorkerCache>(&bytes).ok());
    if let Some(state) = cached
        && state.inputs == inputs
        && PackageLayout::new(&compiled).worker_manifest().is_file()
        && state.outputs == cache::hash_tree(&compiled, |_| false)?
    {
        return Ok(compiled);
    }
    support::reset_path(&compiled)?;
    compile_modules(&PackageLayout::new(&compiled), root, &manifest)?;
    let outputs = cache::hash_tree(&compiled, |_| false)?;
    fs::write(
        marker,
        serde_json::to_vec(&WorkerCache { inputs, outputs })?,
    )?;
    Ok(compiled)
}

/// Package the app in `app_dir`: the `compiled` Worker with its environment,
/// storage and assets.
pub(crate) fn package(app_dir: &Path, compiled: &Path, wrangler: &WranglerConfig) -> Result<()> {
    let layout = PackageLayout::new(app_dir);
    write_worker_environment(
        &layout,
        &WorkerEnvironment {
            vars: wrangler.vars.clone(),
            storage: storage::package(wrangler, &layout)?,
        },
    )?;
    fs::create_dir_all(layout.bundle())?;
    if let Some(assets) = &wrangler.assets {
        copy_dir_contents(&assets.directory, &layout.assets())?;
        write_asset_manifest(&layout, assets)?;
    }
    copy_dir_contents(compiled, app_dir)
}

/// Write `layout`'s asset manifest: each file in its assets directory with its
/// content type, and `assets`'s routing settings.
fn write_asset_manifest(layout: &PackageLayout, assets: &WranglerAssets) -> Result<()> {
    let root = layout.assets();
    let mut files = BTreeMap::new();
    for file in WalkDir::new(&root) {
        let file = file?;
        if !file.file_type().is_file() {
            continue;
        }
        let content_type = mime_guess::from_path(file.path()).first_or_octet_stream();
        files.insert(
            slash_path(file.path().strip_prefix(&root)?)?,
            content_type.essence_str().to_owned(),
        );
    }
    let manifest = serde_json::json!({
        "binding": assets.binding,
        "files": files,
        "htmlHandling": assets.html_handling.as_str(),
        "notFoundHandling": assets.not_found_handling.as_str(),
    });
    fs::write(
        layout.asset_manifest(),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(())
}

/// The directory of `main` and the Worker's modules in it, as `wrangler deploy`
/// finds them for a configuration with `no_bundle`: `main`, and the files its
/// module rules match.
fn collect_modules(wrangler: &WranglerConfig) -> Result<(&Path, WorkerManifest)> {
    let root = wrangler
        .main
        .parent()
        .context("the Worker's main module has no directory")?;
    let entry = slash_path(wrangler.main.strip_prefix(root)?)?;
    let mut modules = BTreeMap::from([(entry.clone(), ModuleType::EsModule)]);
    for file in WalkDir::new(root) {
        let file = file?;
        if !file.file_type().is_file() || file.path() == wrangler.path {
            continue;
        }
        let name = slash_path(file.path().strip_prefix(root)?)?;
        let Some(module_type) = rule_type(&name, wrangler) else {
            continue;
        };
        if !module_type.is_supported() {
            bail!(
                "Worker module {name} is a {} module; tokamak supports ESModule, Text and Data modules",
                module_type.name()
            );
        }
        modules.entry(name).or_insert(module_type);
    }
    Ok((root, WorkerManifest { entry, modules }))
}

/// The type of the first of `wrangler`'s rules with a glob that matches `name`.
fn rule_type(name: &str, wrangler: &WranglerConfig) -> Option<ModuleType> {
    wrangler
        .rules
        .iter()
        .find(|rule| rule.globs.iter().any(|glob| glob_matches(glob, name)))
        .map(|rule| rule.module_type)
}

/// Compile `manifest`'s ES modules to bytecode and copy its Text and Data
/// modules from `root` into `layout`.
fn compile_modules(layout: &PackageLayout, root: &Path, manifest: &WorkerManifest) -> Result<()> {
    for (name, module_type) in &manifest.modules {
        let source = root.join(name);
        match module_type {
            ModuleType::EsModule => {
                let bytecode = compile_module(name, &fs::read(&source)?)
                    .with_context(|| format!("compile Worker module {name}"))?;
                let destination = layout.worker_modules().join(format!("{name}.qjs"));
                fs::create_dir_all(destination.parent().context("module has no directory")?)?;
                fs::write(destination, compress_worker_module(&bytecode)?)?;
            }
            ModuleType::Text | ModuleType::Data => copy_file(&source, layout.bundle().join(name))?,
            ModuleType::CommonJs | ModuleType::CompiledWasm => {
                bail!("Worker module {name} has an unsupported type");
            }
        }
    }
    write_worker_manifest(layout, manifest)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wrangler_config::{self, HtmlHandling, NotFoundHandling};

    /// The Worker `root/index.js` with the module rules `rules`, as JSON.
    fn worker(root: &Path, rules: &str) -> Result<WranglerConfig> {
        fs::write(
            root.join("wrangler.json"),
            format!(r#"{{"name":"app","main":"index.js","rules":{rules}}}"#),
        )?;
        wrangler_config::load_config(&root.join("wrangler.json"))
    }

    fn write(root: &Path, files: &[&str]) -> Result<()> {
        for name in files {
            let path = root.join(name);
            fs::create_dir_all(path.parent().context("no parent")?)?;
            fs::write(path, "")?;
        }
        Ok(())
    }

    #[test]
    fn collects_modules_as_wrangler_does_without_bundling() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        write(
            root,
            &[
                "index.js",
                "assets/lazy.js",
                "chunks/page.mjs",
                "assets/page.html",
                "assets/data.bin",
                "assets/query.sql",
                "notes.md",
                "index.js.map",
                ".vite/manifest.json",
            ],
        )?;
        let wrangler = worker(
            root,
            r#"[{"type":"ESModule","globs":["**/*.js","**/*.mjs"]}]"#,
        )?;

        let (module_root, manifest) = collect_modules(&wrangler)?;

        assert_eq!(module_root, root);
        assert_eq!(manifest.entry, "index.js");
        assert_eq!(
            manifest.modules,
            BTreeMap::from([
                ("assets/data.bin".to_owned(), ModuleType::Data),
                ("assets/lazy.js".to_owned(), ModuleType::EsModule),
                ("assets/page.html".to_owned(), ModuleType::Text),
                ("assets/query.sql".to_owned(), ModuleType::Text),
                ("chunks/page.mjs".to_owned(), ModuleType::EsModule),
                ("index.js".to_owned(), ModuleType::EsModule),
            ])
        );
        Ok(())
    }

    #[test]
    fn rejects_webassembly_and_commonjs_modules() -> Result<()> {
        for (file, rules, expected) in [
            (
                "assets/add.wasm",
                "[]",
                "assets/add.wasm is a CompiledWasm module",
            ),
            (
                "legacy.cjs",
                r#"[{"type":"CommonJS","globs":["**/*.cjs"]}]"#,
                "legacy.cjs is a CommonJS module",
            ),
        ] {
            let directory = tempfile::tempdir()?;
            write(directory.path(), &["index.js", file])?;
            let error = collect_modules(&worker(directory.path(), rules)?).map(|_| ());
            assert!(
                error.as_ref().is_err_and(|error| error.to_string()
                    == format!(
                        "Worker module {expected}; tokamak supports ESModule, Text and Data modules"
                    )),
                "{error:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn writes_content_types_and_routing_modes() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let layout = PackageLayout::new(directory.path());
        fs::create_dir_all(layout.assets().join("styles"))?;
        fs::write(layout.assets().join("index.html"), "home")?;
        fs::write(layout.assets().join("styles/app.css"), "body{}")?;
        let assets = WranglerAssets {
            directory: layout.assets(),
            binding: "ASSETS".to_owned(),
            html_handling: HtmlHandling::Drop,
            not_found_handling: NotFoundHandling::SinglePageApplication,
        };

        write_asset_manifest(&layout, &assets)?;

        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(layout.asset_manifest())?)?;
        assert_eq!(manifest["binding"], "ASSETS");
        assert_eq!(manifest["files"]["index.html"], "text/html");
        assert_eq!(manifest["files"]["styles/app.css"], "text/css");
        assert_eq!(manifest["htmlHandling"], "drop-trailing-slash");
        assert_eq!(manifest["notFoundHandling"], "single-page-application");
        assert!(layout.serves_assets());
        Ok(())
    }

    #[test]
    fn compiles_es_modules_and_copies_text_and_data_modules() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().join("worker");
        fs::create_dir_all(root.join("assets"))?;
        fs::write(
            root.join("index.js"),
            "import page from './assets/page.html'; export default { fetch() { return new Response(page); } };",
        )?;
        fs::write(root.join("assets/page.html"), "<p>page</p>")?;
        let manifest = WorkerManifest {
            entry: "index.js".to_owned(),
            modules: BTreeMap::from([
                ("index.js".to_owned(), ModuleType::EsModule),
                ("assets/page.html".to_owned(), ModuleType::Text),
            ]),
        };
        let layout = PackageLayout::new(directory.path().join("app"));

        compile_modules(&layout, &root, &manifest)?;

        assert!(layout.worker_modules().join("index.js.qjs").is_file());
        assert_eq!(
            fs::read_to_string(layout.bundle().join("assets/page.html"))?,
            "<p>page</p>"
        );
        assert_eq!(tokamak::read_worker_manifest(&layout)?, manifest);
        Ok(())
    }
}
