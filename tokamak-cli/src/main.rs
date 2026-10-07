#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! `tokamak` command line entry point.

mod cache;
mod certs;
mod dev;
mod devices;
mod packs;
mod paths;
mod pinned_tls;
mod pipeline;
mod plugins;
mod project;
mod settings;
mod storage;
mod tokamak_config;
mod vite;
mod worker;
mod wrangler_config;

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use packs::PlatformPack;
use settings::{PlatformOptions, TopOptions};
use tokamak_cli::{Platform, Target};

const BUILD_PLATFORM_HELP: &str = "Platform options: run `tok build <platforms> --help` to list the options each platform accepts.";
const DEV_PLATFORM_HELP: &str =
    "Platform options: run `tok dev <platform> --help` to list the options the platform accepts.";

#[derive(Debug, Parser)]
#[command(name = "tok", bin_name = "tok", about = "tokamak native app tooling")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Build native app bundles for one or more platforms.
    #[command(after_help = BUILD_PLATFORM_HELP)]
    Build {
        /// Comma-separated platform families to build.
        platforms: String,
        /// App project directory.
        #[arg(long, default_value = ".")]
        project: PathBuf,
        /// Build output and cache directory, relative to the project by default.
        #[arg(long = "build-dir")]
        build_dir: Option<PathBuf>,
        /// Platform-pack directory containing platform-pack.json.
        #[arg(long = "platform-pack")]
        platform_pack: Option<PathBuf>,
        #[command(flatten)]
        top: TopOptions,
        /// Command that builds the project [`TOKAMAK_BUILD`].
        #[arg(long = "build", value_name = "COMMAND")]
        build_command: Option<String>,
        /// Package the previous build's output instead of building the project.
        #[arg(long)]
        skip_project_build: bool,
    },
    /// Run a development server against one target.
    #[command(after_help = DEV_PLATFORM_HELP)]
    Dev {
        /// Device selector (for example, macos, ios, android, or a native ID).
        #[arg(value_name = "DEVICE_ID")]
        device_id: String,
        /// App project directory.
        #[arg(long, default_value = ".")]
        project: PathBuf,
        /// Platform-pack directory containing platform-pack.json.
        #[arg(long = "platform-pack")]
        platform_pack: Option<PathBuf>,
        #[command(flatten)]
        top: TopOptions,
        /// Override the detected host address for a physical iOS device.
        #[arg(long, value_name = "ADDRESS")]
        host_address: Option<String>,
        /// Dev command after `--` (for example, `astro dev`).
        #[arg(last = true, value_name = "COMMAND", num_args = 1..)]
        command: Vec<OsString>,
    },
    /// List concrete and provisionable development targets.
    Devices,
    /// List local iOS signing identities and provisioning profiles (macOS only).
    Certs,
    /// List runtime targets supported by this CLI.
    Targets,
    /// Print the version of this CLI.
    Version,
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
    let (arguments, platform_options) = split_arguments(env::args_os().collect())?;
    let matches = command(&arguments).get_matches_from(arguments);
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|error| error.exit());

    match cli.command {
        Command::Build {
            platforms,
            project,
            build_dir,
            platform_pack,
            top,
            build_command,
            skip_project_build,
        } => {
            let platforms = parse_platforms(&platforms)?;
            let summaries = pipeline::run(&pipeline::BuildRequest {
                platforms,
                project_dir: project,
                build_dir,
                platform_pack_dir: platform_pack,
                top,
                platform_options,
                build_command,
                skip_project_build,
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
            platform_pack,
            top,
            host_address,
            command,
        } => dev::run(&dev::Request {
            device_id,
            project_dir: project,
            platform_pack_dir: platform_pack,
            top,
            platform_options,
            host_address,
            command,
        }),
        Command::Devices => {
            devices::list();
            Ok(())
        }
        Command::Certs => certs::list(),
        Command::Targets => {
            list_targets();
            Ok(())
        }
        Command::Version => {
            println!("tok {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}

/// Split platform options from the arguments of the commands that take them.
fn split_arguments(arguments: Vec<OsString>) -> Result<(Vec<OsString>, PlatformOptions)> {
    let takes_platform_options = arguments
        .get(1)
        .is_some_and(|command| command == "build" || command == "dev");
    if takes_platform_options {
        settings::split_platform_options(arguments)
    } else {
        Ok((arguments, PlatformOptions::default()))
    }
}

/// The argument parser; `--help` for named platforms lists their options.
fn command(arguments: &[OsString]) -> clap::Command {
    let command = Cli::command();
    let is_help = |argument: &&OsString| *argument == "--help" || *argument == "-h";
    let asks_for_help = arguments
        .iter()
        .take_while(|argument| *argument != "--")
        .any(|argument| is_help(&argument));
    if !asks_for_help {
        return command;
    }
    let without_help = arguments.iter().filter(|argument| !is_help(argument));
    let Ok(cli) = Cli::try_parse_from(without_help) else {
        return command;
    };
    let (subcommand, platforms, platform_pack) = match cli.command {
        Command::Build {
            platforms,
            platform_pack,
            ..
        } => ("build", parse_platforms(&platforms).ok(), platform_pack),
        Command::Dev {
            device_id,
            platform_pack,
            ..
        } => (
            "dev",
            device_id.parse().ok().map(|platform| vec![platform]),
            platform_pack,
        ),
        _ => return command,
    };
    match platforms {
        Some(platforms) => command.mut_subcommand(subcommand, |subcommand| {
            subcommand.after_help(platforms_help(&platforms, platform_pack.as_deref()))
        }),
        None => command,
    }
}

fn platforms_help(platforms: &[Platform], platform_pack: Option<&Path>) -> String {
    platforms
        .iter()
        .map(|platform| {
            let pack = PlatformPack::load(*platform, platform_pack);
            let manifest = pack
                .as_ref()
                .map(|pack| &pack.manifest)
                .map_err(|error| format!("{error:#}"));
            settings::platform_help(*platform, manifest)
        })
        .collect::<Vec<_>>()
        .join("\n")
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

    use clap::Parser;

    use super::{Cli, Command, Platform, parse_platforms, split_arguments};

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
    fn accepts_top_level_application_options() {
        assert!(matches!(
            Cli::try_parse_from([
                "tok",
                "build",
                "ios",
                "--identifier",
                "com.example.app",
                "--version",
                "1.2.0",
                "--build",
                "pnpm build"
            ]),
            Ok(Cli {
                command: Command::Build { top, build_command, .. }
            }) if top.identifier.as_deref() == Some("com.example.app")
                && top.version.as_deref() == Some("1.2.0")
                && build_command.as_deref() == Some("pnpm build")
        ));
    }

    #[test]
    fn splits_platform_options_only_for_build_and_dev() -> anyhow::Result<()> {
        let arguments = |values: &[&str]| values.iter().map(OsString::from).collect::<Vec<_>>();
        let (remaining, _) = split_arguments(arguments(&[
            "tok",
            "build",
            "ios",
            "--ios-plist",
            "Info.plist",
        ]))?;
        assert_eq!(remaining, arguments(&["tok", "build", "ios"]));
        let (remaining, _) = split_arguments(arguments(&["tok", "targets", "--ios-plist", "x"]))?;
        assert_eq!(
            remaining,
            arguments(&["tok", "targets", "--ios-plist", "x"])
        );
        Ok(())
    }

    #[test]
    fn prints_the_version_with_a_subcommand() {
        assert!(matches!(
            Cli::try_parse_from(["tok", "version"]),
            Ok(Cli {
                command: Command::Version
            })
        ));
        assert!(Cli::try_parse_from(["tok", "--version"]).is_err());
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
