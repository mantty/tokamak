#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Shared tokamak CLI library types.

mod platform_pack;

pub use platform_pack::{
    Artifact, ArtifactKind, ESBUILD_DIRECTORY, ESBUILD_EXECUTABLE, MANIFEST_FILE, Platform,
    PlatformPackError, PlatformPackManifest, RUNTIME_DIRECTORY, Target, load_manifest,
    write_manifest,
};
