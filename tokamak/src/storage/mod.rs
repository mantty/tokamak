//! The stores behind a packaged app's storage bindings, in the app's private
//! data directory.

mod d1;
mod host;
mod sqlite;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use serde::Serialize;

use crate::env_vars::{StorageBinding, store_id_problem};
use crate::packaging::PackageLayout;
use d1::{D1Database, Migrations};

pub(crate) use host::{HOST_EXPORTS, StorageHandle, export_host_functions};

/// Directory of the D1 databases.
const D1: &str = "d1";

/// Files of a store kept in one SQLite database.
const SQLITE_FILES: [&str; 4] = [".sqlite", ".sqlite-wal", ".sqlite-shm", ".sqlite-journal"];

/// The stores a packaged app's bindings name, each opened on first use.
#[derive(Debug)]
pub(crate) struct Storage {
    bindings: BTreeMap<String, D1Binding>,
    /// The bindings for the Worker's `env`, as JSON for the bootstrap.
    installed: String,
}

#[derive(Debug)]
struct D1Binding {
    database: Arc<Database>,
    migrations: Migrations,
    /// The outcome of applying `migrations` in this process.
    migrated: OnceLock<Result<(), String>>,
}

/// A D1 database file and, once used, its connection.
#[derive(Debug)]
struct Database {
    path: PathBuf,
    opened: Mutex<Option<Arc<D1Database>>>,
}

/// A binding the Worker's `env` receives.
#[derive(Serialize)]
struct Installed<'a> {
    name: &'a str,
    #[serde(rename = "type")]
    kind: &'static str,
}

impl Storage {
    /// The stores `bindings` name in `directory`, after deleting every store
    /// there that no binding names.
    pub(crate) fn open(
        directory: &Path,
        app: &PackageLayout,
        bindings: &[StorageBinding],
    ) -> io::Result<Self> {
        for binding in bindings {
            if let Some(problem) = store_id_problem(binding.store()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "store `{}` of binding {} {problem}",
                        binding.store(),
                        binding.name()
                    ),
                ));
            }
        }
        remove_unnamed_stores(&directory.join(D1), bindings)?;
        let mut databases: BTreeMap<&str, Arc<Database>> = BTreeMap::new();
        let mut entries = BTreeMap::new();
        for StorageBinding::D1 {
            name,
            id,
            migrations_table,
            migrations,
        } in bindings
        {
            let database = databases.entry(id).or_insert_with(|| {
                Arc::new(Database {
                    path: directory.join(D1).join(format!("{id}.sqlite")),
                    opened: Mutex::new(None),
                })
            });
            let migrations = Migrations {
                directory: app.d1_migrations(name),
                names: migrations.clone(),
                table: migrations_table.clone(),
            };
            entries.insert(
                name.clone(),
                D1Binding {
                    database: Arc::clone(database),
                    migrations,
                    migrated: OnceLock::new(),
                },
            );
        }
        let installed: Vec<Installed<'_>> = entries
            .keys()
            .map(|name| Installed { name, kind: D1 })
            .collect();
        let installed = serde_json::to_string(&installed).map_err(io::Error::other)?;
        Ok(Self {
            bindings: entries,
            installed,
        })
    }

    /// Whether no binding names a store.
    pub(crate) fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// The bindings for the Worker's `env`, as a JSON array of `name` and
    /// `type`.
    pub(crate) fn installed(&self) -> &str {
        &self.installed
    }

    /// Answer a D1 service request for the binding `name`, returning the
    /// response body and the database's bookmark.
    ///
    /// The binding's migrations apply before its first query in a process.
    /// A failed migration fails every query until a later process applies it.
    pub(crate) fn d1_query(&self, name: &str, body: &str, format: &str) -> (String, String) {
        match self.d1(name) {
            Ok(database) => database.query(body, format),
            Err(error) => (d1::failure_body(&error), String::new()),
        }
    }

    fn d1(&self, name: &str) -> Result<Arc<D1Database>, String> {
        let binding = self
            .bindings
            .get(name)
            .ok_or_else(|| format!("{name} is not a D1 binding"))?;
        let database = binding.database.get()?;
        binding
            .migrated
            .get_or_init(|| database.migrate(&binding.migrations))
            .clone()?;
        Ok(database)
    }
}

impl Database {
    /// The open database, opening it when this is its first use.
    fn get(&self) -> Result<Arc<D1Database>, String> {
        let mut opened = lock(&self.opened);
        if let Some(database) = opened.as_ref() {
            return Ok(Arc::clone(database));
        }
        if let Some(directory) = self.path.parent() {
            fs::create_dir_all(directory).map_err(|error| error.to_string())?;
        }
        let database = Arc::new(D1Database::open(&self.path)?);
        *opened = Some(Arc::clone(&database));
        Ok(database)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Delete every file in `directory` that belongs to no named store.
fn remove_unnamed_stores(directory: &Path, bindings: &[StorageBinding]) -> io::Result<()> {
    let named: BTreeSet<&str> = bindings.iter().map(StorageBinding::store).collect();
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let keep = entry
            .file_name()
            .to_str()
            .and_then(|file| {
                SQLITE_FILES
                    .iter()
                    .find_map(|suffix| file.strip_suffix(suffix))
            })
            .is_some_and(|store| named.contains(store));
        if keep {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
