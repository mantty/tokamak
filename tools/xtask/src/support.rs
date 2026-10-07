use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

/// An empty directory at `path`, replacing whatever was there.
pub(crate) fn reset_dir(path: &Path) -> Result<()> {
    if path.is_symlink() || path.is_file() {
        fs::remove_file(path)?;
    } else if path.is_dir() {
        fs::remove_dir_all(path)?;
    }
    fs::create_dir_all(path)?;
    Ok(())
}

/// Run `command`, failing unless it succeeds.
pub(crate) fn run(command: &mut Command) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("failed to run {}", command.get_program().to_string_lossy()))?;
    if status.success() {
        Ok(())
    } else {
        bail!(
            "{} failed with status {status}",
            command.get_program().to_string_lossy()
        )
    }
}
