#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Platform-pack metadata and validation for `tokamak` runtime artifacts.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// Platform-pack manifest filename.
pub const MANIFEST_FILE: &str = "platform-pack.json";
/// Fixed path of the platform-pack build entrypoint.
pub const BUILD_ENTRYPOINT: &str = "build/entrypoint";
/// Windows platform-pack build entrypoint path, retaining PowerShell's
/// required script extension.
pub const WINDOWS_BUILD_ENTRYPOINT: &str = "build/entrypoint.ps1";
/// Keys a platform object shares with the top level; tokamak validates them.
pub const SHARED_PLATFORM_KEYS: [&str; 3] = ["name", "identifier", "icon"];

/// Whether `key` is lowercase ASCII words joined by single hyphens, the form of
/// every configuration key, command-line option, and platform-pack variable.
#[must_use]
pub fn is_valid_key(key: &str) -> bool {
    key.starts_with(|character: char| character.is_ascii_lowercase())
        && key.split('-').all(|word| {
            !word.is_empty()
                && word
                    .chars()
                    .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
        })
}

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

    /// Prefix of the platform's options, environment variables, and
    /// configuration object; iOS devices and simulators share `ios`.
    #[must_use]
    pub const fn namespace(self) -> &'static str {
        match self {
            Self::Android => "android",
            Self::Ios | Self::IosSimulator => "ios",
            Self::Macos => "macos",
            Self::Windows => "windows",
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

    /// Resolve the default runtime target for the current build host.
    ///
    /// # Errors
    ///
    /// Returns an error when the platform cannot be built on this host.
    pub fn default_target(self) -> Result<Target, PlatformPackError> {
        match self {
            Self::Android => Ok(Target::AndroidArm64),
            Self::Ios => Ok(Target::IosArm64),
            Self::IosSimulator if cfg!(target_arch = "aarch64") => Ok(Target::IosSimulatorArm64),
            Self::IosSimulator if cfg!(target_arch = "x86_64") => Ok(Target::IosSimulatorX64),
            Self::IosSimulator => Err(PlatformPackError::UnsupportedHost(
                "iOS Simulator builds require an Intel or Apple Silicon host",
            )),
            Self::Macos if cfg!(target_arch = "aarch64") => Ok(Target::MacosArm64),
            Self::Macos if cfg!(target_arch = "x86_64") => Ok(Target::MacosX64),
            Self::Macos => Err(PlatformPackError::UnsupportedHost(
                "macOS builds require an Intel or Apple Silicon host",
            )),
            Self::Windows if cfg!(all(target_os = "windows", target_arch = "x86_64")) => {
                Ok(Target::WindowsX64)
            }
            Self::Windows => Err(PlatformPackError::UnsupportedHost(
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
    type Err = PlatformPackError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|platform| platform.directory_name() == value)
            .ok_or_else(|| PlatformPackError::UnknownPlatform(value.to_owned()))
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

    /// Platform-pack build entrypoint path.
    #[must_use]
    pub const fn build_entrypoint_path(self) -> &'static str {
        if matches!(self, Self::WindowsX64) {
            WINDOWS_BUILD_ENTRYPOINT
        } else {
            BUILD_ENTRYPOINT
        }
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
    type Err = PlatformPackError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|target| target.manifest_name() == value)
            .ok_or_else(|| PlatformPackError::UnknownTarget(value.to_owned()))
    }
}

/// Manifest included in each `tokamak` platform pack.
///
/// Every pack also contains the fixed `build/entrypoint` (or
/// `build/entrypoint.ps1` for Windows), which the CLI runs from the pack root.
/// `build INPUT OUTPUT` builds the app; an iOS pack's `certs` lists the local
/// signing assets. The entrypoint receives the user's environment plus
/// `TOKAMAK_<PLATFORM>_<KEY>` for each variable and for the app's `icon` path,
/// and owns every platform-specific project, signing, and packaging step.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformPackManifest {
    /// `tokamak` release version that produced the platform pack.
    pub tokamak_version: String,
    /// Native platform target.
    pub target: Target,
    /// Variables the pack accepts, by name.
    pub variables: BTreeMap<String, PackVariable>,
    /// Keys the pack reads from a plugin's platform section, by name.
    pub plugin_keys: BTreeMap<String, PluginKeyKind>,
}

/// A variable a platform pack accepts, set as `--<platform>-<name>`,
/// `TOKAMAK_<PLATFORM>_<NAME>`, or `<platform>.<name>` in the Tokamak configuration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackVariable {
    /// How the CLI passes the value.
    pub kind: VariableKind,
    /// One-line description shown by `--help`.
    pub description: String,
}

/// How the CLI passes a variable's value to the platform pack.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum VariableKind {
    /// The value is passed unchanged.
    String,
    /// A relative path is made absolute against the current directory, or the
    /// Vite root when the `tokamak()` options set it.
    Path,
}

/// The value a plugin key takes, and how the CLI stages it in the plugin's
/// directory of the build input.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PluginKeyKind {
    /// A string, staged as a file holding it.
    String,
    /// Strings, staged as a directory of files holding each, named by index.
    Strings,
    /// A file in the plugin's package, staged as its copy.
    Path,
    /// Files in the plugin's package, staged as a directory of their copies,
    /// named `<index>-<file name>`.
    Paths,
}

impl PlatformPackManifest {
    /// Validate the manifest contract before a CLI consumes the platform pack.
    ///
    /// # Errors
    ///
    /// Returns an error when the version is missing, or a variable or plugin
    /// key is invalid.
    pub fn validate(&self) -> Result<(), PlatformPackError> {
        if self.tokamak_version.trim().is_empty() {
            return Err(PlatformPackError::MissingVersion);
        }

        for (name, variable) in &self.variables {
            validate_variable(name, variable)?;
        }

        if let Some(key) = self.plugin_keys.keys().find(|key| !is_valid_key(key)) {
            return Err(PlatformPackError::InvalidPluginKey(key.clone()));
        }

        Ok(())
    }

    /// Validate that a CLI version can consume this platform pack.
    ///
    /// # Errors
    ///
    /// Returns an error when the CLI version does not equal the tokamak version
    /// recorded in the manifest.
    pub fn validate_cli_version(&self, cli_version: &str) -> Result<(), PlatformPackError> {
        if self.tokamak_version == cli_version {
            Ok(())
        } else {
            Err(PlatformPackError::IncompatibleTokamakVersion {
                required: self.tokamak_version.clone(),
                actual: cli_version.to_owned(),
            })
        }
    }
}

/// Load a platform-pack manifest from JSON.
///
/// # Errors
///
/// Returns an error when the file cannot be read, the JSON cannot be parsed,
/// or the decoded manifest violates the platform-pack contract.
pub fn load_manifest(path: impl AsRef<Path>) -> Result<PlatformPackManifest, PlatformPackError> {
    let path = path.as_ref();
    let content = fs::read_to_string(path)?;
    let manifest = serde_json::from_str(&content)?;
    PlatformPackManifest::validate(&manifest)?;
    Ok(manifest)
}

/// Write a platform-pack manifest as pretty JSON.
///
/// # Errors
///
/// Returns an error when the manifest is invalid, the JSON cannot be
/// serialized, or the destination file cannot be written.
pub fn write_manifest(
    path: impl AsRef<Path>,
    manifest: &PlatformPackManifest,
) -> Result<(), PlatformPackError> {
    manifest.validate()?;
    let content = serde_json::to_string_pretty(manifest)?;
    fs::write(path.as_ref(), content)?;
    Ok(())
}

/// A command that runs the platform-pack script `script`: PowerShell runs a
/// `.ps1` script on Windows, and Bash runs any other.
#[must_use]
pub fn script_command(script: &Path) -> Command {
    let powershell = cfg!(windows)
        && script
            .extension()
            .is_some_and(|extension| extension == "ps1");
    let mut command = if powershell {
        let mut command = Command::new("powershell");
        command.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]);
        command
    } else {
        Command::new("bash")
    };
    command.arg(script);
    command
}

/// Platform-pack parsing and validation failures.
#[derive(Debug, Error)]
pub enum PlatformPackError {
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
    /// The platform pack requires a different tokamak/CLI version.
    #[error("platform pack was built for tokamak {required}, but this CLI is {actual}")]
    IncompatibleTokamakVersion {
        /// tokamak version recorded in the platform pack.
        required: String,
        /// CLI version that attempted to consume the platform pack.
        actual: String,
    },
    /// Variable name was not lowercase words joined by hyphens.
    #[error("variable name must be lowercase words joined by hyphens: {0}")]
    InvalidVariableName(String),
    /// Variable name belongs to a key the CLI validates itself.
    #[error("variable name is reserved for the CLI: {0}")]
    ReservedVariableName(String),
    /// Variable had no description.
    #[error("variable must have a description: {0}")]
    MissingVariableDescription(String),
    /// Plugin key was not lowercase words joined by hyphens.
    #[error("plugin key must be lowercase words joined by hyphens: {0}")]
    InvalidPluginKey(String),
    /// File IO failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// JSON parsing or serialization failed.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

fn validate_variable(name: &str, variable: &PackVariable) -> Result<(), PlatformPackError> {
    if !is_valid_key(name) {
        return Err(PlatformPackError::InvalidVariableName(name.to_owned()));
    }
    if SHARED_PLATFORM_KEYS.contains(&name) {
        return Err(PlatformPackError::ReservedVariableName(name.to_owned()));
    }
    if variable.description.trim().is_empty() {
        return Err(PlatformPackError::MissingVariableDescription(
            name.to_owned(),
        ));
    }
    Ok(())
}
