//! Path helpers.

use std::fs;
use std::path::Path;

use anyhow::Result;

/// Remove whatever is at `path`.
pub(crate) fn reset_path(path: &Path) -> Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)?;
    } else if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

/// `path` with forward slashes.
pub(crate) fn slash_path(path: &Path) -> Result<String> {
    let path = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("path is not valid UTF-8: {}", path.display()))?;
    Ok(path.replace(std::path::MAIN_SEPARATOR, "/"))
}
