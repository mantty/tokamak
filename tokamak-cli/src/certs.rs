//! Developer signing-asset inventory.

use anyhow::Result;

pub(crate) fn list() -> Result<()> {
    print!("{}", tokamak_apple_signing::inventory()?);
    Ok(())
}
