#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Shared tokamak CLI library types.

mod platform_pack;

pub use platform_pack::{
    MANIFEST_FILE, PackVariable, Platform, PlatformPackError, PlatformPackManifest, PluginKeyKind,
    SHARED_PLATFORM_KEYS, Target, VariableKind, is_valid_key, load_manifest, write_manifest,
};
