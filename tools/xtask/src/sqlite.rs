//! Regenerate the SQLite source in the vendored `libsqlite3-sys`.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::layout::WorkspaceLayout;
use crate::support::{copy_file, reset_dir};

/// The SQLite release workerd builds.
const VERSION: &str = "3530400";
const SOURCE_YEAR: &str = "2026";
const SOURCE_SHA256: &str = "d18fa15aec74d8c17e1463f861095adc01b5ad190256acb4f91d22f0368d232b";
const GENERATED: [&str; 3] = ["sqlite3.c", "sqlite3.h", "sqlite3ext.h"];

/// Download the pinned SQLite source, apply the vendored patches, and write
/// the single-file source built with the workspace's SQLite options.
pub(crate) fn regenerate() -> Result<()> {
    let workspace = WorkspaceLayout::from_source()
        .context("source workspace is unavailable; run this command from a tokamak checkout")?;
    let options = options(&workspace.root().join(".cargo/config.toml"))?;
    let work = workspace.root().join("target/sqlite-source");
    reset_dir(&work)?;
    let archive = work.join("source.zip");
    let url = format!("https://sqlite.org/{SOURCE_YEAR}/sqlite-src-{VERSION}.zip");
    run(Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(&archive)
        .arg(&url))?;
    verify(&archive, &url)?;
    run(Command::new("unzip")
        .arg("-q")
        .arg(&archive)
        .arg("-d")
        .arg(&work))?;
    let source = work.join(format!("sqlite-src-{VERSION}"));
    let vendored = workspace.root().join("libsqlite3-sys");
    for patch in patches(&vendored.join("patches"))? {
        run(Command::new("patch")
            .args(["-p1", "--no-backup-if-mismatch", "-i"])
            .arg(&patch)
            .current_dir(&source))?;
    }
    run(Command::new("./configure")
        .arg("--disable-tcl")
        .current_dir(&source))?;
    run(Command::new("make")
        .arg("sqlite3.c")
        .arg("OPT_FEATURE_FLAGS=")
        .arg(format!("OPTS={options}"))
        .current_dir(&source))?;
    for file in GENERATED {
        copy_file(source.join(file), vendored.join("sqlite3").join(file))?;
    }
    fs::remove_dir_all(&work)?;
    Ok(())
}

/// The workspace's `LIBSQLITE3_FLAGS`, on one line.
fn options(path: &Path) -> Result<String> {
    let config: toml::Table = toml::from_str(&fs::read_to_string(path)?)?;
    let flags = config
        .get("env")
        .and_then(|env| env.get("LIBSQLITE3_FLAGS"))
        .and_then(toml::Value::as_str)
        .with_context(|| format!("{} does not set env.LIBSQLITE3_FLAGS", path.display()))?;
    Ok(flags.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn verify(archive: &Path, url: &str) -> Result<()> {
    let digest = Sha256::digest(fs::read(archive)?);
    let hex = digest.iter().fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    });
    if hex == SOURCE_SHA256 {
        Ok(())
    } else {
        bail!("{url} has SHA-256 {hex}, not {SOURCE_SHA256}")
    }
}

fn patches(directory: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut patches = fs::read_dir(directory)?
        .map(|entry| Ok(entry?.path()))
        .collect::<Result<Vec<_>>>()?;
    patches.retain(|path| {
        path.extension()
            .is_some_and(|extension| extension == "patch")
    });
    patches.sort();
    Ok(patches)
}

fn run(command: &mut Command) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::options;

    #[test]
    fn reads_the_workspace_sqlite_options_on_one_line() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let config = directory.path().join("config.toml");
        std::fs::write(
            &config,
            "[env]\nLIBSQLITE3_FLAGS = \"\"\"\n-DSQLITE_ENABLE_FTS5\n-USQLITE_USE_URI\n\"\"\"\n",
        )?;

        assert_eq!(options(&config)?, "-DSQLITE_ENABLE_FTS5 -USQLITE_USE_URI");
        Ok(())
    }
}
