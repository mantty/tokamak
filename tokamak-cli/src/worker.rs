//! Worker package preparation.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tokamak::{
    AssetManifest, ModuleType, PackageLayout, WorkerEnvironment, WorkerManifest, write_worker,
    write_worker_environment,
};
use tokamak_cli::copy_dir_contents;
use walkdir::WalkDir;

use super::paths::{self, slash_path};
use super::wrangler_config::{WranglerAssets, WranglerConfig, WranglerModuleType, glob_matches};
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
    let mut files = manifest
        .modules
        .keys()
        .map(|name| root.join(name))
        .collect::<Vec<_>>();
    // The running tok compiles and packages the modules, so it is an input too.
    files.push(std::env::current_exe()?);
    let inputs = format!(
        "{}:{}",
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
        && state.outputs == cache::hash_tree(&compiled)?
    {
        return Ok(compiled);
    }
    paths::reset_path(&compiled)?;
    write_worker(&PackageLayout::new(&compiled), root, &manifest)?;
    let outputs = cache::hash_tree(&compiled)?;
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
    let manifest = AssetManifest {
        binding: assets.binding.clone(),
        files,
        html_handling: assets.html_handling,
        not_found_handling: assets.not_found_handling,
    };
    tokamak::write_asset_manifest(layout, &manifest)?;
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
        if let Some(rule_type) = rule_type(&name, wrangler) {
            let module_type = module_type(&name, rule_type)?;
            modules.entry(name).or_insert(module_type);
        }
    }
    Ok((root, WorkerManifest { entry, modules }))
}

/// The type of the first of `wrangler`'s rules with a glob that matches `name`.
fn rule_type(name: &str, wrangler: &WranglerConfig) -> Option<WranglerModuleType> {
    wrangler
        .rules
        .iter()
        .find(|rule| rule.globs.iter().any(|glob| glob_matches(glob, name)))
        .map(|rule| rule.module_type)
}

/// The type the runtime loads the module `name`, of `rule_type`, as.
fn module_type(name: &str, rule_type: WranglerModuleType) -> Result<ModuleType> {
    match rule_type {
        WranglerModuleType::ESModule => Ok(ModuleType::EsModule),
        WranglerModuleType::Text => Ok(ModuleType::Text),
        WranglerModuleType::Data => Ok(ModuleType::Data),
        WranglerModuleType::CommonJS | WranglerModuleType::CompiledWasm => bail!(
            "Worker module {name} is a {rule_type:?} module; tokamak supports ESModule, Text and Data modules"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wrangler_config;
    use tokamak::{HtmlHandling, NotFoundHandling};

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
            html_handling: HtmlHandling::DropTrailingSlash,
            not_found_handling: NotFoundHandling::SinglePageApplication,
        };

        write_asset_manifest(&layout, &assets)?;

        assert_eq!(
            tokamak::read_asset_manifest(&layout)?,
            AssetManifest {
                binding: "ASSETS".to_owned(),
                files: [("index.html", "text/html"), ("styles/app.css", "text/css")]
                    .map(|(file, content_type)| (file.to_owned(), content_type.to_owned()))
                    .into(),
                html_handling: HtmlHandling::DropTrailingSlash,
                not_found_handling: NotFoundHandling::SinglePageApplication,
            }
        );
        Ok(())
    }
}
