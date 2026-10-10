//! App-declared entitlements and how they combine with a profile's.

use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use plist::{Dictionary, Value};

#[cfg(any(target_os = "macos", test))]
use crate::Platform;
use crate::info_plist::overlay_dictionary;

/// Entitlements signed with the profile's value, so one declaration serves
/// development and distribution profiles.
#[cfg(any(target_os = "macos", test))]
const PROFILE_VALUED: [&str; 2] = ["aps-environment", "com.apple.developer.aps-environment"];

/// The app's entitlements file, named by `variable`, when it is set.
///
/// # Errors
///
/// Returns an error when the file is missing or is not a plist with a
/// dictionary root.
pub(crate) fn declared(variable: &str) -> Result<Option<Dictionary>> {
    let Some(path) = env::var_os(variable).map(PathBuf::from) else {
        return Ok(None);
    };
    if !path.is_file() {
        bail!("{variable} does not point to a file: {}", path.display());
    }
    Value::from_file(&path)
        .with_context(|| format!("read entitlements {}", path.display()))?
        .into_dictionary()
        .map(Some)
        .with_context(|| format!("entitlements root must be a dictionary: {}", path.display()))
}

/// The first declared entitlement the profile's entitlements do not permit.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn unpermitted<'a>(profile: &Dictionary, declared: &'a Dictionary) -> Option<&'a str> {
    declared
        .iter()
        .find(|(key, value)| match profile.get(key.as_str()) {
            None => true,
            Some(_) if PROFILE_VALUED.contains(&key.as_str()) => false,
            Some(allowed) => !permits(allowed, value),
        })
        .map(|(key, _)| key.as_str())
}

/// Whether a profile's `allowed` value covers a `declared` value: the same
/// value, `*`, or a prefix wildcard such as `TEAMID.*`.
#[cfg(any(target_os = "macos", test))]
fn permits(allowed: &Value, declared: &Value) -> bool {
    match (allowed, declared) {
        (Value::String(allowed), Value::String(declared)) => {
            allowed == declared
                || allowed
                    .strip_suffix('*')
                    .is_some_and(|prefix| declared.starts_with(prefix))
        }
        (_, Value::Array(declared)) => declared.iter().all(|value| permits(allowed, value)),
        (Value::Array(allowed), declared) => allowed.iter().any(|value| permits(value, declared)),
        (allowed, declared) => allowed == declared,
    }
}

/// The profile's entitlements overlaid with the declared ones, keeping the
/// profile's value for profile-valued entitlements. The application identifier
/// is the app's own, not the profile's pattern, and is the only keychain group
/// unless groups are declared; the profile's `TEAMID.*` group is shared by
/// every app of the team.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn for_signing(
    profile: &Dictionary,
    platform: Platform,
    application_identifier: &str,
    declared: Option<&Dictionary>,
) -> Dictionary {
    let mut entitlements = profile.clone();
    let identifier = Value::String(application_identifier.to_owned());
    if entitlements.contains_key("keychain-access-groups") {
        entitlements.insert(
            "keychain-access-groups".to_owned(),
            Value::Array(vec![identifier.clone()]),
        );
    }
    entitlements.insert(platform.application_identifier_key().to_owned(), identifier);
    if let Some(declared) = declared {
        overlay_dictionary(&mut entitlements, declared.clone());
        for key in PROFILE_VALUED {
            if let Some(value) = profile.get(key) {
                entitlements.insert(key.to_owned(), value.clone());
            }
        }
    }
    entitlements
}

/// The entitlements a simulator build embeds in its executable: the app's
/// identifier and keychain group, overlaid with the declared entitlements.
pub(crate) fn for_simulator(identifier: &str, declared: Option<&Dictionary>) -> Dictionary {
    let mut entitlements = Dictionary::new();
    entitlements.insert(
        "application-identifier".to_owned(),
        Value::String(identifier.to_owned()),
    );
    entitlements.insert(
        "keychain-access-groups".to_owned(),
        Value::Array(vec![Value::String(identifier.to_owned())]),
    );
    if let Some(declared) = declared {
        overlay_dictionary(&mut entitlements, declared.clone());
    }
    entitlements
}

/// The first declared entitlement that only a provisioning profile can
/// authorise, which an ad-hoc signature cannot carry.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn first_requiring_profile(declared: &Dictionary) -> Option<&str> {
    declared.keys().map(String::as_str).find(|key| {
        key.starts_with("com.apple.developer.")
            || matches!(
                *key,
                "aps-environment" | "com.apple.application-identifier" | "keychain-access-groups"
            )
    })
}

#[cfg(test)]
mod tests {
    use plist::{Dictionary, Value};

    use super::{first_requiring_profile, for_signing, for_simulator, unpermitted};
    use crate::Platform;

    fn dictionary(values: &[(&str, Value)]) -> Dictionary {
        values
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect()
    }

    fn string(value: &str) -> Value {
        Value::String(value.to_owned())
    }

    fn strings(values: &[&str]) -> Value {
        Value::Array(values.iter().copied().map(string).collect())
    }

    fn profile() -> Dictionary {
        dictionary(&[
            ("application-identifier", string("TEAM.com.example.app")),
            ("aps-environment", string("development")),
            ("com.apple.developer.associated-domains", string("*")),
            ("keychain-access-groups", strings(&["TEAM.*"])),
            ("get-task-allow", Value::Boolean(true)),
        ])
    }

    #[test]
    fn permits_values_the_profile_holds_or_covers_with_a_wildcard() {
        let declared = dictionary(&[
            ("get-task-allow", Value::Boolean(true)),
            (
                "com.apple.developer.associated-domains",
                strings(&["applinks:example.com"]),
            ),
            (
                "keychain-access-groups",
                strings(&["TEAM.com.example.shared"]),
            ),
        ]);

        assert_eq!(unpermitted(&profile(), &declared), None);
    }

    #[test]
    fn permits_any_value_of_a_profile_valued_entitlement() {
        let declared = dictionary(&[("aps-environment", string("production"))]);

        assert_eq!(unpermitted(&profile(), &declared), None);
    }

    #[test]
    fn names_an_entitlement_the_profile_does_not_permit() {
        for declared in [
            dictionary(&[(
                "com.apple.developer.icloud-services",
                strings(&["CloudKit"]),
            )]),
            dictionary(&[("keychain-access-groups", strings(&["OTHER.com.example"]))]),
            dictionary(&[("get-task-allow", Value::Boolean(false))]),
        ] {
            let key = declared.keys().next().map(String::as_str);
            assert_eq!(unpermitted(&profile(), &declared), key);
        }
    }

    #[test]
    fn overlays_declared_entitlements_and_keeps_profile_values() {
        let declared = dictionary(&[
            ("aps-environment", string("production")),
            (
                "keychain-access-groups",
                strings(&["TEAM.com.example.shared"]),
            ),
        ]);

        let entitlements = for_signing(
            &profile(),
            Platform::Ios,
            "TEAM.com.example.app",
            Some(&declared),
        );

        assert_eq!(
            entitlements.get("aps-environment"),
            Some(&string("development"))
        );
        assert_eq!(
            entitlements.get("keychain-access-groups"),
            Some(&strings(&["TEAM.com.example.shared"]))
        );
        assert_eq!(
            entitlements.get("get-task-allow"),
            Some(&Value::Boolean(true))
        );
    }

    #[test]
    fn signs_with_the_apps_identifier_and_keychain_group_in_place_of_the_profiles_patterns() {
        let mut wildcard = profile();
        wildcard.insert("application-identifier".to_owned(), string("TEAM.*"));

        let entitlements = for_signing(&wildcard, Platform::Ios, "TEAM.com.example.app", None);

        let mut expected = profile();
        expected.insert(
            "keychain-access-groups".to_owned(),
            strings(&["TEAM.com.example.app"]),
        );
        assert_eq!(entitlements, expected);
    }

    #[test]
    fn signs_a_mac_app_with_its_own_identifier_and_no_ungranted_keychain_group() {
        let profile = dictionary(&[("com.apple.application-identifier", string("TEAM.*"))]);

        assert_eq!(
            for_signing(&profile, Platform::Macos, "TEAM.com.example.app", None),
            dictionary(&[(
                "com.apple.application-identifier",
                string("TEAM.com.example.app")
            )])
        );
    }

    #[test]
    fn embeds_declared_entitlements_for_the_simulator() {
        let declared = dictionary(&[("aps-environment", string("development"))]);

        assert_eq!(
            for_simulator("com.example.app", Some(&declared)),
            dictionary(&[
                ("application-identifier", string("com.example.app")),
                ("keychain-access-groups", strings(&["com.example.app"])),
                ("aps-environment", string("development")),
            ])
        );
    }

    #[test]
    fn finds_entitlements_an_ad_hoc_signature_cannot_carry() {
        let sandboxed = dictionary(&[("com.apple.security.app-sandbox", Value::Boolean(true))]);
        let pushed = dictionary(&[
            ("com.apple.security.app-sandbox", Value::Boolean(true)),
            ("com.apple.developer.aps-environment", string("development")),
        ]);

        assert_eq!(first_requiring_profile(&sandboxed), None);
        assert_eq!(
            first_requiring_profile(&pushed),
            Some("com.apple.developer.aps-environment")
        );
    }
}
