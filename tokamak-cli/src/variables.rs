//! Target-pack environment overrides.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::str::FromStr;

use anyhow::{Result, bail};

/// A command-line target-pack variable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SetVariable {
    key: String,
    value: OsString,
}

impl FromStr for SetVariable {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (key, value) = value
            .split_once('=')
            .ok_or_else(|| "expected NAME=VALUE".to_owned())?;
        if !valid_key(key) {
            return Err(
                "NAME must be lower-case kebab-case, include a platform namespace, and must not start with tokamak-"
                    .to_owned(),
            );
        }
        Ok(Self {
            key: key.to_owned(),
            value: OsString::from(value),
        })
    }
}

pub(crate) fn environment(values: &[SetVariable]) -> Result<BTreeMap<String, OsString>> {
    let mut environment = BTreeMap::new();
    for variable in values {
        let environment_name = format!(
            "TOKAMAK_{}",
            variable.key.replace('-', "_").to_ascii_uppercase()
        );
        if environment
            .insert(environment_name, variable.value.clone())
            .is_some()
        {
            bail!("duplicate --set value for {}", variable.key);
        }
    }
    Ok(environment)
}

fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.contains('-')
        && !key.starts_with('-')
        && !key.ends_with('-')
        && !key.contains("--")
        && !key.starts_with("tokamak-")
        && key.bytes().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == b'-'
        })
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::str::FromStr;

    use super::{SetVariable, environment};

    #[test]
    fn maps_kebab_case_keys_to_tokamak_environment_names() -> anyhow::Result<()> {
        let variable = SetVariable::from_str("ios-team-id=S96CQLW983")
            .map_err(|error| anyhow::anyhow!("valid variable: {error}"))?;
        let environment = environment(&[variable])?;
        assert_eq!(
            environment.get("TOKAMAK_IOS_TEAM_ID"),
            Some(&OsString::from("S96CQLW983"))
        );
        Ok(())
    }

    #[test]
    fn preserves_equals_in_values() {
        let variable = SetVariable::from_str("android-token=a=b")
            .unwrap_or_else(|error| panic!("valid variable: {error}"));
        assert_eq!(variable.value, "a=b");
    }

    #[test]
    fn rejects_invalid_keys() {
        for value in [
            "team=ID",
            "ios_team_id=ID",
            "IOS-team-id=ID",
            "-ios-team-id=ID",
            "ios-team-id-=ID",
            "ios--team=ID",
            "TOKAMAK_IOS_TEAM_ID=ID",
            "tokamak-ios-team-id=ID",
        ] {
            assert!(SetVariable::from_str(value).is_err(), "accepted {value}");
        }
    }

    #[test]
    fn rejects_duplicate_environment_names() {
        let first = SetVariable::from_str("ios-team-id=FIRST")
            .unwrap_or_else(|error| panic!("valid variable: {error}"));
        let second = SetVariable::from_str("ios-team-id=SECOND")
            .unwrap_or_else(|error| panic!("valid variable: {error}"));
        assert!(
            environment(&[first, second]).is_err_and(|error| {
                error.to_string() == "duplicate --set value for ios-team-id"
            })
        );
    }
}
