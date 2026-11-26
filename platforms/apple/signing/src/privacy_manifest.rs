use std::collections::BTreeMap;
use std::env;
use std::path::Path;

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

/// Write the app's privacy manifest: the file the `privacy-manifest` setting
/// names, or otherwise the runtime's required reason APIs joined with each
/// plugin's privacy manifest.
///
/// # Errors
///
/// Returns an error when the setting's file or a plugin's manifest cannot be
/// read as a plist dictionary, or when the manifest cannot be written.
pub fn write_privacy_manifest(input: &Path, output: &Path) -> Result<()> {
    let platform = read_required(&input.join("metadata/platform"))?;
    let variable = pack_variable(&platform, "PRIVACY_MANIFEST");
    let manifest = match env::var_os(&variable) {
        Some(value) => read_dictionary(&configured_file(&variable, value)?)?,
        None => generated_manifest(input)?,
    };
    Value::Dictionary(manifest)
        .to_file_xml(output)
        .with_context(|| format!("write privacy manifest {}", output.display()))
}

/// The runtime's required reason APIs joined with the privacy manifest of
/// each plugin staged in `input`: tracking when any tracks, and the union of
/// the tracking domains, collected data types and each API category's reasons.
fn generated_manifest(input: &Path) -> Result<Dictionary> {
    let mut tracking = false;
    let mut tracking_domains = Vec::new();
    let mut collected_data = Vec::new();
    let mut apis: BTreeMap<String, Vec<Value>> = RUNTIME_APIS
        .iter()
        .map(|&(category, reason)| (category.into(), vec![reason.into()]))
        .collect();
    for plugin in sorted_directories(&input.join("plugins"))? {
        let path = plugin.join("privacy-manifest");
        if !path.is_file() {
            continue;
        }
        let manifest = read_dictionary(&path)?;
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
    use std::fs;

    use plist::{Dictionary, Value};

    use super::generated_manifest;

    /// An accessed API entry of `category` with `reasons`.
    fn api(category: &str, reasons: &[&str]) -> Value {
        let reasons = reasons.iter().map(|reason| Value::from(*reason)).collect();
        Value::Dictionary(Dictionary::from_iter([
            ("NSPrivacyAccessedAPIType", Value::from(category)),
            ("NSPrivacyAccessedAPITypeReasons", Value::Array(reasons)),
        ]))
    }

    #[test]
    fn joins_the_runtimes_apis_with_each_plugins_manifest() -> anyhow::Result<()> {
        let input = tempfile::tempdir()?;
        let user_defaults = "NSPrivacyAccessedAPICategoryUserDefaults";
        let manifests = [
            (
                "alerts",
                Dictionary::from_iter([(
                    "NSPrivacyAccessedAPITypes",
                    Value::Array(vec![api(user_defaults, &["CA92.1"])]),
                )]),
            ),
            (
                "places",
                Dictionary::from_iter([
                    ("NSPrivacyTracking", Value::Boolean(true)),
                    (
                        "NSPrivacyTrackingDomains",
                        Value::Array(vec!["example.com".into()]),
                    ),
                    (
                        "NSPrivacyAccessedAPITypes",
                        Value::Array(vec![
                            api(user_defaults, &["CA92.1", "1C8F.1"]),
                            api("NSPrivacyAccessedAPICategoryFileTimestamp", &["DDA9.1"]),
                        ]),
                    ),
                ]),
            ),
        ];
        for (plugin, manifest) in manifests {
            let directory = input.path().join("plugins").join(plugin);
            fs::create_dir_all(&directory)?;
            Value::Dictionary(manifest).to_file_xml(directory.join("privacy-manifest"))?;
        }
        fs::create_dir_all(input.path().join("plugins/silent"))?;

        assert_eq!(
            generated_manifest(input.path())?,
            Dictionary::from_iter([
                ("NSPrivacyTracking", Value::Boolean(true)),
                (
                    "NSPrivacyTrackingDomains",
                    Value::Array(vec!["example.com".into()]),
                ),
                ("NSPrivacyCollectedDataTypes", Value::Array(Vec::new())),
                (
                    "NSPrivacyAccessedAPITypes",
                    Value::Array(vec![
                        api("NSPrivacyAccessedAPICategoryDiskSpace", &["E174.1"]),
                        api(
                            "NSPrivacyAccessedAPICategoryFileTimestamp",
                            &["C617.1", "DDA9.1"],
                        ),
                        api(user_defaults, &["CA92.1", "1C8F.1"]),
                    ]),
                ),
            ])
        );
        Ok(())
    }

    #[test]
    fn rejects_a_plugins_accessed_api_without_a_type() -> anyhow::Result<()> {
        let input = tempfile::tempdir()?;
        let directory = input.path().join("plugins/alerts");
        fs::create_dir_all(&directory)?;
        let manifest = Dictionary::from_iter([(
            "NSPrivacyAccessedAPITypes",
            Value::Array(vec![Value::Dictionary(Dictionary::new())]),
        )]);
        Value::Dictionary(manifest).to_file_xml(directory.join("privacy-manifest"))?;

        let Err(error) = generated_manifest(input.path()) else {
            anyhow::bail!("the manifest was accepted");
        };
        assert!(
            format!("{error:#}").contains("has an accessed API without a type"),
            "{error:#}"
        );
        Ok(())
    }
}
