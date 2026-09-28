#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Host-side Apple signing tool shipped in Apple platform packs.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};

use tokamak_apple_signing::{
    inventory, sign_ios_bundle, sign_macos_bundle, write_info_plist, write_simulator_entitlements,
};

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
    let mut arguments = std::env::args_os().skip(1);
    match arguments
        .next()
        .and_then(|value| value.into_string().ok())
        .as_deref()
    {
        Some("inventory") => {
            if arguments.next().is_some() {
                bail!("usage: tokamak-apple-signing inventory");
            }
            print!("{}", inventory()?);
            Ok(())
        }
        Some("plist") => plist(arguments),
        Some("sign") => sign(arguments),
        Some("simulator-entitlements") => simulator_entitlements(arguments),
        _ => bail!(
            "usage: tokamak-apple-signing inventory | plist --input INPUT --output OUTPUT [--icon-info-plist PATH] | sign --platform ios|macos --project PROJECT --bundle BUNDLE --bundle-id ID [--device-id DEVICE] | simulator-entitlements --identifier ID --output OUTPUT"
        ),
    }
}

fn plist(arguments: impl Iterator<Item = OsString>) -> Result<()> {
    let mut options = Options::parse(
        "plist",
        arguments,
        &["--input", "--output", "--icon-info-plist"],
    )?;
    let input = options.required("--input")?;
    let output = options.required("--output")?;
    let icon_info_plist = options.optional("--icon-info-plist").map(PathBuf::from);
    write_info_plist(
        Path::new(&input),
        Path::new(&output),
        icon_info_plist.as_deref(),
    )
}

fn sign(arguments: impl Iterator<Item = OsString>) -> Result<()> {
    let mut options = Options::parse(
        "signing",
        arguments,
        &[
            "--platform",
            "--project",
            "--bundle",
            "--bundle-id",
            "--device-id",
        ],
    )?;
    let platform = options.required("--platform")?;
    let project = PathBuf::from(options.required("--project")?);
    let bundle = PathBuf::from(options.required("--bundle")?);
    let bundle_id = options.required("--bundle-id")?;
    match (platform.as_str(), options.optional("--device-id")) {
        ("ios", device_id) => sign_ios_bundle(&project, &bundle, &bundle_id, device_id.as_deref()),
        ("macos", None) => sign_macos_bundle(&project, &bundle, &bundle_id),
        ("macos", Some(_)) => bail!("macOS signing does not take --device-id"),
        (platform, _) => bail!("unknown signing platform: {platform}"),
    }
}

fn simulator_entitlements(arguments: impl Iterator<Item = OsString>) -> Result<()> {
    let mut options = Options::parse(
        "simulator-entitlements",
        arguments,
        &["--identifier", "--output"],
    )?;
    let identifier = options.required("--identifier")?;
    let output = options.required("--output")?;
    write_simulator_entitlements(&identifier, Path::new(&output))
}

/// A subcommand's `--name value` options.
struct Options {
    command: &'static str,
    values: BTreeMap<String, String>,
}

impl Options {
    fn parse(
        command: &'static str,
        mut arguments: impl Iterator<Item = OsString>,
        names: &[&str],
    ) -> Result<Self> {
        let text = |value: OsString| {
            value
                .into_string()
                .map_err(|_| anyhow::anyhow!("{command} arguments must be valid UTF-8"))
        };
        let mut values = BTreeMap::new();
        while let Some(name) = arguments.next() {
            let name = text(name)?;
            if !names.contains(&name.as_str()) {
                bail!("unknown {command} option: {name}");
            }
            let value = arguments
                .next()
                .with_context(|| format!("a value is required for each {command} option"))?;
            values.insert(name, text(value)?);
        }
        Ok(Self { command, values })
    }

    fn required(&mut self, name: &str) -> Result<String> {
        self.optional(name)
            .with_context(|| format!("{} requires {name}", self.command))
    }

    fn optional(&mut self, name: &str) -> Option<String> {
        self.values.remove(name)
    }
}
