//! Developer signing-asset inventory, which the iOS platform pack lists.

use std::collections::BTreeMap;
use std::ffi::OsStr;

use anyhow::Result;
use tokamak_cli::Platform;

use super::{pipeline, support};

pub(crate) fn list() -> Result<()> {
    let (pack_root, manifest) = pipeline::load_platform_pack(Platform::Ios, None)?;
    support::run_entrypoint(
        &pack_root,
        manifest.target,
        &[OsStr::new("certs")],
        &BTreeMap::new(),
    )
}
