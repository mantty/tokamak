#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Shared tokamak CLI library types.

mod platform_pack;

pub use platform_pack::{
    Artifact, ArtifactKind, MANIFEST_FILE, PackVariable, Platform, PlatformPackError,
    PlatformPackManifest, Target, VariableKind, load_manifest, write_manifest,
};
