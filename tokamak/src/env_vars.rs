//! Serialized Worker environment bindings.

use std::collections::BTreeMap;
use std::fs;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::packaging::PackageLayout;

/// Failures reading or writing the packaged Worker environment.
#[derive(Debug, Error)]
pub enum Error {
    /// Operating-system IO failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// JSON encoding or decoding failed.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Result type for packaged environment operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Bindings that tokamak passes to a Worker at runtime.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkerEnvironment {
    /// Text and JSON values declared in Wrangler's `vars` section.
    #[serde(default)]
    pub vars: BTreeMap<String, Value>,
    /// Bindings backed by stores on the device.
    #[serde(default)]
    pub storage: Vec<StorageBinding>,
}

/// A binding to a store on the device, identified as Cloudflare identifies
/// the resource it names.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum StorageBinding {
    /// A D1 database.
    D1 {
        /// Binding name in the Worker's `env`.
        name: String,
        /// Database identifier.
        id: String,
        /// Table recording applied migrations.
        migrations_table: String,
        /// Packaged migrations, relative to the binding's migrations
        /// directory, in the order they apply.
        migrations: Vec<String>,
    },
}

impl StorageBinding {
    /// Binding name in the Worker's `env`.
    #[must_use]
    pub fn name(&self) -> &str {
        let Self::D1 { name, .. } = self;
        name
    }

    /// Identifier of the store the binding names.
    #[must_use]
    pub fn store(&self) -> &str {
        let Self::D1 { id, .. } = self;
        id
    }
}

/// Longest store identifier; SQLite's suffixes then fit a 255-byte file name.
const MAX_STORE_ID_LENGTH: usize = 200;

/// Why `id` cannot name a store's files on every platform, when it cannot.
#[must_use]
pub fn store_id_problem(id: &str) -> Option<&'static str> {
    if id.is_empty() || id.len() > MAX_STORE_ID_LENGTH {
        return Some("must be 1 to 200 characters");
    }
    if !id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "-_.$".contains(character))
    {
        return Some("may contain only ASCII letters, digits, '-', '_', '.' and '$'");
    }
    if id.starts_with('.') || id.ends_with('.') {
        return Some("must not start or end with '.'");
    }
    let stem = id.split('.').next().unwrap_or(id).to_ascii_uppercase();
    let numbered = (stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.len() == 4
        && stem.ends_with(|character: char| character.is_ascii_digit());
    (numbered || ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str()))
        .then_some("is a device name Windows reserves")
}

/// Write the normalized Worker environment into an app bundle.
///
/// # Errors
///
/// Returns an error when the environment cannot be serialized or written.
pub fn write(layout: &PackageLayout, environment: &WorkerEnvironment) -> Result<()> {
    fs::write(
        layout.worker_environment(),
        serde_json::to_vec(environment)?,
    )?;
    Ok(())
}

/// Read the normalized Worker environment from an app bundle.
///
/// # Errors
///
/// Returns an error when the environment cannot be read or parsed.
pub fn load(layout: &PackageLayout) -> Result<WorkerEnvironment> {
    Ok(serde_json::from_slice(&fs::read(
        layout.worker_environment(),
    )?)?)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::{StorageBinding, WorkerEnvironment, load, store_id_problem, write};
    use crate::packaging::PackageLayout;

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    #[test]
    fn round_trips_text_and_json_vars() -> TestResult {
        let directory = tempfile::tempdir()?;
        let layout = PackageLayout::new(directory.path());
        let environment = WorkerEnvironment {
            vars: BTreeMap::from([
                ("TEXT".to_owned(), json!("value")),
                ("JSON".to_owned(), json!({ "enabled": true })),
            ]),
            storage: vec![StorageBinding::D1 {
                name: "DB".to_owned(),
                id: "app".to_owned(),
                migrations_table: "d1_migrations".to_owned(),
                migrations: vec!["0001_init.sql".to_owned()],
            }],
        };

        write(&layout, &environment)?;

        assert_eq!(load(&layout)?, environment);
        Ok(())
    }

    #[test]
    fn describes_storage_bindings_by_type() -> TestResult {
        let binding: StorageBinding = serde_json::from_value(json!({
            "type": "d1", "name": "DB", "id": "app", "migrationsTable": "applied", "migrations": [],
        }))?;

        assert_eq!(binding.name(), "DB");
        assert_eq!(binding.store(), "app");
        Ok(())
    }

    #[test]
    fn accepts_only_identifiers_safe_as_file_names() {
        for id in [
            "0f2ac74b498b48028cb68387c421e279",
            "8a5e1d43-7c3e-4bb6-9a8e-0d2f4c1b3e5a",
            "app-files",
            "SESSION",
            "my.bucket",
            "$DB_2",
            "CONSOLE",
        ] {
            assert_eq!(store_id_problem(id), None, "{id}");
        }
        for id in [
            "",
            "../escape",
            "a/b",
            "a\\b",
            ".hidden",
            "trailing.",
            "space d",
            "CON",
            "nul.db",
            "com1",
            "LPT9",
            "é",
        ] {
            assert!(store_id_problem(id).is_some(), "{id}");
        }
        assert!(store_id_problem(&"a".repeat(201)).is_some());
    }
}
