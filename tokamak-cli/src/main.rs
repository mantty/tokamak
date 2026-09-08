#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! `tokamak` command line entry point.

mod dev;
mod devices;
mod ios_signing;
mod pipeline;
mod plugins;
mod support;
mod worker;

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tokamak_cli::{Platform, Target};

#[derive(Debug, Parser)]
#[command(name = "tok", version, about = "tokamak native app tooling")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Build native app bundles for one or more platforms.
    Build {
        /// Comma-separated platform families to build.
        platforms: String,
        /// App project directory.
        #[arg(long, default_value = ".")]
        project: PathBuf,
        /// Target-pack directory containing target-pack.json.
        #[arg(long = "target-pack")]
        target_pack: Option<PathBuf>,
        /// Tokamak configuration file or directory. Defaults to the current directory.
        #[arg(short = 'c', long = "config", default_value = ".")]
        config: PathBuf,
        /// Path to the Wrangler configuration file.
        #[arg(short = 'w', long = "wrangler")]
        wrangler: Option<PathBuf>,
        /// Reuse an existing dist/ directory instead of running the web build.
        #[arg(long)]
        skip_web_build: bool,
    },
    /// Run a development server against one target.
    Dev {
        /// Device selector (for example, macos, ios, android, or a native ID).
        #[arg(value_name = "DEVICE_ID")]
        device_id: String,
        /// App project directory.
        #[arg(long, default_value = ".")]
        project: PathBuf,
        /// Target-pack directory containing target-pack.json.
        #[arg(long = "target-pack")]
        target_pack: Option<PathBuf>,
        /// Tokamak configuration file or directory. Defaults to the current directory.
        #[arg(short = 'c', long = "config", default_value = ".")]
        config: PathBuf,
        /// Path to the Wrangler configuration file.
        #[arg(short = 'w', long = "wrangler")]
        wrangler: Option<PathBuf>,
        /// HTTP endpoint served by the framework's development command.
        #[arg(long, value_name = "URL", default_value = "http://localhost:5173")]
        server: String,
        /// Override the detected host address for a physical iOS device.
        #[arg(long, value_name = "ADDRESS")]
        host_address: Option<String>,
        /// Dev command after `--` (for example, `astro dev`).
        #[arg(last = true, value_name = "COMMAND", num_args = 1..)]
        command: Vec<OsString>,
    },
    /// List concrete and provisionable development targets.
    Devices,
    /// List runtime targets supported by this CLI.
    Targets,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Build {
            platforms,
            project,
            target_pack,
            config,
            wrangler,
            skip_web_build,
        } => {
            let platforms = parse_platforms(&platforms)?;
            let summaries = pipeline::run(&pipeline::BuildRequest {
                platforms,
                project_dir: project,
                target_pack_dir: target_pack,
                tokamak_config_path: config,
                wrangler_config_path: wrangler,
                skip_web_build,
            })?;
            for summary in summaries {
                println!(
                    "Built {} bundle: {}",
                    summary.platform.display_name(),
                    summary.bundle_dir.display()
                );
            }
            Ok(())
        }
        Command::Dev {
            device_id,
            project,
            target_pack,
            config,
            wrangler,
            server,
            host_address,
            command,
        } => {
            let request = dev::Request {
                device_id,
                project_dir: project,
                target_pack_dir: target_pack,
                tokamak_config_path: config,
                wrangler_config_path: wrangler,
                server,
                host_address,
                command,
            };
            dev::run(&request)
        }
        Command::Devices => {
            devices::list();
            Ok(())
        }
        Command::Targets => {
            list_targets();
            Ok(())
        }
    }
}

fn parse_platforms(value: &str) -> Result<Vec<Platform>> {
    value
        .split(',')
        .map(|platform| platform.parse().map_err(anyhow::Error::from))
        .collect()
}

fn list_targets() {
    for target in Target::ALL {
        println!("{target}");
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;

    use clap::Parser;

    use super::{Cli, Command, Platform, parse_platforms};

    #[test]
    fn parses_comma_separated_platforms() {
        assert!(matches!(
            parse_platforms("macos,android"),
            Ok(platforms) if platforms == vec![Platform::Macos, Platform::Android]
        ));
    }

    #[test]
    fn accepts_one_comma_separated_platform_argument() {
        assert!(matches!(
            Cli::try_parse_from(["tok", "build", "macos,android"]),
            Ok(Cli { command: Command::Build { platforms, .. } })
                if platforms == "macos,android"
        ));
    }

    #[test]
    fn defaults_tokamak_config_to_the_current_directory() {
        assert!(matches!(
            Cli::try_parse_from(["tok", "build", "macos"]),
            Ok(Cli {
                command: Command::Build {
                    config,
                    wrangler,
                    ..
                }
            }) if config.as_path() == Path::new(".") && wrangler.is_none()
        ));
    }

    #[test]
    fn keeps_tokamak_and_wrangler_config_paths_separate() {
        assert!(matches!(
            Cli::try_parse_from([
                "tok",
                "build",
                "macos",
                "--config",
                "tokamak.jsonc",
                "--wrangler",
                "dist/wrangler.json"
            ]),
            Ok(Cli {
                command: Command::Build {
                    config,
                    wrangler: Some(wrangler),
                    ..
                }
            }) if config.as_path() == Path::new("tokamak.jsonc")
                && wrangler.as_path() == Path::new("dist/wrangler.json")
        ));
    }

    #[test]
    fn accepts_devices_command() {
        assert!(matches!(
            Cli::try_parse_from(["tok", "devices"]),
            Ok(Cli {
                command: Command::Devices
            })
        ));
    }

    #[test]
    fn accepts_dev_command() {
        assert!(matches!(
            Cli::try_parse_from(["tok", "dev", "ios", "--", "astro", "dev"]),
            Ok(Cli {
                command: Command::Dev { device_id, command, .. }
            }) if device_id == "ios"
                && command == vec![OsString::from("astro"), OsString::from("dev")]
        ));
    }

    #[test]
    fn rejects_multiple_positional_platform_arguments() {
        assert!(Cli::try_parse_from(["tok", "build", "macos", "android"]).is_err());
    }

    #[test]
    fn rejects_the_removed_platforms_flag() {
        assert!(Cli::try_parse_from(["tok", "build", "--platforms=macos"]).is_err());
    }
}
