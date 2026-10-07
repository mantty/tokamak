use std::collections::BTreeMap;
use std::fs;
use std::str::FromStr;

use tokamak_cli::{
    PackVariable, Platform, PlatformPackError, PlatformPackManifest, PluginKeyKind, Target,
    VariableKind, is_valid_key, load_manifest, write_manifest,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
fn valid_manifest() -> PlatformPackManifest {
    PlatformPackManifest {
        tokamak_version: "0.1.0".to_owned(),
        target: Target::IosArm64,
        variables: BTreeMap::from([(
            "plist".to_owned(),
            PackVariable {
                kind: VariableKind::Path,
                description: "Info.plist values layered over the generated plist".to_owned(),
            },
        )]),
        plugin_keys: BTreeMap::from([("sources".to_owned(), PluginKeyKind::Paths)]),
    }
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
        Err(PlatformPackError::UnknownTarget(target)) if target == "ios-armv7"
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
fn names_each_target_s_build_entrypoint() {
    for target in Target::ALL {
        let expected = if *target == Target::WindowsX64 {
            "build/entrypoint.ps1"
        } else {
            "build/entrypoint"
        };
        assert_eq!(target.build_entrypoint_path(), expected);
    }
}

#[test]
fn validates_a_manifest() {
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
        Err(PlatformPackError::MissingVersion)
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
        Err(PlatformPackError::IncompatibleTokamakVersion { required, actual })
            if required == "0.1.0" && actual == "0.1.1"
    ));
}

#[test]
fn ios_devices_and_simulators_share_a_namespace() {
    let namespaces = Platform::ALL
        .iter()
        .map(|platform| platform.namespace())
        .collect::<Vec<_>>();
    assert_eq!(namespaces, ["android", "ios", "ios", "macos", "windows"]);
}

#[test]
fn rejects_invalid_variables() {
    for (name, description) in [
        ("Plist", "Info.plist values"),
        ("team_id", "Signing team"),
        ("identifier", "Bundle identifier"),
        ("plist", " "),
    ] {
        let mut manifest = valid_manifest();
        manifest.variables = BTreeMap::from([(
            name.to_owned(),
            PackVariable {
                kind: VariableKind::String,
                description: description.to_owned(),
            },
        )]);
        let result = manifest.validate();
        assert!(
            matches!(
                result,
                Err(PlatformPackError::InvalidVariableName(ref rejected)
                    | PlatformPackError::ReservedVariableName(ref rejected)
                    | PlatformPackError::MissingVariableDescription(ref rejected)) if rejected == name
            ),
            "accepted {name}: {result:?}"
        );
    }
}

#[test]
fn rejects_invalid_plugin_keys() {
    let mut manifest = valid_manifest();
    manifest.plugin_keys = BTreeMap::from([("../class".to_owned(), PluginKeyKind::String)]);

    assert!(matches!(
        manifest.validate(),
        Err(PlatformPackError::InvalidPluginKey(key)) if key == "../class"
    ));
}

#[test]
fn round_trips_manifest_json_without_losing_contract_fields() -> TestResult {
    let temp_dir = tempfile::tempdir()?;
    let manifest_path = temp_dir.path().join("platform-pack.json");
    let manifest = valid_manifest();

    write_manifest(&manifest_path, &manifest)?;
    let json = fs::read_to_string(&manifest_path)?;
    assert!(json.contains("\"target\": \"ios-arm64\""));
    assert!(json.contains("\"tokamakVersion\": \"0.1.0\""));
    assert!(json.contains("\"kind\": \"path\""));
    assert!(json.contains("\"sources\": \"paths\""));
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
    let manifest_path = temp_dir.path().join("platform-pack.json");
    fs::write(
        &manifest_path,
        r#"{
  "tokamakVersion": "0.1.0",
  "target": "ios-arm64",
  "variables": {},
  "pluginKeys": { "../class": "string" }
}"#,
    )?;

    assert!(matches!(
        load_manifest(&manifest_path),
        Err(PlatformPackError::InvalidPluginKey(key)) if key == "../class"
    ));

    Ok(())
}

#[test]
fn keys_are_lowercase_words_joined_by_hyphens() {
    for key in ["plist", "team-id", "build-number", "a1-b2"] {
        assert!(is_valid_key(key), "rejected {key}");
    }
    for key in [
        "", "Plist", "team_id", "-plist", "plist-", "team--id", "1plist",
    ] {
        assert!(!is_valid_key(key), "accepted {key}");
    }
}
