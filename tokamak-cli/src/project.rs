//! The project's own build, whose output `tok build` packages.

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use super::wrangler_config::WranglerConfig;

/// Run the project's build `command`, or its `package.json` build script, with
/// `environment` set.
pub(crate) fn build<'a>(
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

/// Fail unless the Worker and assets `config` names exist.
pub(crate) fn check_output(config: &WranglerConfig) -> Result<()> {
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
    let (program, arguments): (String, &[&str]) = if project.join("pnpm-lock.yaml").is_file() {
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

fn package_manager(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.cmd")
    } else {
        name.to_owned()
    }
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
}
