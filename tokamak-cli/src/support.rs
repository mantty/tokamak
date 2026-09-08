//! Shared native app build helpers.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use tokamak::{TokamakConfig, TokamakIcons, WranglerConfig};
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

pub(crate) fn run_web_build(project: &Path) -> Result<()> {
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

pub(crate) fn validate_web_build(config: &WranglerConfig) -> Result<()> {
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
    let Some(icons) = config.icons.as_ref() else {
        return Ok(());
    };
    let Some(source) = icon_path(icons, platform) else {
        return Ok(());
    };

    let directory_required = matches!(
        platform,
        Platform::Android | Platform::Ios | Platform::IosSimulator
    );
    if !source.exists() {
        bail!(
            "Tokamak {} icon path does not exist: {}",
            platform.display_name(),
            source.display()
        );
    }
    if source.is_dir() != directory_required {
        let expected = if directory_required {
            "directory"
        } else {
            "file"
        };
        bail!(
            "Tokamak {} icon path must be a {expected}: {}",
            platform.display_name(),
            source.display()
        );
    }

    let destination = input.join("icons").join(platform.directory_name());
    if directory_required {
        fs::create_dir_all(&destination)?;
        copy_dir_contents(source, &destination)?;
    } else {
        let expected_extension = match platform {
            Platform::Macos => "icns",
            Platform::Windows => "ico",
            Platform::Android | Platform::Ios | Platform::IosSimulator => unreachable!(),
        };
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
        if !extension.eq_ignore_ascii_case(expected_extension) {
            bail!(
                "Tokamak {} icon path must use the .{expected_extension} format: {}",
                platform.display_name(),
                source.display()
            );
        }
        let destination = destination.join(format!("AppIcon.{expected_extension}"));
        copy_file(source, destination)?;
    }
    Ok(())
}

fn icon_path(config: &TokamakIcons, platform: Platform) -> Option<&Path> {
    match platform {
        Platform::Android => config.android.as_deref(),
        Platform::Ios | Platform::IosSimulator => config.ios.as_deref(),
        Platform::Macos => config.macos.as_deref(),
        Platform::Windows => config.windows.as_deref(),
    }
}

pub(crate) fn run_entrypoint(
    pack_root: &Path,
    input: &Path,
    output: &Path,
    target: Target,
    environment: &[(&str, &std::ffi::OsStr)],
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
    for (name, value) in environment {
        command.env(name, value);
    }
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

    use tokamak::{TokamakConfig, TokamakIcons};
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
            icons: Some(TokamakIcons {
                android: Some(android),
                windows: Some(windows),
                ..TokamakIcons::default()
            }),
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
