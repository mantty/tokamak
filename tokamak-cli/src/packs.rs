//! Platform packs as `tok` finds and runs them.

use std::collections::BTreeMap;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use tokamak_cli::{
    MANIFEST_FILE, Platform, PlatformPackManifest, Target, load_manifest, script_command,
};

const PLATFORM_PACK_PATH_ENV: &str = "TOKAMAK_PLATFORM_PACK_PATH";

/// A platform pack: its root directory and its manifest.
pub(crate) struct PlatformPack {
    root: PathBuf,
    pub(crate) manifest: PlatformPackManifest,
}

impl PlatformPack {
    /// The platform pack in `directory`, or else the installed one, that
    /// builds `platform`.
    pub(crate) fn load(platform: Platform, directory: Option<&Path>) -> Result<Self> {
        let manifest_path = fs::canonicalize(resolve_manifest(platform, directory)?)?;
        let manifest = load_manifest(&manifest_path)
            .with_context(|| format!("invalid platform pack: {}", manifest_path.display()))?;
        manifest
            .validate_cli_version(env!("CARGO_PKG_VERSION"))
            .with_context(|| format!("incompatible platform pack: {}", manifest_path.display()))?;
        if !platform.accepts(manifest.target) {
            bail!(
                "platform pack {} cannot build {}",
                manifest.target,
                platform.display_name()
            );
        }
        let root = manifest_path
            .parent()
            .context("platform-pack manifest must have a parent directory")?
            .to_path_buf();
        Ok(Self { root, manifest })
    }

    /// Build the app in `input` into `output`.
    pub(crate) fn build(
        &self,
        input: &Path,
        output: &Path,
        environment: &BTreeMap<String, OsString>,
    ) -> Result<()> {
        let (input, output) = (command_path(input), command_path(output));
        let arguments = [OsStr::new("build"), input.as_os_str(), output.as_os_str()];
        self.run(&arguments, environment)
    }

    /// Run the entrypoint from the pack root with `arguments`.
    pub(crate) fn run(
        &self,
        arguments: &[&OsStr],
        environment: &BTreeMap<String, OsString>,
    ) -> Result<()> {
        let entrypoint = self.root.join(self.manifest.target.build_entrypoint_path());
        if !entrypoint.is_file() {
            bail!(
                "platform pack is missing its build entrypoint: {}",
                entrypoint.display()
            );
        }
        let status = script_command(&command_path(&entrypoint))
            .args(arguments)
            .envs(environment)
            .current_dir(command_path(&self.root))
            .status()
            .with_context(|| format!("failed to run {}", entrypoint.display()))?;
        if status.success() {
            Ok(())
        } else {
            bail!("platform-pack build entrypoint failed with status {status}")
        }
    }
}

/// `path` as Bash and PowerShell accept it: without the verbatim prefix of a
/// canonical Windows drive path.
fn command_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let value = path.to_string_lossy();
        if let Some(value) = value.strip_prefix(r"\\?\")
            && value.as_bytes().get(1) == Some(&b':')
        {
            return value.into();
        }
    }
    path.into()
}

fn resolve_manifest(platform: Platform, explicit_dir: Option<&Path>) -> Result<PathBuf> {
    if let Some(directory) = explicit_dir {
        if directory.is_file() {
            bail!(
                "--platform-pack must point to a platform-pack directory, not a manifest file: {}",
                directory.display()
            );
        }
        if !directory.is_dir() {
            bail!("platform-pack directory not found: {}", directory.display());
        }
        let manifest = directory.join(MANIFEST_FILE);
        if manifest.is_file() {
            return Ok(manifest);
        }
        bail!("platform-pack manifest not found: {}", manifest.display());
    }

    let target = platform.default_target().map_err(anyhow::Error::from)?;
    if let Some(roots) = env::var_os(PLATFORM_PACK_PATH_ENV).filter(|roots| !roots.is_empty()) {
        return env::split_paths(&roots)
            .find_map(|root| manifest_in(&root, target))
            .with_context(|| {
                format!(
                    "{PLATFORM_PACK_PATH_ENV} does not contain a platform pack for {target}; npm installs @tokamakdev/platform-{target} with @tokamakdev/tok on hosts that can build it"
                )
            });
    }

    if let Some(manifest) = bundled_manifest(target).or_else(|| installed_manifest(target)) {
        return Ok(manifest);
    }

    bail!(
        "no platform pack found for {target}; install @tokamakdev/tok with npm, run the tokamak installer, pass --platform-pack, or build one with `cargo run -p xtask -- platform-pack --target {target}`"
    )
}

fn manifest_in(root: &Path, target: Target) -> Option<PathBuf> {
    let manifest = root.join(target.to_string()).join(MANIFEST_FILE);
    manifest.is_file().then_some(manifest)
}

/// The manifest under the installer's per-user data directory.
fn installed_manifest(target: Target) -> Option<PathBuf> {
    let root = env::home_dir()?.join(".local/share/tokamak/platform-packs");
    manifest_in(&root, target)
}

fn bundled_manifest(target: Target) -> Option<PathBuf> {
    let exe = env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    [
        exe_dir.join("platform-packs"),
        exe_dir.join("../share/tokamak/platform-packs"),
        exe_dir.join("../Resources/platform-packs"),
    ]
    .into_iter()
    .find_map(|root| manifest_in(&root, target))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::resolve_manifest;
    use tokamak_cli::{MANIFEST_FILE, Platform};

    #[test]
    fn resolves_an_explicit_platform_pack_directory() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let manifest = directory.path().join(MANIFEST_FILE);
        fs::write(&manifest, "manifest")?;

        assert_eq!(
            resolve_manifest(Platform::Macos, Some(directory.path()))?,
            manifest
        );
        Ok(())
    }

    #[test]
    fn rejects_an_explicit_manifest_file() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let manifest = directory.path().join(MANIFEST_FILE);
        fs::write(&manifest, "manifest")?;

        let Err(error) = resolve_manifest(Platform::Macos, Some(&manifest)) else {
            return Err("a manifest file was accepted as a platform pack".into());
        };
        assert!(
            error
                .to_string()
                .contains("must point to a platform-pack directory")
        );
        Ok(())
    }
}
