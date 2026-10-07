//! Developer signing-asset inventory, which the iOS platform pack lists.

use std::collections::BTreeMap;
use std::ffi::OsStr;

use anyhow::Result;
use tokamak_cli::Platform;

use super::packs::PlatformPack;

pub(crate) fn list() -> Result<()> {
    PlatformPack::load(Platform::Ios, None)?.run(&[OsStr::new("certs")], &BTreeMap::new())
}
