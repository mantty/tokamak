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
mod plugin_api;
mod plugin_settings;
mod plugins;
mod project;
mod scaffold;
mod settings;
mod storage;
mod tokamak_config;
mod vite;
mod worker;
mod worker_graph;
mod wrangler_config;

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use packs::PlatformPack;
use settings::{NamespacedOptions, TopOptions};
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
    /// Create, generate and check plugin packages.
    Plugin {
        #[command(subcommand)]
        command: PluginCommand,
    },
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

#[derive(Debug, Subcommand)]
enum PluginCommand {
    /// Create a publishable plugin package.
    New {
        /// New directory, named with lowercase words joined by hyphens.
        directory: PathBuf,
        /// Comma-separated platforms a native implementation runs on: ios, macos or android.
        /// Without them, the plugin composes APIs in TypeScript.
        #[arg(long, value_delimiter = ',')]
        platforms: Vec<Platform>,
    },
    /// Write a platform family's TypeScript API and native dispatch, in the plugin package in
    /// the current directory, from the family's registration file.
    Generate {
        /// The platform family: apple (iOS and macOS) or android.
        #[arg(value_enum)]
        family: plugin_api::Family,
    },
    /// Fail when the generated files in the plugin package in the current directory differ from
    /// what this host's toolchains generate.
    Check,
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
    let (arguments, platform_options, plugins) = split_arguments(env::args_os().collect())?;
    let matches = command(&arguments, &plugins).get_matches_from(arguments);
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|error| error.exit());
    let namespaced_options = match matches.subcommand() {
        Some((_, matches)) => platform_options.with_plugin_options(matches, &plugins),
        None => platform_options,
    };

    match cli.command {
        Command::Plugin {
            command:
                PluginCommand::New {
                    directory,
                    platforms,
                },
        } => scaffold::create(&directory, &platforms),
        Command::Plugin {
            command: PluginCommand::Generate { family },
        } => plugin_api::generate(&env::current_dir()?, family),
        Command::Plugin {
            command: PluginCommand::Check,
        } => plugin_api::check(&env::current_dir()?),
        Command::Build {
            platforms,
            project,
            build_dir,
            platform_pack,
            top,
            build_command,
            skip_project_build,
        } => build(&pipeline::BuildRequest {
            platforms: parse_platforms(&platforms)?,
            project_dir: project,
            build_dir,
            platform_pack_dir: platform_pack,
            top,
            namespaced_options,
            plugins,
            build_command,
            skip_project_build,
        }),
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
            namespaced_options,
            plugins,
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

/// Build the bundles `request` asks for and say where each one is.
fn build(request: &pipeline::BuildRequest) -> Result<()> {
    for summary in pipeline::run(request)? {
        println!(
            "Built {} bundle: {}",
            summary.platform.display_name(),
            paths::command_path(&summary.bundle_dir).display()
        );
    }
    Ok(())
}

/// Split platform options from the arguments of the commands that take them,
/// with the plugins of the project the arguments name.
fn split_arguments(
    arguments: Vec<OsString>,
) -> Result<(Vec<OsString>, NamespacedOptions, Vec<plugins::Plugin>)> {
    let takes_platform_options = arguments
        .get(1)
        .is_some_and(|command| command == "build" || command == "dev");
    if !takes_platform_options {
        return Ok((arguments, NamespacedOptions::default(), Vec::new()));
    }
    // A missing project fails later with its own message.
    let project = project_argument(&arguments);
    let plugins = if project.is_dir() {
        plugins::discover(&project)?
    } else {
        Vec::new()
    };
    let (arguments, options) = settings::split_namespaced_options(arguments)?;
    Ok((arguments, options, plugins))
}

/// The `--project` argument's directory, or the current directory.
fn project_argument(arguments: &[OsString]) -> PathBuf {
    let mut arguments = arguments.iter().take_while(|argument| *argument != "--");
    while let Some(argument) = arguments.next() {
        if argument == "--project" {
            return arguments
                .next()
                .map_or_else(|| PathBuf::from("."), PathBuf::from);
        }
        if let Some(project) = argument
            .to_str()
            .and_then(|argument| argument.strip_prefix("--project="))
        {
            return PathBuf::from(project);
        }
    }
    PathBuf::from(".")
}

/// The argument parser, with the plugins' options; `--help` for named
/// platforms lists their options.
fn command(arguments: &[OsString], plugins: &[plugins::Plugin]) -> clap::Command {
    let plugin_arguments = settings::plugin_arguments(plugins);
    let command = Cli::command()
        .mut_subcommand("build", |build| build.args(plugin_arguments.clone()))
        .mut_subcommand("dev", |dev| dev.args(plugin_arguments));
    let is_help = |argument: &&OsString| *argument == "--help" || *argument == "-h";
    let asks_for_help = arguments
        .iter()
        .take_while(|argument| *argument != "--")
        .any(|argument| is_help(&argument));
    if !asks_for_help {
        return command;
    }
    let without_help = arguments.iter().filter(|argument| !is_help(argument));
    let Ok(matches) = command.clone().try_get_matches_from(without_help) else {
        return command;
    };
    let Ok(cli) = Cli::from_arg_matches(&matches) else {
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
    let Some(platforms) = platforms else {
        return command;
    };
    let mut help = platforms_help(&platforms, platform_pack.as_deref());
    if let Some(plugins_help) = settings::plugins_help(plugins) {
        help = format!("{help}\n{plugins_help}");
    }
    command.mut_subcommand(subcommand, |subcommand| subcommand.after_help(help))
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
    use std::path::PathBuf;

    use clap::{CommandFactory, Parser};

    use super::{
        Cli, Command, Platform, command, parse_platforms, plugins, project_argument,
        split_arguments,
    };

    #[test]
    fn parses_comma_separated_platforms() {
        assert_eq!(
            parse_platforms("macos,android").ok(),
            Some(vec![Platform::Macos, Platform::Android])
        );
    }

    #[test]
    fn accepts_one_comma_separated_platform_argument() {
        let Ok(Cli {
            command: Command::Build { platforms, .. },
        }) = Cli::try_parse_from(["tok", "build", "macos,android"])
        else {
            panic!("`tok build macos,android` did not parse as a build command");
        };
        assert_eq!(platforms, "macos,android");
    }

    #[test]
    fn accepts_top_level_application_options() {
        let Ok(Cli {
            command: Command::Build {
                top, build_command, ..
            },
        }) = Cli::try_parse_from([
            "tok",
            "build",
            "ios",
            "--identifier",
            "com.example.app",
            "--version",
            "1.2.0",
            "--build",
            "pnpm build",
        ])
        else {
            panic!("`tok build ios` with application options did not parse as a build command");
        };
        assert_eq!(
            (
                top.identifier.as_deref(),
                top.version.as_deref(),
                build_command.as_deref()
            ),
            (Some("com.example.app"), Some("1.2.0"), Some("pnpm build"))
        );
    }

    #[test]
    fn splits_platform_options_only_for_build_and_dev() -> anyhow::Result<()> {
        let arguments = |values: &[&str]| values.iter().map(OsString::from).collect::<Vec<_>>();
        let (remaining, _, _) = split_arguments(arguments(&[
            "tok",
            "build",
            "ios",
            "--ios-plist",
            "Info.plist",
        ]))?;
        assert_eq!(remaining, arguments(&["tok", "build", "ios"]));
        let (remaining, _, _) =
            split_arguments(arguments(&["tok", "targets", "--ios-plist", "x"]))?;
        assert_eq!(
            remaining,
            arguments(&["tok", "targets", "--ios-plist", "x"])
        );
        Ok(())
    }

    #[test]
    fn finds_the_project_argument_before_the_command_separator() {
        let arguments = |values: &[&str]| values.iter().map(OsString::from).collect::<Vec<_>>();
        for (values, project) in [
            (&["tok", "build", "ios", "--project", "app"][..], "app"),
            (&["tok", "build", "ios", "--project=app"][..], "app"),
            (&["tok", "build", "ios"][..], "."),
            (
                &["tok", "dev", "x", "--", "vite", "--project", "other"][..],
                ".",
            ),
        ] {
            assert_eq!(project_argument(&arguments(values)), PathBuf::from(project));
        }
    }

    #[test]
    fn parses_the_options_of_the_projects_plugins() -> anyhow::Result<()> {
        let project = tempfile::tempdir()?;
        std::fs::write(
            project.path().join("package.json"),
            r#"{"dependencies":{"alerts":"1.0.0"}}"#,
        )?;
        let plugin = project.path().join("node_modules/alerts");
        std::fs::create_dir_all(&plugin)?;
        std::fs::write(
            plugin.join("tokamak-plugin.json"),
            serde_json::json!({
                "schemaVersion": 1,
                "id": "alerts",
                "settings": { "loud": { "kind": "boolean", "description": "Loud" } },
                "build": "build.mjs",
            })
            .to_string(),
        )?;
        let mut arguments = vec![OsString::from("tok"), "build".into(), "ios".into()];
        arguments.extend(["--project".into(), project.path().as_os_str().to_owned()]);
        arguments.extend(["--alerts-ios-loud".into(), "--ios-plist=Info.plist".into()]);

        let (remaining, _, plugins) = split_arguments(arguments.clone())?;
        let matches = command(&remaining, &plugins).try_get_matches_from(&remaining)?;

        assert_eq!(
            (remaining.as_slice(), plugins.len()),
            (&arguments[..6], 1),
            "(arguments left for tok, plugins discovered)"
        );
        let Some(("build", build)) = matches.subcommand() else {
            panic!("the arguments did not parse as `tok build`");
        };
        assert_eq!(
            build.get_one::<serde_json::Value>("alerts-ios-loud"),
            Some(&serde_json::Value::Bool(true))
        );
        Ok(())
    }

    #[test]
    fn reserves_the_first_word_of_each_of_tok_s_own_options() {
        let command = Cli::command();
        let options = ["build", "dev"].into_iter().flat_map(|subcommand| {
            command
                .find_subcommand(subcommand)
                .into_iter()
                .flat_map(clap::Command::get_arguments)
                .filter_map(clap::Arg::get_long)
                .map(move |option| (subcommand, option))
        });
        for (subcommand, option) in options {
            let word = option.split('-').next().unwrap_or(option);
            assert!(
                plugins::RESERVED_WORDS.contains(&word),
                "--{option} of tok {subcommand} starts with an unreserved word"
            );
        }
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
        let Ok(Cli {
            command: Command::Dev {
                device_id, command, ..
            },
        }) = Cli::try_parse_from(["tok", "dev", "ios", "--", "astro", "dev"])
        else {
            panic!("`tok dev ios -- astro dev` did not parse as a dev command");
        };
        assert_eq!(
            (device_id.as_str(), command),
            ("ios", vec![OsString::from("astro"), OsString::from("dev")])
        );
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
