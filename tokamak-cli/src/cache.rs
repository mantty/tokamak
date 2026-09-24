//! Content fingerprints for reusable build outputs.

use std::fmt::Write as _;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

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
