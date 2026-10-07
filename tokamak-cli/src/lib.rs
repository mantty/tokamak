#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Shared tokamak CLI library types.

mod files;
mod platform_pack;

pub use files::{copy_dir_contents, copy_file};
pub use platform_pack::{
    MANIFEST_FILE, PackVariable, Platform, PlatformPackError, PlatformPackManifest, PluginKeyKind,
    SHARED_PLATFORM_KEYS, Target, VariableKind, is_valid_key, load_manifest, script_command,
    write_manifest,
};
