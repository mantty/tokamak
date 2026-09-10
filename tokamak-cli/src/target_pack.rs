#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Target-pack metadata and validation for `tokamak` runtime artifacts.

use std::fmt;
use std::fs;
use std::path::{Component, Path};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// Target-pack manifest filename.
pub const MANIFEST_FILE: &str = "target-pack.json";
/// Common runtime tool directory inside a target pack.
pub const RUNTIME_DIRECTORY: &str = "tools/runtime";
/// Common esbuild directory inside a target pack.
pub const ESBUILD_DIRECTORY: &str = "tools/runtime/node_modules/esbuild";
/// Common esbuild executable inside a target pack.
pub const ESBUILD_EXECUTABLE: &str = "tools/runtime/node_modules/esbuild/bin/esbuild";
/// Fixed path of the target-pack build entrypoint.
pub const BUILD_ENTRYPOINT: &str = "build/entrypoint";
/// Windows target-pack build entrypoint path, retaining PowerShell's
/// required script extension.
pub const WINDOWS_BUILD_ENTRYPOINT: &str = "build/entrypoint.ps1";

/// A platform family supported by the tokamak CLI.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Platform {
    /// Android devices and emulators.
    Android,
    /// Physical iOS devices.
    Ios,
    /// iOS Simulator.
    IosSimulator,
    /// macOS applications.
    Macos,
    /// Windows applications.
    Windows,
}

impl Platform {
    /// All supported platform families in stable display order.
    pub const ALL: &'static [Self] = &[
        Self::Android,
        Self::Ios,
        Self::IosSimulator,
        Self::Macos,
        Self::Windows,
    ];

    /// Platform family name used by the CLI and staging output.
    #[must_use]
    pub const fn directory_name(self) -> &'static str {
        match self {
            Self::Android => "android",
            Self::Ios => "ios",
            Self::IosSimulator => "ios-simulator",
            Self::Macos => "macos",
            Self::Windows => "windows",
        }
    }

    /// Repository directory containing the platform recipe and shell source.
    #[must_use]
    pub const fn repository_directory_name(self) -> &'static str {
        match self {
            Self::Android => "android",
            Self::Windows => "windows",
            Self::Ios | Self::IosSimulator | Self::Macos => "apple",
        }
    }

    /// Platform-owned target-pack recipe filename.
    #[must_use]
    pub const fn target_pack_recipe_file_name(self) -> &'static str {
        if matches!(self, Self::Windows) {
            "target-pack.ps1"
        } else {
            "target-pack"
        }
    }

    /// User-facing platform name.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Android => "Android",
            Self::Ios => "iOS",
            Self::IosSimulator => "iOS Simulator",
            Self::Macos => "macOS",
            Self::Windows => "Windows",
        }
    }

    /// Output filename for an application with the given slug.
    #[must_use]
    pub fn output_name(self, app_slug: &str) -> String {
        match self {
            Self::Android => format!("{app_slug}.apk"),
            Self::Windows => app_slug.to_owned(),
            Self::Ios | Self::IosSimulator | Self::Macos => format!("{app_slug}.app"),
        }
    }

    /// Whether frontend plugins may provide native code for this platform.
    #[must_use]
    pub const fn supports_plugins(self) -> bool {
        !matches!(self, Self::Windows)
    }

    /// Resolve the default runtime target for the current build host.
    ///
    /// # Errors
    ///
    /// Returns an error when the platform cannot be built on this host.
    pub fn default_target(self) -> Result<Target, TargetPackError> {
        match self {
            Self::Android => Ok(Target::AndroidArm64),
            Self::Ios => Ok(Target::IosArm64),
            Self::IosSimulator if cfg!(target_arch = "aarch64") => Ok(Target::IosSimulatorArm64),
            Self::IosSimulator if cfg!(target_arch = "x86_64") => Ok(Target::IosSimulatorX64),
            Self::IosSimulator => Err(TargetPackError::UnsupportedHost(
                "iOS Simulator builds require an Intel or Apple Silicon host",
            )),
            Self::Macos if cfg!(target_arch = "aarch64") => Ok(Target::MacosArm64),
            Self::Macos if cfg!(target_arch = "x86_64") => Ok(Target::MacosX64),
            Self::Macos => Err(TargetPackError::UnsupportedHost(
                "macOS builds require an Intel or Apple Silicon host",
            )),
            Self::Windows if cfg!(all(target_os = "windows", target_arch = "x86_64")) => {
                Ok(Target::WindowsX64)
            }
            Self::Windows => Err(TargetPackError::UnsupportedHost(
                "Windows builds require a 64-bit Windows host",
            )),
        }
    }

    /// Return whether the target belongs to this platform family.
    #[must_use]
    pub fn accepts(self, target: Target) -> bool {
        self == target.platform()
    }
}

impl FromStr for Platform {
    type Err = TargetPackError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|platform| platform.directory_name() == value)
            .ok_or_else(|| TargetPackError::UnknownPlatform(value.to_owned()))
    }
}

/// Supported `tokamak` runtime target triples.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Target {
    /// 64-bit ARM Android devices and emulators.
    AndroidArm64,
    /// Physical iOS devices.
    IosArm64,
    /// Apple Silicon iOS Simulator.
    IosSimulatorArm64,
    /// Intel iOS Simulator.
    IosSimulatorX64,
    /// Apple Silicon macOS.
    MacosArm64,
    /// Intel macOS.
    MacosX64,
    /// 64-bit Windows.
    WindowsX64,
}

impl Target {
    /// All supported targets in stable display order.
    pub const ALL: &'static [Self] = &[
        Self::AndroidArm64,
        Self::IosArm64,
        Self::IosSimulatorArm64,
        Self::IosSimulatorX64,
        Self::MacosArm64,
        Self::MacosX64,
        Self::WindowsX64,
    ];

    fn manifest_name(self) -> &'static str {
        match self {
            Self::AndroidArm64 => "android-arm64",
            Self::IosArm64 => "ios-arm64",
            Self::IosSimulatorArm64 => "ios-simulator-arm64",
            Self::IosSimulatorX64 => "ios-simulator-x64",
            Self::MacosArm64 => "macos-arm64",
            Self::MacosX64 => "macos-x64",
            Self::WindowsX64 => "windows-x64",
        }
    }

    /// Platform family for this runtime target.
    #[must_use]
    pub const fn platform(self) -> Platform {
        match self {
            Self::AndroidArm64 => Platform::Android,
            Self::IosArm64 => Platform::Ios,
            Self::IosSimulatorArm64 | Self::IosSimulatorX64 => Platform::IosSimulator,
            Self::MacosArm64 | Self::MacosX64 => Platform::Macos,
            Self::WindowsX64 => Platform::Windows,
        }
    }

    /// Rust target triple used to build this target.
    #[must_use]
    pub const fn rust_target(self) -> &'static str {
        match self {
            Self::AndroidArm64 => "aarch64-linux-android",
            Self::MacosArm64 => "aarch64-apple-darwin",
            Self::MacosX64 => "x86_64-apple-darwin",
            Self::IosArm64 => "aarch64-apple-ios",
            Self::IosSimulatorArm64 => "aarch64-apple-ios-sim",
            Self::IosSimulatorX64 => "x86_64-apple-ios",
            Self::WindowsX64 => "x86_64-pc-windows-msvc",
        }
    }

    /// Target-pack build entrypoint path.
    #[must_use]
    pub const fn build_entrypoint_path(self) -> &'static str {
        if matches!(self, Self::WindowsX64) {
            WINDOWS_BUILD_ENTRYPOINT
        } else {
            BUILD_ENTRYPOINT
        }
    }

    /// Runtime artifact kind produced for this target.
    #[must_use]
    pub const fn runtime_artifact_kind(self) -> ArtifactKind {
        if matches!(self, Self::WindowsX64) {
            ArtifactKind::RuntimeExecutable
        } else {
            ArtifactKind::RuntimeLibrary
        }
    }

    /// Runtime artifact path relative to the target-pack root.
    #[must_use]
    pub const fn runtime_artifact_path(self) -> &'static str {
        match self.platform() {
            Platform::Android => "bin/libtokamak.so",
            Platform::Windows => "bin/tokamak-shell-windows.exe",
            Platform::Ios | Platform::IosSimulator | Platform::Macos => {
                "frameworks/TokamakRuntime.framework"
            }
        }
    }

    /// Runtime artifact path relative to the application build input.
    #[must_use]
    pub const fn runtime_staging_path(self) -> &'static str {
        match self.platform() {
            Platform::Android => "runtime/libtokamak.so",
            Platform::Windows => "runtime/tokamak-shell-windows.exe",
            Platform::Ios | Platform::IosSimulator | Platform::Macos => {
                "runtime/frameworks/TokamakRuntime.framework"
            }
        }
    }

    /// Whether the target pack contains shell sources for the application
    /// build entrypoint to compile.
    #[must_use]
    pub const fn has_native_shell(self) -> bool {
        !matches!(self, Self::WindowsX64)
    }

    /// Host tools required to build an application from this target pack.
    #[must_use]
    pub const fn required_tools(self) -> &'static [&'static str] {
        match self.platform() {
            Platform::Android => &["gradle"],
            Platform::Windows => &[],
            Platform::Ios | Platform::IosSimulator | Platform::Macos => &["xcrun"],
        }
    }

    /// Artifact contract for this target pack.
    #[must_use]
    pub fn artifacts(self) -> Vec<Artifact> {
        let mut artifacts = vec![Artifact {
            kind: self.runtime_artifact_kind(),
            path: self.runtime_artifact_path().to_owned(),
        }];
        if self.has_native_shell() {
            artifacts.push(Artifact {
                kind: ArtifactKind::NativeShellDirectory,
                path: "native-shell".to_owned(),
            });
        }
        artifacts.extend([Artifact {
            kind: ArtifactKind::EsbuildExecutable,
            path: ESBUILD_EXECUTABLE.to_owned(),
        }]);
        artifacts
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.manifest_name())
    }
}

impl Serialize for Target {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Target {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(serde::de::Error::custom)
    }
}

impl FromStr for Target {
    type Err = TargetPackError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|target| target.manifest_name() == value)
            .ok_or_else(|| TargetPackError::UnknownTarget(value.to_owned()))
    }
}

/// Artifact categories that a target pack can expose to the CLI.
///
/// Unknown artifact kinds intentionally fail deserialization. Additive artifact
/// kinds require a new CLI/target-pack release before older CLIs can consume
/// them.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ArtifactKind {
    /// Precompiled runtime library or framework.
    RuntimeLibrary,
    /// Precompiled native application-shell executable.
    RuntimeExecutable,
    /// Native application-shell sources compiled during an app build.
    NativeShellDirectory,
    /// Host-side JavaScript compiler used to produce Worker bytecode.
    EsbuildExecutable,
}

/// A single file or directory provided by a target pack.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    /// The artifact role.
    pub kind: ArtifactKind,
    /// Path relative to the target-pack root.
    pub path: String,
}

/// Manifest included in each `tokamak` target pack.
///
/// In addition to the paths listed here, every pack contains the fixed
/// `build/entrypoint` builder entrypoint (or `build/entrypoint.ps1` for
/// Windows). The CLI invokes it with `build`, an input directory, and an
/// output path; the entrypoint owns the platform-specific project and signing
/// work.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetPackManifest {
    /// `tokamak` release version that produced the target pack.
    pub tokamak_version: String,
    /// Native platform target.
    pub target: Target,
    /// Runtime artifacts available in this pack.
    pub artifacts: Vec<Artifact>,
    /// Host tools required to consume this pack locally.
    pub required_tools: Vec<String>,
}

impl TargetPackManifest {
    /// Validate the manifest contract before a CLI consumes the target pack.
    ///
    /// # Errors
    ///
    /// Returns an error when required fields are missing or an artifact path
    /// escapes the pack root.
    pub fn validate(&self) -> Result<(), TargetPackError> {
        if self.tokamak_version.trim().is_empty() {
            return Err(TargetPackError::MissingVersion);
        }

        if self.artifacts.is_empty() {
            return Err(TargetPackError::MissingArtifacts);
        }

        for artifact in &self.artifacts {
            validate_relative_path(&artifact.path)?;
        }

        Ok(())
    }

    /// Validate that a CLI version can consume this target pack.
    ///
    /// # Errors
    ///
    /// Returns an error when the CLI version does not equal the tokamak version
    /// recorded in the manifest.
    pub fn validate_cli_version(&self, cli_version: &str) -> Result<(), TargetPackError> {
        if self.tokamak_version == cli_version {
            Ok(())
        } else {
            Err(TargetPackError::IncompatibleTokamakVersion {
                required: self.tokamak_version.clone(),
                actual: cli_version.to_owned(),
            })
        }
    }
}

/// Load a target-pack manifest from JSON.
///
/// # Errors
///
/// Returns an error when the file cannot be read, the JSON cannot be parsed,
/// or the decoded manifest violates the target-pack contract.
pub fn load_manifest(path: impl AsRef<Path>) -> Result<TargetPackManifest, TargetPackError> {
    let path = path.as_ref();
    let content = fs::read_to_string(path)?;
    let manifest = serde_json::from_str(&content)?;
    TargetPackManifest::validate(&manifest)?;
    Ok(manifest)
}

/// Write a target-pack manifest as pretty JSON.
///
/// # Errors
///
/// Returns an error when the manifest is invalid, the JSON cannot be
/// serialized, or the destination file cannot be written.
pub fn write_manifest(
    path: impl AsRef<Path>,
    manifest: &TargetPackManifest,
) -> Result<(), TargetPackError> {
    manifest.validate()?;
    let content = serde_json::to_string_pretty(manifest)?;
    fs::write(path.as_ref(), content)?;
    Ok(())
}

/// Target-pack parsing and validation failures.
#[derive(Debug, Error)]
pub enum TargetPackError {
    /// Unknown target name.
    #[error("unknown target '{0}'")]
    UnknownTarget(String),
    /// Unknown platform name.
    #[error("unknown platform '{0}'")]
    UnknownPlatform(String),
    /// The requested platform cannot be built on this host.
    #[error("{0}")]
    UnsupportedHost(&'static str),
    /// `tokamak` version was empty.
    #[error("tokamakVersion must not be empty")]
    MissingVersion,
    /// The target pack requires a different tokamak/CLI version.
    #[error("target pack was built for tokamak {required}, but this CLI is {actual}")]
    IncompatibleTokamakVersion {
        /// tokamak version recorded in the target pack.
        required: String,
        /// CLI version that attempted to consume the target pack.
        actual: String,
    },
    /// Manifest had no artifacts.
    #[error("target pack must contain at least one artifact")]
    MissingArtifacts,
    /// Artifact path was empty.
    #[error("artifact path must not be empty")]
    EmptyArtifactPath,
    /// Artifact path was absolute or escaped the pack root.
    #[error("artifact path must stay inside the target pack: {0}")]
    UnsafeArtifactPath(String),
    /// File IO failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// JSON parsing or serialization failed.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

fn validate_relative_path(path: &str) -> Result<(), TargetPackError> {
    if path.is_empty() {
        return Err(TargetPackError::EmptyArtifactPath);
    }

    if path.contains('\\') || has_windows_drive_prefix(path) {
        return Err(TargetPackError::UnsafeArtifactPath(path.to_owned()));
    }

    for component in Path::new(path).components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(TargetPackError::UnsafeArtifactPath(path.to_owned()));
            }
        }
    }

    Ok(())
}

fn has_windows_drive_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}
