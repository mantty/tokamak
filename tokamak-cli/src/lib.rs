#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Shared tokamak CLI library types.

mod files;
mod platform_pack;

use std::process::ExitCode;

pub use files::{copy_dir_contents, copy_dir_contents_except, copy_file};
pub use platform_pack::{
    MANIFEST_FILE, PackVariable, Platform, PlatformPackError, PlatformPackManifest, PluginKeyKind,
    SHARED_PLATFORM_KEYS, Target, VariableKind, is_valid_key, load_manifest, script_command,
    write_manifest,
};

/// The exit code of a command-line tool whose run ended with `result`. A
/// failure is printed as `error: ` and the error with its causes on one line.
#[must_use]
pub fn exit_code(result: anyhow::Result<()>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
