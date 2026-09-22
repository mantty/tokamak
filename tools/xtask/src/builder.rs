use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use tokamak_cli::{
    Artifact, ESBUILD_DIRECTORY, PlatformPackManifest, RUNTIME_DIRECTORY, Target, write_manifest,
};

use crate::layout::WorkspaceLayout;
use crate::support::copy_dir_contents;

const ESBUILD_HOSTS: &[&str] = &["darwin-arm64", "darwin-x64", "linux-x64", "win32-x64"];
const ESBUILD_LAUNCHER: &str = include_str!("esbuild-launcher.cjs");
const ESBUILD_LICENSE: &str = include_str!("esbuild-license.txt");

pub(crate) fn build_source_platform_pack(target: Target) -> Result<PathBuf> {
    let workspace = WorkspaceLayout::from_source()
        .context("source workspace is unavailable; run this command from a tokamak checkout")?;
    build_source_platform_pack_at(&workspace, target)
}

fn build_source_platform_pack_at(workspace: &WorkspaceLayout, target: Target) -> Result<PathBuf> {
    let pack_dir = workspace.platform_pack(target);
    let recipe_output = workspace.recipe_output(target);
    reset_dir(&pack_dir)?;
    reset_dir(&recipe_output)?;

    run_platform_recipe(workspace, target, &recipe_output)?;
    copy_dir_contents(&recipe_output, &pack_dir).context("copy platform-pack artifacts")?;
    fs::remove_dir_all(&recipe_output)?;

    deploy_runtime(workspace, &pack_dir)?;
    let artifacts = target.artifacts();
    validate_artifacts(&pack_dir, &artifacts)?;

    let manifest = PlatformPackManifest {
        tokamak_version: env!("CARGO_PKG_VERSION").to_owned(),
        target,
        artifacts,
        required_tools: target
            .required_tools()
            .iter()
            .map(|tool| (*tool).to_owned())
            .collect(),
    };
    let manifest_path = workspace.manifest(target);
    write_manifest(&manifest_path, &manifest)?;
    Ok(manifest_path)
}

fn run_platform_recipe(workspace: &WorkspaceLayout, target: Target, output: &Path) -> Result<()> {
    let recipe = workspace.platform_recipe(target);
    if !recipe.is_file() {
        bail!("platform-pack recipe is missing: {}", recipe.display());
    }

    let mut command = if recipe
        .extension()
        .is_some_and(|extension| extension == "ps1")
    {
        let mut command = Command::new("powershell");
        command.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]);
        command.arg(&recipe);
        command
    } else {
        let mut command = Command::new("bash");
        command.arg(&recipe);
        command
    };
    let target_name = target.to_string();
    let status = command
        .args(["build", &target_name, target.rust_target()])
        .arg(output)
        .current_dir(workspace.root())
        .status()
        .with_context(|| format!("failed to run {}", recipe.display()))?;
    if status.success() {
        Ok(())
    } else {
        bail!(
            "platform-pack recipe failed with status {status}: {}",
            recipe.display()
        )
    }
}

fn deploy_runtime(workspace: &WorkspaceLayout, pack_root: &Path) -> Result<()> {
    let runtime = pack_root.join(RUNTIME_DIRECTORY);
    reset_dir(&runtime)?;
    fs::create_dir_all(pack_root.join(ESBUILD_DIRECTORY).join("bin"))?;
    fs::write(
        pack_root.join(tokamak_cli::ESBUILD_EXECUTABLE),
        ESBUILD_LAUNCHER,
    )?;
    fs::write(
        pack_root.join(ESBUILD_DIRECTORY).join("LICENSE.md"),
        ESBUILD_LICENSE,
    )?;
    for host in ESBUILD_HOSTS {
        copy_dir_contents(
            &workspace.esbuild_host_package(host),
            &pack_root
                .join("tools/runtime/node_modules/@esbuild")
                .join(host),
        )
        .with_context(|| format!("copy esbuild binary for {host}"))?;
    }
    Ok(())
}

fn validate_artifacts(root: &Path, artifacts: &[Artifact]) -> Result<()> {
    for artifact in artifacts {
        let path = root.join(&artifact.path);
        if !path.exists() {
            bail!(
                "platform-pack artifact was not produced: {}",
                path.display()
            );
        }
    }
    Ok(())
}

fn reset_dir(path: &Path) -> Result<()> {
    if path.is_symlink() || path.is_file() {
        fs::remove_file(path)?;
    } else if path.is_dir() {
        fs::remove_dir_all(path)?;
    }
    fs::create_dir_all(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use super::{deploy_runtime, validate_artifacts};
    use tokamak_cli::{Artifact, ArtifactKind, ESBUILD_EXECUTABLE};

    use crate::layout::WorkspaceLayout;

    #[test]
    fn rejects_missing_pack_artifacts() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let result = validate_artifacts(
            directory.path(),
            &[Artifact {
                kind: ArtifactKind::RuntimeLibrary,
                path: "runtime".to_owned(),
            }],
        );

        assert!(result.is_err_and(|error| {
            error
                .to_string()
                .contains("platform-pack artifact was not produced")
        }));
        Ok(())
    }

    #[test]
    fn accepts_files_and_directories_from_the_recipe() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        fs::create_dir(directory.path().join("runtime"))?;
        fs::write(directory.path().join("runtime/file"), "runtime")?;

        validate_artifacts(
            directory.path(),
            &[Artifact {
                kind: ArtifactKind::RuntimeLibrary,
                path: "runtime".to_owned(),
            }],
        )?;
        Ok(())
    }

    #[test]
    fn packages_esbuild_for_every_cli_host() -> anyhow::Result<()> {
        let workspace_root = tempfile::tempdir()?;
        create_esbuild_install(workspace_root.path())?;
        let pack_root = tempfile::tempdir()?;

        deploy_runtime(
            &WorkspaceLayout::from_root(workspace_root.path()),
            pack_root.path(),
        )?;

        for path in [
            "darwin-arm64/bin/esbuild",
            "darwin-x64/bin/esbuild",
            "linux-x64/bin/esbuild",
            "win32-x64/esbuild.exe",
        ] {
            assert!(
                pack_root
                    .path()
                    .join("tools/runtime/node_modules/@esbuild")
                    .join(path)
                    .is_file(),
                "missing packaged esbuild binary for {path}"
            );
        }
        assert!(
            pack_root
                .path()
                .join("tools/runtime/node_modules/esbuild/LICENSE.md")
                .is_file()
        );
        let launcher = pack_root.path().join(ESBUILD_EXECUTABLE);
        assert_launcher_paths(&launcher)?;
        assert_launcher_runs(pack_root.path(), &launcher)?;
        Ok(())
    }

    fn assert_launcher_paths(launcher: &Path) -> anyhow::Result<()> {
        for (platform, arch, expected) in [
            ("darwin", "arm64", "@esbuild/darwin-arm64/bin/esbuild"),
            ("darwin", "x64", "@esbuild/darwin-x64/bin/esbuild"),
            ("linux", "x64", "@esbuild/linux-x64/bin/esbuild"),
            ("win32", "x64", "@esbuild/win32-x64/esbuild.exe"),
        ] {
            let output = Command::new("node")
                .args([
                    "-e",
                    "const {esbuildPath}=require(process.argv[1]); console.log(esbuildPath(process.argv[2], process.argv[3]))",
                ])
                .arg(launcher)
                .args([platform, arch])
                .output()?;
            assert!(output.status.success());
            assert!(
                String::from_utf8(output.stdout)?
                    .trim()
                    .replace('\\', "/")
                    .ends_with(expected),
                "launcher resolved the wrong esbuild binary for {platform}-{arch}"
            );
        }
        Ok(())
    }

    fn assert_launcher_runs(pack_root: &Path, launcher: &Path) -> anyhow::Result<()> {
        let node = Command::new("node")
            .args(["-p", "process.execPath"])
            .output()?;
        assert!(node.status.success());
        let node = String::from_utf8(node.stdout)?;
        let current_binary = if cfg!(windows) {
            "win32-x64/esbuild.exe"
        } else if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
            "darwin-arm64/bin/esbuild"
        } else if cfg!(target_os = "macos") {
            "darwin-x64/bin/esbuild"
        } else {
            "linux-x64/bin/esbuild"
        };
        fs::copy(
            node.trim(),
            pack_root
                .join("tools/runtime/node_modules/@esbuild")
                .join(current_binary),
        )?;
        let output = Command::new("node")
            .arg(launcher)
            .arg("--version")
            .output()?;
        assert!(output.status.success());
        Ok(())
    }

    fn create_esbuild_install(root: &Path) -> anyhow::Result<()> {
        for path in [
            "darwin-arm64/bin/esbuild",
            "darwin-x64/bin/esbuild",
            "linux-x64/bin/esbuild",
            "win32-x64/esbuild.exe",
        ] {
            let binary = root
                .join("tools/esbuild-hosts/node_modules/@esbuild")
                .join(path);
            fs::create_dir_all(
                binary
                    .parent()
                    .ok_or_else(|| anyhow::anyhow!("esbuild binary has no parent"))?,
            )?;
            fs::write(binary, "esbuild")?;
        }
        Ok(())
    }
}
