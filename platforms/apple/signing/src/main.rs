#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Host-side Apple signing tool shipped in Apple platform packs.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};

use tokamak_apple_signing::{inventory, sign_ios_bundle, write_info_plist};

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
        _ => bail!(
            "usage: tokamak-apple-signing inventory | plist --input INPUT --output OUTPUT [--icon-info-plist PATH] | sign --project PROJECT --bundle BUNDLE --bundle-id ID [--device-id DEVICE]"
        ),
    }
}

fn plist(mut arguments: impl Iterator<Item = OsString>) -> Result<()> {
    let mut input = None;
    let mut output = None;
    let mut icon_info_plist = None;
    while let Some(argument) = arguments.next() {
        let argument = argument
            .into_string()
            .map_err(|_| anyhow::anyhow!("plist arguments must be valid UTF-8"))?;
        let value = arguments
            .next()
            .context("a value is required for each plist option")?
            .into_string()
            .map_err(|_| anyhow::anyhow!("plist arguments must be valid UTF-8"))?;
        match argument.as_str() {
            "--input" => input = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            "--icon-info-plist" => icon_info_plist = Some(PathBuf::from(value)),
            _ => bail!("unknown plist option: {argument}"),
        }
    }
    let input = input.context("plist requires --input")?;
    let output = output.context("plist requires --output")?;
    write_info_plist(&input, &output, icon_info_plist.as_deref())
}

fn sign(mut arguments: impl Iterator<Item = OsString>) -> Result<()> {
    let mut project = None;
    let mut bundle = None;
    let mut bundle_id = None;
    let mut device_id = None;

    while let Some(argument) = arguments.next() {
        let argument = argument
            .into_string()
            .map_err(|_| anyhow::anyhow!("signing arguments must be valid UTF-8"))?;
        let value = arguments
            .next()
            .context("a value is required for each signing option")?
            .into_string()
            .map_err(|_| anyhow::anyhow!("signing arguments must be valid UTF-8"))?;
        match argument.as_str() {
            "--project" => project = Some(PathBuf::from(value)),
            "--bundle" => bundle = Some(PathBuf::from(value)),
            "--bundle-id" => bundle_id = Some(value),
            "--device-id" => device_id = Some(value),
            _ => bail!("unknown signing option: {argument}"),
        }
    }

    let project = project.context("signing requires --project")?;
    let bundle = bundle.context("signing requires --bundle")?;
    let bundle_id = bundle_id.context("signing requires --bundle-id")?;
    sign_ios_bundle(&project, &bundle, &bundle_id, device_id.as_deref())
}
