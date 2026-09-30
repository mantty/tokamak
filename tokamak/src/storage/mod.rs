//! The stores behind a packaged app's storage bindings, in the app's private
//! data directory.
//!
//! The rest of the runtime reaches it only through [`ENTRY`], and its
//! JavaScript only through `bindings.mjs`.

mod d1;
mod host;
mod keys;
mod kv;
mod r2;
mod sqlite;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use rquickjs::{Ctx, Module};
use serde::Serialize;

use crate::env_vars::{StorageBinding, store_id_problem};
use crate::linked::{OpenedStorage, StorageEntry, StorageRuntime};
use crate::packaging::PackageLayout;
use d1::{D1Database, Migrations};
use kv::KvNamespace;
use r2::R2Bucket;

use host::{HostModule, StorageHandle};

include!(concat!(env!("OUT_DIR"), "/storage_builtins.rs"));

/// The storage part's entry point, exported as [`crate::STORAGE_ENTRY_POINT`].
#[unsafe(export_name = "tokamak_storage")]
static ENTRY: StorageEntry = StorageEntry { open };

/// The stores `bindings` name, as the runtime uses them.
fn open(
    directory: &Path,
    scratch: &Path,
    app: &PackageLayout,
    bindings: &[StorageBinding],
) -> OpenedStorage {
    Ok(Arc::new(Storage::open(directory, scratch, app, bindings)?))
}

/// Directory of each kind of store.
const KV: &str = "kv";
const D1: &str = "d1";
const R2: &str = "r2";

/// Files of a store kept in one SQLite database.
const SQLITE_FILES: [&str; 4] = [".sqlite", ".sqlite-wal", ".sqlite-shm", ".sqlite-journal"];

/// The stores a packaged app's bindings name, each opened on first use.
#[derive(Debug)]
pub(crate) struct Storage {
    bindings: BTreeMap<String, Binding>,
    /// The bindings for the Worker's `env`, as JSON for the bootstrap.
    installed: String,
}

#[derive(Debug)]
enum Binding {
    Kv(Arc<Store<KvNamespace>>),
    D1 {
        database: Arc<Store<D1Database>>,
        migrations: Migrations,
        /// The outcome of applying `migrations` in this process.
        migrated: OnceLock<Result<(), String>>,
    },
    R2(Arc<Store<R2Bucket>>),
}

/// A store opened from its location.
trait Open: Sized {
    /// The suffix that follows a store's ID in the name of its location.
    const SUFFIX: &'static str;
    /// The suffixes that follow a store's ID in the names of its entries in
    /// its kind's directory.
    const FILES: &'static [&'static str];

    fn open(location: &Location) -> Result<Self, String>;
}

/// Where a store keeps its data, and a directory for its temporary files,
/// which device backups leave out.
#[derive(Debug)]
struct Location {
    path: PathBuf,
    scratch: PathBuf,
}

/// A store's location and, once used, the store.
#[derive(Debug)]
struct Store<T> {
    location: Location,
    opened: Mutex<Option<Arc<T>>>,
}

/// A binding the Worker's `env` receives.
#[derive(Serialize)]
struct Installed<'a> {
    name: &'a str,
    #[serde(rename = "type")]
    kind: &'static str,
}

impl Storage {
    /// The stores `bindings` name in `directory`, with their temporary files
    /// in `scratch`, after deleting every store in either that no binding
    /// names.
    pub(crate) fn open(
        directory: &Path,
        scratch: &Path,
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
        let mut namespaces = Stores::open(directory, scratch, KV, bindings)?;
        let mut databases = Stores::open(directory, scratch, D1, bindings)?;
        let mut buckets = Stores::open(directory, scratch, R2, bindings)?;
        let mut entries = BTreeMap::new();
        for binding in bindings {
            let entry = match binding {
                StorageBinding::Kv { id, .. } => Binding::Kv(namespaces.get(id)),
                StorageBinding::R2 { id, .. } => Binding::R2(buckets.get(id)),
                StorageBinding::D1 {
                    name,
                    id,
                    migrations_table,
                    migrations,
                } => Binding::D1 {
                    database: databases.get(id),
                    migrations: Migrations {
                        directory: app.d1_migrations(name),
                        names: migrations.clone(),
                        table: migrations_table.clone(),
                    },
                    migrated: OnceLock::new(),
                },
            };
            entries.insert(binding.name().to_owned(), entry);
        }
        let installed: Vec<Installed<'_>> = entries
            .iter()
            .map(|(name, binding)| Installed {
                name,
                kind: match binding {
                    Binding::Kv(_) => KV,
                    Binding::D1 { .. } => D1,
                    Binding::R2(_) => R2,
                },
            })
            .collect();
        let installed = serde_json::to_string(&installed).map_err(io::Error::other)?;
        Ok(Self {
            bindings: entries,
            installed,
        })
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
        let Some(Binding::D1 {
            database,
            migrations,
            migrated,
        }) = self.bindings.get(name)
        else {
            return Err(format!("{name} is not a D1 binding"));
        };
        let database = database.get()?;
        migrated
            .get_or_init(|| database.migrate(migrations))
            .clone()?;
        Ok(database)
    }

    /// The KV namespace the binding `name` names.
    pub(crate) fn kv(&self, name: &str) -> Result<Arc<KvNamespace>, String> {
        let Some(Binding::Kv(namespace)) = self.bindings.get(name) else {
            return Err(format!("{name} is not a KV binding"));
        };
        namespace.get()
    }

    /// The R2 bucket the binding `name` names.
    pub(crate) fn r2(&self, name: &str) -> Result<Arc<R2Bucket>, String> {
        let Some(Binding::R2(bucket)) = self.bindings.get(name) else {
            return Err(format!("{name} is not an R2 binding"));
        };
        bucket.get()
    }
}

impl StorageRuntime for Storage {
    fn attach(self: Arc<Self>, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        let bindings = ctx.json_parse(self.installed.as_str())?;
        ctx.globals().set("__tokamak_storage", bindings)?;
        ctx.store_userdata(StorageHandle(self))?;
        Ok(())
    }

    fn module<'js>(&self, ctx: &Ctx<'js>, name: &str) -> Option<rquickjs::Result<Module<'js>>> {
        if name == "tokamak:storage" {
            return Some(Module::declare_def::<HostModule, _>(ctx.clone(), name));
        }
        let bytecode = crate::compat::find_bytecode(STORAGE_BUILTINS, name)?;
        // Builtin bytecode is compiled with the runtime during its build.
        Some(unsafe { Module::load(ctx.clone(), bytecode) })
    }
}

/// The stores of one kind, each shared by the bindings that name it.
struct Stores<T> {
    directory: PathBuf,
    scratch: PathBuf,
    by_id: BTreeMap<String, Arc<Store<T>>>,
}

impl<T: Open> Stores<T> {
    /// The stores of `kind`, after deleting every one that none of
    /// `bindings` names.
    fn open(
        directory: &Path,
        scratch: &Path,
        kind: &'static str,
        bindings: &[StorageBinding],
    ) -> io::Result<Self> {
        let stores = Self {
            directory: directory.join(kind),
            scratch: scratch.join(kind),
            by_id: BTreeMap::new(),
        };
        let named: BTreeSet<&str> = bindings
            .iter()
            .filter(|binding| kind_of(binding) == kind)
            .map(StorageBinding::store)
            .collect();
        remove_entries(&stores.directory, |name| {
            T::FILES
                .iter()
                .filter_map(|suffix| name.strip_suffix(suffix))
                .any(|store| named.contains(store))
        })?;
        remove_entries(&stores.scratch, |name| named.contains(name))?;
        Ok(stores)
    }

    fn get(&mut self, id: &str) -> Arc<Store<T>> {
        let location = Location {
            path: self.directory.join(format!("{id}{}", T::SUFFIX)),
            scratch: self.scratch.join(id),
        };
        Arc::clone(self.by_id.entry(id.to_owned()).or_insert_with(|| {
            Arc::new(Store {
                location,
                opened: Mutex::new(None),
            })
        }))
    }
}

impl<T: Open> Store<T> {
    /// The open store, opening it when this is its first use.
    fn get(&self) -> Result<Arc<T>, String> {
        let mut opened = lock(&self.opened);
        if let Some(store) = opened.as_ref() {
            return Ok(Arc::clone(store));
        }
        if let Some(directory) = self.location.path.parent() {
            fs::create_dir_all(directory).map_err(text)?;
        }
        let store = Arc::new(T::open(&self.location)?);
        *opened = Some(Arc::clone(&store));
        Ok(store)
    }
}

fn kind_of(binding: &StorageBinding) -> &'static str {
    match binding {
        StorageBinding::Kv { .. } => KV,
        StorageBinding::D1 { .. } => D1,
        StorageBinding::R2 { .. } => R2,
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// Delete every entry of `directory` whose name `keep` rejects.
fn remove_entries(directory: &Path, keep: impl Fn(&str) -> bool) -> io::Result<()> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        if entry.file_name().to_str().is_some_and(&keep) {
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
