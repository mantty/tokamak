//! Content fingerprints for reusable build outputs.

use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokamak::WranglerConfig;
use walkdir::WalkDir;

use super::support;

pub(crate) type ProjectFiles = std::collections::BTreeMap<PathBuf, (u64, SystemTime)>;

#[derive(Deserialize, Serialize)]
struct ProjectBuild {
    version: String,
    project: PathBuf,
    wrangler: PathBuf,
    vars_source: PathBuf,
    fingerprint: String,
    outputs: Vec<PathBuf>,
    required: Vec<PathBuf>,
}

pub(crate) fn project_is_current(project: &Path, build_dir: &Path) -> Result<bool> {
    let marker = build_dir.join(".tokamak/project-build.json");
    let Ok(state) = fs::read(&marker) else {
        return Ok(false);
    };
    let Ok(state) = serde_json::from_slice::<ProjectBuild>(&state) else {
        return Ok(false);
    };
    if state.version != env!("CARGO_PKG_VERSION") || state.project != project {
        return Ok(false);
    }
    let snapshot = build_dir.join(".tokamak/project-output");
    for output in &state.outputs {
        let destination = project.join(output);
        if !destination.exists() {
            let source = snapshot.join(output);
            if !source.is_file() {
                return Ok(false);
            }
            support::copy_file(source, destination)?;
        }
    }
    if !state.wrangler.is_file() {
        return Ok(false);
    }
    if state.required.iter().any(|path| !path.exists()) {
        return Ok(false);
    }
    Ok(state.fingerprint
        == project_fingerprint(project, build_dir, &state.wrangler, &state.vars_source)?)
}

pub(crate) fn record_project_build(
    project: &Path,
    build_dir: &Path,
    wrangler: &WranglerConfig,
    before: &ProjectFiles,
) -> Result<()> {
    let wrangler_path = fs::canonicalize(&wrangler.path)?;
    let vars_source = fs::canonicalize(&wrangler.vars_source)?;
    let marker = build_dir.join(".tokamak/project-build.json");
    let previous = fs::read(&marker)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<ProjectBuild>(&bytes).ok());
    let after = project_files(project, build_dir)?;
    let mut outputs = after
        .iter()
        .filter(|(path, metadata)| before.get(*path) != Some(metadata))
        .map(|(path, _)| path.clone())
        .collect::<Vec<_>>();
    if let Some(previous) = previous {
        outputs.extend(
            previous
                .outputs
                .into_iter()
                .filter(|path| after.contains_key(path)),
        );
    }
    outputs.sort();
    outputs.dedup();
    let snapshot = build_dir.join(".tokamak/project-output");
    support::reset_path(&snapshot)?;
    for output in &outputs {
        support::copy_file(project.join(output), snapshot.join(output))?;
    }
    let state = ProjectBuild {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        project: project.to_path_buf(),
        wrangler: wrangler_path.clone(),
        vars_source: vars_source.clone(),
        fingerprint: project_fingerprint(project, build_dir, &wrangler_path, &vars_source)?,
        outputs,
        required: required_outputs(wrangler)?,
    };
    fs::create_dir_all(marker.parent().context("project cache has no parent")?)?;
    fs::write(marker, serde_json::to_vec(&state)?)?;
    Ok(())
}

pub(crate) fn project_files(project: &Path, build_dir: &Path) -> Result<ProjectFiles> {
    let mut files = ProjectFiles::new();
    for entry in WalkDir::new(project)
        .follow_links(true)
        .into_iter()
        .filter_entry(|entry| !excluded_project_path(project, build_dir, entry.path()))
    {
        let entry = entry?;
        if entry.file_type().is_file() {
            let metadata = entry.metadata()?;
            files.insert(
                entry.path().strip_prefix(project)?.to_path_buf(),
                (metadata.len(), metadata.modified()?),
            );
        }
    }
    Ok(files)
}

fn project_fingerprint(
    project: &Path,
    build_dir: &Path,
    wrangler: &Path,
    vars_source: &Path,
) -> Result<String> {
    hash_tree(project, |path| {
        path == wrangler
            || path == vars_source
            || excluded_project_path(project, build_dir, path)
            || ["tokamak.json", "tokamak.jsonc"]
                .iter()
                .any(|name| path == project.join(name))
    })
}

fn required_outputs(wrangler: &WranglerConfig) -> Result<Vec<PathBuf>> {
    let mut required = vec![wrangler.main.clone()];
    if let Some(assets) = &wrangler.assets {
        required.push(assets.directory.clone());
        for entry in WalkDir::new(&assets.directory) {
            let entry = entry?;
            if entry.file_type().is_file() {
                required.push(entry.path().to_path_buf());
            }
        }
    }
    Ok(required)
}

fn excluded_project_path(project: &Path, build_dir: &Path, path: &Path) -> bool {
    path.starts_with(build_dir)
        || [".git", "node_modules", ".wrangler", "target"]
            .iter()
            .any(|name| path.starts_with(project.join(name)))
}

pub(crate) fn hash_tree(root: &Path, exclude: impl Fn(&Path) -> bool) -> Result<String> {
    let mut files = Vec::new();
    for entry in WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .filter_entry(|entry| !exclude(entry.path()))
    {
        let entry = entry?;
        if entry.file_type().is_file() {
            files.push(entry.path().to_path_buf());
        }
    }
    hash_paths(&files)
}

pub(crate) fn hash_paths(paths: &[PathBuf]) -> Result<String> {
    let mut paths = paths.to_vec();
    paths.sort();
    let mut hash = Sha256::new();
    for path in paths {
        let mut file = File::open(&path).with_context(|| format!("read {}", path.display()))?;
        let name = path.as_os_str().as_encoded_bytes();
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name);
        hash.update(file.metadata()?.len().to_le_bytes());
        let mut buffer = [0; 16 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hash.update(&buffer[..read]);
        }
    }
    let mut encoded = String::with_capacity(64);
    for byte in hash.finalize() {
        write!(&mut encoded, "{byte:02x}")?;
    }
    Ok(encoded)
}
