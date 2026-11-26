use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use plist::{Dictionary, Value};

use crate::info_plist::{
    append_missing, configured_file, pack_variable, read_dictionary, read_required,
    sorted_directories,
};

/// The required reason APIs the runtime's executable calls, each with its
/// reason: metadata of files in the app's container, and the file system
/// checks of the SQLite it bundles.
const RUNTIME_APIS: [(&str, &str); 2] = [
    ("NSPrivacyAccessedAPICategoryDiskSpace", "E174.1"),
    ("NSPrivacyAccessedAPICategoryFileTimestamp", "C617.1"),
];

/// Write the app's privacy manifest: the runtime's required reason APIs joined
/// with each plugin's privacy manifest and the file the `privacy-manifest`
/// setting names.
///
/// # Errors
///
/// Returns an error when the setting's file or a plugin's manifest cannot be
/// read as a plist dictionary, or when the manifest cannot be written.
pub fn write_privacy_manifest(input: &Path, output: &Path) -> Result<()> {
    let platform = read_required(&input.join("metadata/platform"))?;
    let variable = pack_variable(&platform, "PRIVACY_MANIFEST");
    let mut manifests: Vec<PathBuf> = sorted_directories(&input.join("plugins"))?
        .into_iter()
        .map(|plugin| plugin.join("privacy-manifest"))
        .filter(|path| path.is_file())
        .collect();
    if let Some(value) = env::var_os(&variable) {
        manifests.push(configured_file(&variable, value)?);
    }
    Value::Dictionary(joined_manifest(&manifests)?)
        .to_file_xml(output)
        .with_context(|| format!("write privacy manifest {}", output.display()))
}

/// The runtime's required reason APIs joined with `manifests`: tracking when
/// any tracks, and the union of the tracking domains, collected data types and
/// each API category's reasons.
fn joined_manifest(manifests: &[PathBuf]) -> Result<Dictionary> {
    let mut tracking = false;
    let mut tracking_domains = Vec::new();
    let mut collected_data = Vec::new();
    let mut apis: BTreeMap<String, Vec<Value>> = RUNTIME_APIS
        .iter()
        .map(|&(category, reason)| (category.into(), vec![reason.into()]))
        .collect();
    for path in manifests {
        let manifest = read_dictionary(path)?;
        tracking |= manifest.get("NSPrivacyTracking") == Some(&Value::Boolean(true));
        append_missing(
            &mut tracking_domains,
            array(&manifest, "NSPrivacyTrackingDomains"),
        );
        append_missing(
            &mut collected_data,
            array(&manifest, "NSPrivacyCollectedDataTypes"),
        );
        for api in array(&manifest, "NSPrivacyAccessedAPITypes") {
            let api = api.into_dictionary().unwrap_or_default();
            let category = api
                .get("NSPrivacyAccessedAPIType")
                .and_then(Value::as_string)
                .with_context(|| {
                    format!("{} has an accessed API without a type", path.display())
                })?;
            append_missing(
                apis.entry(category.into()).or_default(),
                array(&api, "NSPrivacyAccessedAPITypeReasons"),
            );
        }
    }
    let apis = apis.into_iter().map(|(category, reasons)| {
        Value::Dictionary(Dictionary::from_iter([
            ("NSPrivacyAccessedAPIType", Value::String(category)),
            ("NSPrivacyAccessedAPITypeReasons", Value::Array(reasons)),
        ]))
    });
    Ok(Dictionary::from_iter([
        ("NSPrivacyTracking", Value::Boolean(tracking)),
        ("NSPrivacyTrackingDomains", Value::Array(tracking_domains)),
        ("NSPrivacyCollectedDataTypes", Value::Array(collected_data)),
        ("NSPrivacyAccessedAPITypes", Value::Array(apis.collect())),
    ]))
}

/// The array `dictionary` has at `key`, empty without one.
fn array(dictionary: &Dictionary, key: &str) -> Vec<Value> {
    dictionary
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use plist::{Dictionary, Value};

    use super::joined_manifest;

    /// An accessed API entry of `category` with `reasons`.
    fn api(category: &str, reasons: &[&str]) -> Value {
        let reasons = reasons.iter().map(|reason| Value::from(*reason)).collect();
        Value::Dictionary(Dictionary::from_iter([
            ("NSPrivacyAccessedAPIType", Value::from(category)),
            ("NSPrivacyAccessedAPITypeReasons", Value::Array(reasons)),
        ]))
    }

    /// A manifest that tracks through one domain and collects one data type,
    /// with the accessed `apis`.
    fn tracking_manifest(apis: Vec<Value>) -> Dictionary {
        let collected = Dictionary::from_iter([(
            "NSPrivacyCollectedDataType",
            Value::from("NSPrivacyCollectedDataTypeEmailAddress"),
        )]);
        Dictionary::from_iter([
            ("NSPrivacyTracking", Value::Boolean(true)),
            (
                "NSPrivacyTrackingDomains",
                Value::Array(vec!["example.com".into()]),
            ),
            (
                "NSPrivacyCollectedDataTypes",
                Value::Array(vec![Value::Dictionary(collected)]),
            ),
            ("NSPrivacyAccessedAPITypes", Value::Array(apis)),
        ])
    }

    #[test]
    fn joins_the_runtimes_apis_with_each_manifest() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let user_defaults = "NSPrivacyAccessedAPICategoryUserDefaults";
        let file_timestamp = "NSPrivacyAccessedAPICategoryFileTimestamp";
        let manifests = [
            Dictionary::from_iter([(
                "NSPrivacyAccessedAPITypes",
                Value::Array(vec![api(user_defaults, &["CA92.1"])]),
            )]),
            tracking_manifest(vec![
                api(user_defaults, &["CA92.1", "1C8F.1"]),
                api(file_timestamp, &["DDA9.1"]),
            ]),
        ];
        let mut paths = Vec::new();
        for (index, manifest) in manifests.into_iter().enumerate() {
            let path = directory.path().join(index.to_string());
            Value::Dictionary(manifest).to_file_xml(&path)?;
            paths.push(path);
        }

        assert_eq!(
            joined_manifest(&paths)?,
            tracking_manifest(vec![
                api("NSPrivacyAccessedAPICategoryDiskSpace", &["E174.1"]),
                api(file_timestamp, &["C617.1", "DDA9.1"]),
                api(user_defaults, &["CA92.1", "1C8F.1"]),
            ])
        );
        Ok(())
    }

    #[test]
    fn rejects_an_accessed_api_without_a_type() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("privacy-manifest");
        let manifest = Dictionary::from_iter([(
            "NSPrivacyAccessedAPITypes",
            Value::Array(vec![Value::Dictionary(Dictionary::new())]),
        )]);
        Value::Dictionary(manifest).to_file_xml(&path)?;

        let Err(error) = joined_manifest(&[path]) else {
            anyhow::bail!("the manifest was accepted");
        };
        assert!(
            format!("{error:#}").contains("has an accessed API without a type"),
            "{error:#}"
        );
        Ok(())
    }
}
