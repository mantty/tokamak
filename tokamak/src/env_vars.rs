//! Serialized Worker environment bindings.

use std::collections::BTreeMap;
use std::fs;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::packaging::{PackageLayout, Result};

/// Bindings that tokamak passes to a Worker at runtime.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkerEnvironment {
    /// Text and JSON values declared in Wrangler's `vars` section.
    #[serde(default)]
    pub vars: BTreeMap<String, Value>,
    /// Bindings backed by stores on the device.
    #[serde(default)]
    pub storage: Vec<StorageBinding>,
    /// Bindings to packaged Workers and their exported entrypoints.
    #[serde(default)]
    pub services: BTreeMap<String, ServiceBinding>,
}

/// A receiving Worker and its module export.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceBinding {
    /// Packaged service name: `main` or `tokamak`.
    pub service: String,
    /// Exported entrypoint, including `default`.
    pub entrypoint: String,
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
    /// A KV namespace.
    Kv {
        /// Binding name in the Worker's `env`.
        name: String,
        /// Namespace identifier.
        id: String,
    },
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
    /// An R2 bucket.
    R2 {
        /// Binding name in the Worker's `env`.
        name: String,
        /// Bucket name.
        id: String,
    },
}

impl StorageBinding {
    /// Binding name in the Worker's `env`.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Kv { name, .. } | Self::D1 { name, .. } | Self::R2 { name, .. } => name,
        }
    }

    /// Identifier of the store the binding names.
    #[must_use]
    pub fn store(&self) -> &str {
        match self {
            Self::Kv { id, .. } | Self::D1 { id, .. } | Self::R2 { id, .. } => id,
        }
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
#[cfg(feature = "native")]
pub(crate) fn load(layout: &PackageLayout) -> Result<WorkerEnvironment> {
    crate::packaging::read_json(&layout.worker_environment())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::{ServiceBinding, StorageBinding, WorkerEnvironment, store_id_problem, write};
    use crate::packaging::{PackageLayout, read_json};

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    #[test]
    fn round_trips_text_and_json_vars() -> TestResult {
        let directory = tempfile::tempdir()?;
        let layout = PackageLayout::new(directory.path());
        let environment = WorkerEnvironment {
            services: BTreeMap::from([(
                "DEVICE".to_owned(),
                ServiceBinding {
                    service: "tokamak".to_owned(),
                    entrypoint: "Device".to_owned(),
                },
            )]),
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

        assert_eq!(
            read_json::<WorkerEnvironment>(&layout.worker_environment())?,
            environment
        );
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
        let too_long = "a".repeat(201);
        for (id, safe) in [
            ("0f2ac74b498b48028cb68387c421e279", true),
            ("8a5e1d43-7c3e-4bb6-9a8e-0d2f4c1b3e5a", true),
            ("app-files", true),
            ("SESSION", true),
            ("my.bucket", true),
            ("$DB_2", true),
            ("CONSOLE", true),
            ("", false),
            ("../escape", false),
            ("a/b", false),
            ("a\\b", false),
            (".hidden", false),
            ("trailing.", false),
            ("space d", false),
            ("CON", false),
            ("nul.db", false),
            ("com1", false),
            ("LPT9", false),
            ("é", false),
            (too_long.as_str(), false),
        ] {
            let problem = store_id_problem(id);
            assert_eq!(problem.is_none(), safe, "{id}: {problem:?}");
        }
    }
}
