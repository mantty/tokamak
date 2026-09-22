//! Shared native app build helpers.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use tokamak::{TokamakConfig, WranglerConfig};
use tokamak_cli::{ArtifactKind, Platform, Target, TargetPackManifest};
use walkdir::WalkDir;

pub(crate) fn artifact_path(
    pack_root: &Path,
    manifest: &TargetPackManifest,
    kind: &ArtifactKind,
) -> Result<PathBuf> {
    manifest
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == *kind)
        .map(|artifact| pack_root.join(&artifact.path))
        .with_context(|| format!("target pack missing {kind:?} artifact"))
}

pub(crate) fn validate_target(manifest: &TargetPackManifest, platform: Platform) -> Result<()> {
    if platform.accepts(manifest.target) {
        Ok(())
    } else {
        bail!(
            "target pack {} cannot build {}",
            manifest.target,
            platform.display_name()
        );
    }
}

pub(crate) fn run_project_build(project: &Path) -> Result<()> {
    if !project.join("package.json").is_file() {
        bail!("package.json not found in {}", project.display());
    }
    let (program, arguments): (&str, &[&str]) = if project.join("pnpm-lock.yaml").is_file() {
        (package_manager("pnpm"), &["run", "build"])
    } else if project.join("yarn.lock").is_file() {
        (package_manager("yarn"), &["build"])
    } else {
        (package_manager("npm"), &["run", "build"])
    };
    let status = Command::new(program)
        .args(arguments)
        .current_dir(project)
        .status()
        .with_context(|| format!("failed to run {program}"))?;
    if status.success() {
        Ok(())
    } else {
        bail!("{program} build failed with status {status}")
    }
}

pub(crate) fn package_manager(name: &'static str) -> &'static str {
    if cfg!(windows) {
        match name {
            "npm" => "npm.cmd",
            "pnpm" => "pnpm.cmd",
            "yarn" => "yarn.cmd",
            _ => name,
        }
    } else {
        name
    }
}

pub(crate) fn validate_project_build(config: &WranglerConfig) -> Result<()> {
    if !config.main.is_file() {
        bail!("worker main not found: {}", config.main.display());
    }
    if let Some(assets) = &config.assets
        && !assets.directory.is_dir()
    {
        bail!("assets directory not found: {}", assets.directory.display());
    }
    Ok(())
}

pub(crate) fn build_dir(project: &Path, platform: Platform) -> PathBuf {
    project.join("build").join(platform.directory_name())
}

pub(crate) fn command_path(path: &Path) -> PathBuf {
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

pub(crate) fn reset_path(path: &Path) -> Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)?;
    } else if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub(crate) fn stage_platform_artifacts(
    input: &Path,
    pack_root: &Path,
    manifest: &TargetPackManifest,
) -> Result<()> {
    let target = manifest.target;
    let runtime = artifact_path(pack_root, manifest, &target.runtime_artifact_kind())?;
    let destination = input.join(target.runtime_staging_path());
    if runtime.is_dir() {
        copy_dir_contents(&runtime, &destination)?;
    } else {
        copy_file(runtime, destination)?;
    }

    if target.has_native_shell() {
        copy_dir_contents(
            &artifact_path(pack_root, manifest, &ArtifactKind::NativeShellDirectory)?,
            &input.join("native-shell"),
        )?;
    }
    Ok(())
}

pub(crate) fn stage_platform_icons(
    input: &Path,
    config: &TokamakConfig,
    platform: Platform,
) -> Result<()> {
    let Some(source) = config.icon.for_platform(platform.directory_name()) else {
        return Ok(());
    };

    if !source.exists() {
        bail!(
            "Tokamak {} icon path does not exist: {}",
            platform.display_name(),
            source.display()
        );
    }

    let destination = input.join("icons").join(platform.directory_name());
    match platform {
        Platform::Android => {
            if !source.is_dir() {
                bail!(
                    "Tokamak {} icon path must be a directory: {}",
                    platform.display_name(),
                    source.display()
                );
            }
            fs::create_dir_all(&destination)?;
            copy_dir_contents(source, &destination)?;
        }
        Platform::Ios | Platform::IosSimulator | Platform::Macos => {
            if !source.is_dir() {
                bail!(
                    "Tokamak {} icon path must be an .icon directory: {}",
                    platform.display_name(),
                    source.display()
                );
            }
            let is_icon_package = source
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("icon"));
            if !is_icon_package {
                bail!(
                    "Tokamak {} icon path must use the .icon format: {}",
                    platform.display_name(),
                    source.display()
                );
            }
            let destination = destination.join("AppIcon.icon");
            fs::create_dir_all(&destination)?;
            copy_dir_contents(source, &destination)?;
        }
        Platform::Windows => {
            if source.is_dir() {
                bail!(
                    "Tokamak {} icon path must be a file: {}",
                    platform.display_name(),
                    source.display()
                );
            }
            let extension = source
                .extension()
                .and_then(|extension| extension.to_str())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Tokamak {} icon path must have a file extension: {}",
                        platform.display_name(),
                        source.display()
                    )
                })?;
            if !extension.eq_ignore_ascii_case("ico") {
                bail!(
                    "Tokamak {} icon path must use the .ico format: {}",
                    platform.display_name(),
                    source.display()
                );
            }
            copy_file(source, destination.join("AppIcon.ico"))?;
        }
    }
    Ok(())
}

pub(crate) fn run_entrypoint(
    pack_root: &Path,
    input: &Path,
    output: &Path,
    target: Target,
    environment: &std::collections::BTreeMap<String, std::ffi::OsString>,
) -> Result<()> {
    let entrypoint = pack_root.join(target.build_entrypoint_path());
    if !entrypoint.is_file() {
        bail!(
            "target pack is missing its build entrypoint: {}",
            entrypoint.display()
        );
    }

    let mut command = if cfg!(windows)
        && entrypoint
            .extension()
            .is_some_and(|extension| extension == "ps1")
    {
        let mut command = Command::new("powershell");
        command.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]);
        command.arg(command_path(&entrypoint));
        command
    } else {
        let mut command = Command::new("bash");
        command.arg(command_path(&entrypoint));
        command
    };
    command.envs(environment);
    let status = command
        .args(["build"])
        .arg(command_path(input))
        .arg(command_path(output))
        .current_dir(command_path(pack_root))
        .status()
        .with_context(|| format!("failed to run {}", entrypoint.display()))?;
    if status.success() {
        Ok(())
    } else {
        bail!("target-pack build entrypoint failed with status {status}")
    }
}

pub(crate) fn copy_file(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<()> {
    let from = from.as_ref();
    let to = to.as_ref();
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(from, to).with_context(|| format!("copy {} to {}", from.display(), to.display()))?;
    Ok(())
}

pub(crate) fn copy_dir_contents(from: &Path, to: &Path) -> Result<()> {
    for entry in WalkDir::new(from).follow_links(true) {
        let entry = entry.map_err(std::io::Error::other)?;
        let relative = entry.path().strip_prefix(from)?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        let destination = to.join(relative);
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            fs::create_dir_all(destination)?;
        } else if metadata.is_file() {
            copy_file(entry.path(), destination)?;
        } else {
            bail!("unsupported file in {}", entry.path().display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tokamak::{PlatformValues, TokamakConfig};
    use tokamak_cli::Platform;

    use super::{package_manager, stage_platform_icons};

    #[test]
    fn selects_platform_package_manager_commands() {
        let suffix = if cfg!(windows) { ".cmd" } else { "" };
        for name in ["npm", "pnpm", "yarn"] {
            assert_eq!(package_manager(name), format!("{name}{suffix}"));
        }
    }

    #[test]
    fn stages_only_the_configured_platform_icons() -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempfile::tempdir()?;
        let android = temporary.path().join("android");
        fs::create_dir_all(android.join("mipmap-mdpi"))?;
        fs::write(android.join("mipmap-mdpi/ic_launcher.png"), "png")?;
        let windows = temporary.path().join("icon.ico");
        fs::write(&windows, "ico")?;
        let config = TokamakConfig {
            icon: PlatformValues {
                android: Some(android),
                windows: Some(windows),
                ..PlatformValues::default()
            },
            ..TokamakConfig::default()
        };

        let android_input = temporary.path().join("android-input");
        stage_platform_icons(&android_input, &config, Platform::Android)?;
        assert_eq!(
            fs::read_to_string(android_input.join("icons/android/mipmap-mdpi/ic_launcher.png"))?,
            "png"
        );

        let windows_input = temporary.path().join("windows-input");
        stage_platform_icons(&windows_input, &config, Platform::Windows)?;
        assert_eq!(
            fs::read_to_string(windows_input.join("icons/windows/AppIcon.ico"))?,
            "ico"
        );
        Ok(())
    }

    #[test]
    fn absent_platform_icon_does_not_create_staging_files() -> Result<(), Box<dyn std::error::Error>>
    {
        let temporary = tempfile::tempdir()?;

        stage_platform_icons(
            &temporary.path().join("input"),
            &TokamakConfig::default(),
            Platform::Macos,
        )?;

        assert!(!temporary.path().join("input").exists());
        Ok(())
    }

    #[test]
    fn stages_apple_icon_packages_for_both_platforms() -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempfile::tempdir()?;
        let source = temporary.path().join("Brand.icon");
        fs::create_dir(&source)?;
        fs::write(source.join("icon.json"), "icon")?;
        let config = TokamakConfig {
            icon: PlatformValues {
                ios: Some(source.clone()),
                macos: Some(source),
                ..PlatformValues::default()
            },
            ..TokamakConfig::default()
        };

        for platform in [Platform::Ios, Platform::Macos] {
            let input = temporary.path().join(platform.directory_name());
            stage_platform_icons(&input, &config, platform)?;
            assert_eq!(
                fs::read_to_string(input.join(format!(
                    "icons/{}/AppIcon.icon/icon.json",
                    platform.directory_name()
                )))?,
                "icon"
            );
        }
        Ok(())
    }

    #[test]
    fn rejects_non_icon_apple_packages() -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempfile::tempdir()?;
        let source = temporary.path().join("AppIcon.invalid");
        fs::create_dir(&source)?;
        let config = TokamakConfig {
            icon: PlatformValues {
                macos: Some(source),
                ..PlatformValues::default()
            },
            ..TokamakConfig::default()
        };

        let Err(error) =
            stage_platform_icons(&temporary.path().join("input"), &config, Platform::Macos)
        else {
            return Err(std::io::Error::other("non-.icon package was accepted").into());
        };
        assert!(error.to_string().contains("must use the .icon format"));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn copies_link_targets_as_regular_files() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        let temporary = tempfile::tempdir()?;
        let source = temporary.path().join("source");
        let destination = temporary.path().join("destination");
        std::fs::create_dir_all(&source)?;
        std::fs::write(source.join("target"), "content")?;
        symlink("target", source.join("link"))?;

        super::copy_dir_contents(&source, &destination)?;

        assert_eq!(
            std::fs::read_to_string(destination.join("link"))?,
            "content"
        );
        assert!(
            !std::fs::symlink_metadata(destination.join("link"))?
                .file_type()
                .is_symlink()
        );
        Ok(())
    }
}
