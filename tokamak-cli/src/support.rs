//! Shared native app build helpers.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use tokamak_cli::{Platform, PlatformPackManifest, Target};
use walkdir::WalkDir;

use super::wrangler_config::WranglerConfig;

pub(crate) fn validate_target(manifest: &PlatformPackManifest, platform: Platform) -> Result<()> {
    if platform.accepts(manifest.target) {
        Ok(())
    } else {
        bail!(
            "platform pack {} cannot build {}",
            manifest.target,
            platform.display_name()
        );
    }
}

/// Run the project's build `command`, or its `package.json` build script, with
/// `environment` set.
pub(crate) fn run_project_build<'a>(
    project: &Path,
    command: Option<&str>,
    environment: impl IntoIterator<Item = (&'a str, &'a OsStr)>,
) -> Result<()> {
    let mut build = match command {
        Some(command) => shell_command(command),
        None => package_build_command(project)?,
    };
    let status = build
        .envs(environment)
        .current_dir(project)
        .status()
        .with_context(|| format!("failed to run {}", build.get_program().to_string_lossy()))?;
    if status.success() {
        Ok(())
    } else {
        bail!("project build failed with status {status}")
    }
}

#[cfg(not(windows))]
fn shell_command(command: &str) -> Command {
    let mut shell = Command::new("sh");
    shell.arg("-c").arg(command);
    shell
}

#[cfg(windows)]
fn shell_command(command: &str) -> Command {
    use std::os::windows::process::CommandExt;

    let mut shell = Command::new("cmd");
    shell
        .args(["/d", "/s", "/c"])
        .raw_arg(format!("\"{command}\""));
    shell
}

fn package_build_command(project: &Path) -> Result<Command> {
    if !project.join("package.json").is_file() {
        bail!(
            "package.json not found in {}; add one or set the build command with --build or TOKAMAK_BUILD",
            project.display()
        );
    }
    let (program, arguments): (&str, &[&str]) = if project.join("pnpm-lock.yaml").is_file() {
        (package_manager("pnpm"), &["run", "build"])
    } else if project.join("yarn.lock").is_file() {
        (package_manager("yarn"), &["build"])
    } else {
        (package_manager("npm"), &["run", "build"])
    };
    let mut build = Command::new(program);
    build.args(arguments);
    Ok(build)
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

/// Run the platform pack's entrypoint from the pack root with `arguments`.
pub(crate) fn run_entrypoint(
    pack_root: &Path,
    target: Target,
    arguments: &[&OsStr],
    environment: &BTreeMap<String, OsString>,
) -> Result<()> {
    let entrypoint = pack_root.join(target.build_entrypoint_path());
    if !entrypoint.is_file() {
        bail!(
            "platform pack is missing its build entrypoint: {}",
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
        command
    } else {
        Command::new("bash")
    };
    let status = command
        .arg(command_path(&entrypoint))
        .args(arguments)
        .envs(environment)
        .current_dir(command_path(pack_root))
        .status()
        .with_context(|| format!("failed to run {}", entrypoint.display()))?;
    if status.success() {
        Ok(())
    } else {
        bail!("platform-pack build entrypoint failed with status {status}")
    }
}

/// Build the app in `input` into `output` with the platform pack.
pub(crate) fn build_with_pack(
    pack_root: &Path,
    target: Target,
    input: &Path,
    output: &Path,
    environment: &BTreeMap<String, OsString>,
) -> Result<()> {
    let (input, output) = (command_path(input), command_path(output));
    let arguments = [OsStr::new("build"), input.as_os_str(), output.as_os_str()];
    run_entrypoint(pack_root, target, &arguments, environment)
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

/// Whether `path`, with forward slashes, matches the glob `pattern`.
pub(crate) fn glob_matches(pattern: &str, path: &str) -> bool {
    let pattern = pattern.trim_start_matches("./");
    let path = path.trim_start_matches("./");
    let pattern = pattern.split('/').collect::<Vec<_>>();
    let path = path.split('/').collect::<Vec<_>>();
    glob_segments(&pattern, &path)
}

fn glob_segments(pattern: &[&str], path: &[&str]) -> bool {
    match pattern {
        [] => path.is_empty(),
        ["**", rest @ ..] => {
            glob_segments(rest, path) || (!path.is_empty() && glob_segments(pattern, &path[1..]))
        }
        [segment, rest @ ..] => {
            !path.is_empty() && segment_matches(segment, path[0]) && glob_segments(rest, &path[1..])
        }
    }
}

fn segment_matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    let mut states = vec![(0, 0)];
    while let Some((pattern_index, value_index)) = states.pop() {
        if pattern_index == pattern.len() {
            if value_index == value.len() {
                return true;
            }
            continue;
        }
        match pattern[pattern_index] {
            b'*' => {
                states.push((pattern_index + 1, value_index));
                if value_index < value.len() {
                    states.push((pattern_index, value_index + 1));
                }
            }
            b'?' if value_index < value.len() => {
                states.push((pattern_index + 1, value_index + 1));
            }
            character if value_index < value.len() && character == value[value_index] => {
                states.push((pattern_index + 1, value_index + 1));
            }
            _ => {}
        }
    }
    false
}

/// `path` with forward slashes.
pub(crate) fn slash_path(path: &Path) -> Result<String> {
    let path = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("path is not valid UTF-8: {}", path.display()))?;
    Ok(path.replace(std::path::MAIN_SEPARATOR, "/"))
}

#[cfg(test)]
mod tests {
    use super::package_manager;

    #[test]
    fn selects_platform_package_manager_commands() {
        let suffix = if cfg!(windows) { ".cmd" } else { "" };
        for name in ["npm", "pnpm", "yarn"] {
            assert_eq!(package_manager(name), format!("{name}{suffix}"));
        }
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
