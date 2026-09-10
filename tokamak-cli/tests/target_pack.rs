use std::fs;
use std::str::FromStr;

use tokamak_cli::{
    Artifact, ArtifactKind, Platform, Target, TargetPackError, TargetPackManifest, load_manifest,
    write_manifest,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type TargetMetadata = (
    Target,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    bool,
    &'static str,
);

const TARGET_METADATA: &[TargetMetadata] = &[
    (
        Target::AndroidArm64,
        "aarch64-linux-android",
        "android",
        "target-pack",
        "bin/libtokamak.so",
        "runtime/libtokamak.so",
        true,
        "build/entrypoint",
    ),
    (
        Target::IosArm64,
        "aarch64-apple-ios",
        "apple",
        "target-pack",
        "frameworks/TokamakRuntime.framework",
        "runtime/frameworks/TokamakRuntime.framework",
        true,
        "build/entrypoint",
    ),
    (
        Target::IosSimulatorArm64,
        "aarch64-apple-ios-sim",
        "apple",
        "target-pack",
        "frameworks/TokamakRuntime.framework",
        "runtime/frameworks/TokamakRuntime.framework",
        true,
        "build/entrypoint",
    ),
    (
        Target::IosSimulatorX64,
        "x86_64-apple-ios",
        "apple",
        "target-pack",
        "frameworks/TokamakRuntime.framework",
        "runtime/frameworks/TokamakRuntime.framework",
        true,
        "build/entrypoint",
    ),
    (
        Target::MacosArm64,
        "aarch64-apple-darwin",
        "apple",
        "target-pack",
        "frameworks/TokamakRuntime.framework",
        "runtime/frameworks/TokamakRuntime.framework",
        true,
        "build/entrypoint",
    ),
    (
        Target::MacosX64,
        "x86_64-apple-darwin",
        "apple",
        "target-pack",
        "frameworks/TokamakRuntime.framework",
        "runtime/frameworks/TokamakRuntime.framework",
        true,
        "build/entrypoint",
    ),
    (
        Target::WindowsX64,
        "x86_64-pc-windows-msvc",
        "windows",
        "target-pack.ps1",
        "bin/tokamak-shell-windows.exe",
        "runtime/tokamak-shell-windows.exe",
        false,
        "build/entrypoint.ps1",
    ),
];

fn valid_manifest() -> TargetPackManifest {
    TargetPackManifest {
        tokamak_version: "0.1.0".to_owned(),
        target: Target::IosArm64,
        artifacts: vec![Artifact {
            kind: ArtifactKind::RuntimeLibrary,
            path: "frameworks/TokamakRuntime.framework".to_owned(),
        }],
        required_tools: vec!["xcode".to_owned()],
    }
}

fn assert_unsafe_artifact_path(path: &str) {
    let mut manifest = valid_manifest();
    path.clone_into(&mut manifest.artifacts[0].path);

    assert!(matches!(
        manifest.validate(),
        Err(TargetPackError::UnsafeArtifactPath(rejected)) if rejected == path
    ));
}

#[test]
fn parses_known_targets_and_rejects_unknown_targets() {
    let parsed = Target::from_str("ios-arm64").map_err(|error| error.to_string());
    assert_eq!(parsed, Ok(Target::IosArm64));
    let simulator = Target::from_str("ios-simulator-arm64").map_err(|error| error.to_string());
    assert_eq!(simulator, Ok(Target::IosSimulatorArm64));
    assert_eq!(Target::IosSimulatorX64.to_string(), "ios-simulator-x64");
    assert_eq!(Target::MacosArm64.to_string(), "macos-arm64");
    assert_eq!(Target::MacosX64.to_string(), "macos-x64");
    assert_eq!(Target::WindowsX64.to_string(), "windows-x64");
    assert!(matches!(
        Target::from_str("ios-armv7"),
        Err(TargetPackError::UnknownTarget(target)) if target == "ios-armv7"
    ));
}

#[test]
fn maps_targets_to_one_platform_metadata_model() {
    assert_eq!(Target::AndroidArm64.platform(), Platform::Android);
    assert_eq!(Target::IosArm64.platform(), Platform::Ios);
    assert_eq!(Target::IosSimulatorArm64.platform(), Platform::IosSimulator);
    assert_eq!(Target::MacosArm64.platform(), Platform::Macos);
    assert_eq!(Target::WindowsX64.platform(), Platform::Windows);
    assert_eq!(Platform::Macos.output_name("demo"), "demo.app");
    assert_eq!(Platform::Android.output_name("demo"), "demo.apk");
    assert_eq!(Platform::Windows.output_name("demo"), "demo");
}

#[test]
fn exposes_canonical_build_metadata_for_every_target() {
    for (
        target,
        rust_target,
        repository_directory,
        recipe,
        runtime_artifact,
        runtime_staging,
        has_native_shell,
        entrypoint,
    ) in TARGET_METADATA.iter().copied()
    {
        let platform = target.platform();
        assert_eq!(target.rust_target(), rust_target);
        assert_eq!(platform.repository_directory_name(), repository_directory);
        assert_eq!(platform.target_pack_recipe_file_name(), recipe);
        assert_eq!(target.runtime_artifact_path(), runtime_artifact);
        assert_eq!(target.runtime_staging_path(), runtime_staging);
        assert_eq!(target.has_native_shell(), has_native_shell);
        assert_eq!(target.build_entrypoint_path(), entrypoint);
    }
}

#[test]
fn target_artifacts_describe_the_complete_pack_contract() {
    let artifacts = Target::MacosArm64.artifacts();

    assert_eq!(
        artifacts
            .iter()
            .map(|artifact| (&artifact.kind, artifact.path.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (
                &ArtifactKind::RuntimeLibrary,
                "frameworks/TokamakRuntime.framework"
            ),
            (&ArtifactKind::NativeShellDirectory, "native-shell"),
            (
                &ArtifactKind::EsbuildExecutable,
                "tools/runtime/node_modules/esbuild/bin/esbuild"
            ),
        ]
    );
}

#[test]
fn validates_manifest_with_relative_artifact_paths() {
    let manifest = valid_manifest();

    assert!(manifest.validate().is_ok());
}

#[test]
fn serializes_every_target_with_its_display_name() -> TestResult {
    for target in Target::ALL {
        let json = serde_json::to_string(target)?;
        assert_eq!(json, format!("\"{target}\""));

        let parsed: Target = serde_json::from_str(&json)?;
        assert_eq!(parsed, *target);
    }

    Ok(())
}

#[test]
fn rejects_blank_tokamak_versions() {
    let mut manifest = valid_manifest();
    manifest.tokamak_version = "  ".to_owned();

    assert!(matches!(
        manifest.validate(),
        Err(TargetPackError::MissingVersion)
    ));
}

#[test]
fn accepts_matching_cli_version() {
    let manifest = valid_manifest();
    assert!(manifest.validate_cli_version("0.1.0").is_ok());
}

#[test]
fn rejects_a_different_cli_version() {
    let manifest = valid_manifest();
    assert!(matches!(
        manifest.validate_cli_version("0.1.1"),
        Err(TargetPackError::IncompatibleTokamakVersion { required, actual })
            if required == "0.1.0" && actual == "0.1.1"
    ));
}

#[test]
fn rejects_manifests_without_artifacts() {
    let mut manifest = valid_manifest();
    manifest.artifacts.clear();

    assert!(matches!(
        manifest.validate(),
        Err(TargetPackError::MissingArtifacts)
    ));
}

#[test]
fn rejects_empty_artifact_paths() {
    let mut manifest = valid_manifest();
    manifest.artifacts[0].path.clear();

    assert!(matches!(
        manifest.validate(),
        Err(TargetPackError::EmptyArtifactPath)
    ));
}

#[test]
fn rejects_absolute_and_parent_artifact_paths() -> TestResult {
    let absolute_path = std::env::current_dir()?
        .join("tokamak-runtime")
        .display()
        .to_string();

    assert_unsafe_artifact_path(&absolute_path);
    assert_unsafe_artifact_path("../tokamak-runtime");

    Ok(())
}

#[test]
fn rejects_windows_absolute_artifact_paths_on_any_host() {
    for path in [
        r"C:\tokamak\tokamak-runtime.exe",
        r"\\server\share\tokamak-runtime.exe",
    ] {
        assert_unsafe_artifact_path(path);
    }
}

#[test]
fn round_trips_manifest_json_without_losing_contract_fields() -> TestResult {
    let temp_dir = tempfile::tempdir()?;
    let manifest_path = temp_dir.path().join("target-pack.json");
    let manifest = valid_manifest();

    write_manifest(&manifest_path, &manifest)?;
    let json = fs::read_to_string(&manifest_path)?;
    assert!(json.contains("\"target\": \"ios-arm64\""));
    assert!(json.contains("\"tokamakVersion\": \"0.1.0\""));
    assert!(!json.contains("requiredCliVersion"));
    assert!(!json.contains("schemaVersion"));
    assert!(!json.contains("sha256"));

    let loaded = load_manifest(&manifest_path)?;
    assert_eq!(loaded, manifest);

    Ok(())
}

#[test]
fn load_manifest_rejects_contract_invalid_json() -> TestResult {
    let temp_dir = tempfile::tempdir()?;
    let manifest_path = temp_dir.path().join("target-pack.json");
    fs::write(
        &manifest_path,
        r#"{
  "tokamakVersion": "0.1.0",
  "target": "ios-arm64",
  "artifacts": [{"kind": "runtimeLibrary", "path": "../tokamak-runtime"}],
  "requiredTools": []
}"#,
    )?;

    assert!(matches!(
        load_manifest(&manifest_path),
        Err(TargetPackError::UnsafeArtifactPath(path)) if path == "../tokamak-runtime"
    ));

    Ok(())
}
