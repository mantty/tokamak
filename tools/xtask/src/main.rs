#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Maintainer-only tokamak build tasks.

mod builder;
mod layout;
mod support;

use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tokamak_cli::Target;

#[derive(Debug, Parser)]
#[command(name = "xtask", about = "tokamak maintainer tasks")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Build a platform pack from the current tokamak workspace.
    PlatformPack {
        /// Runtime target to build.
        #[arg(long)]
        target: Target,
    },
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
        Command::PlatformPack { target } => {
            let manifest = builder::build_source_platform_pack(target)?;
            println!("Built platform pack: {}", manifest.display());
        }
    }
    Ok(())
}
