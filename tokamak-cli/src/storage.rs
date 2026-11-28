//! The storage bindings packaged into an app, with D1 migrations.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use tokamak::{Backup, PackageLayout, StorageBinding, store_id_problem};
use tokamak_cli::copy_file;
use walkdir::WalkDir;

use super::paths::slash_path;
use super::tokamak_config::{BindingConfig, BindingKind};
use super::wrangler_config::{WranglerConfig, WranglerMigrations, WranglerStorage, glob_matches};

/// The app's `bindings` option: tokamak's settings for storage bindings, by
/// Wrangler binding name.
pub(crate) type BindingSettings = BTreeMap<String, BindingConfig>;

/// Check that `binding_settings` suit the storage bindings in `workers`, and
/// give each store one `backup` value.
pub(crate) fn check<'a>(
    workers: impl IntoIterator<Item = &'a WranglerConfig>,
    binding_settings: &BindingSettings,
) -> Result<()> {
    let mut backups = BTreeMap::new();
    for binding in workers.into_iter().flat_map(|wrangler| &wrangler.storage) {
        let backup = backup(binding, binding_settings)?;
        let (kind, store) = (binding.kind(), binding.store());
        if backups
            .insert((kind, store), backup)
            .is_some_and(|other| other != backup)
        {
            bail!("{kind} store `{store}` is bound with different `backup` values");
        }
    }
    Ok(())
}

/// The `backup` value `binding_settings` give the store `binding` names:
/// the default without an entry of the binding's name.
fn backup(binding: &WranglerStorage, binding_settings: &BindingSettings) -> Result<Backup> {
    let (kind, name) = (binding.kind(), binding.name());
    let Some(config) = binding_settings.get(name) else {
        return Ok(Backup::default());
    };
    let matches = matches!(
        (binding, config.kind),
        (WranglerStorage::Kv { .. }, BindingKind::Kv)
            | (WranglerStorage::D1 { .. }, BindingKind::D1)
            | (WranglerStorage::R2 { .. }, BindingKind::R2)
    );
    if !matches {
        bail!(
            "`bindings.{name}` of the tokamak() options has another `type` than the {kind} binding of that name"
        );
    }
    Ok(config.backup)
}

/// The storage bindings `wrangler` declares with the `backup` values of
/// `binding_settings`, after copying each D1 database's migrations into `layout`.
pub(crate) fn package(
    wrangler: &WranglerConfig,
    binding_settings: &BindingSettings,
    layout: &PackageLayout,
) -> Result<Vec<StorageBinding>> {
    validate(wrangler)?;
    wrangler
        .storage
        .iter()
        .map(|binding| package_binding(binding, backup(binding, binding_settings)?, layout))
        .collect()
}

fn package_binding(
    binding: &WranglerStorage,
    backup: Backup,
    layout: &PackageLayout,
) -> Result<StorageBinding> {
    Ok(match binding {
        WranglerStorage::Kv { name, id } => StorageBinding::Kv {
            name: name.clone(),
            id: id.clone(),
            backup,
        },
        WranglerStorage::D1 {
            name,
            id,
            migrations,
        } => StorageBinding::D1 {
            name: name.clone(),
            id: id.clone(),
            backup,
            migrations_table: migrations.table.clone(),
            migrations: package_migrations(name, migrations, layout)?,
        },
        WranglerStorage::R2 { name, id } => StorageBinding::R2 {
            name: name.clone(),
            id: id.clone(),
            backup,
        },
    })
}

/// Copy a D1 binding's migrations into `layout`, returning their names in
/// the order they apply.
fn package_migrations(
    name: &str,
    migrations: &WranglerMigrations,
    layout: &PackageLayout,
) -> Result<Vec<String>> {
    let names = migration_names(migrations)
        .with_context(|| format!("find the migrations of D1 binding {name}"))?;
    for migration in &names {
        copy_file(
            migrations.directory.join(migration),
            layout.d1_migrations(name).join(migration),
        )?;
    }
    Ok(names)
}

/// A kind's store identifiers must be file names that no file system
/// confuses.
fn validate(wrangler: &WranglerConfig) -> Result<()> {
    let mut stores: BTreeMap<(&str, String), &str> = BTreeMap::new();
    for binding in &wrangler.storage {
        let (kind, name, store) = (binding.kind(), binding.name(), binding.store());
        if let Some(problem) = store_id_problem(store) {
            bail!("{kind} binding {name} names the store `{store}`, which {problem}");
        }
        if let Some(other) = stores.insert((kind, store.to_ascii_lowercase()), store)
            && other != store
        {
            bail!(
                "{kind} stores `{other}` and `{store}` differ only in case, which some file systems cannot tell apart"
            );
        }
    }
    Ok(())
}

/// The migration files Wrangler applies, relative to their directory, in
/// Wrangler's order: by leading number, then by name, segment by segment.
fn migration_names(migrations: &WranglerMigrations) -> Result<Vec<String>> {
    if !migrations.directory.is_dir() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in WalkDir::new(&migrations.directory).min_depth(1) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let name = slash_path(entry.path().strip_prefix(&migrations.directory)?)?;
        if !name.split('/').any(|segment| segment.starts_with('.'))
            && glob_matches(&migrations.pattern, &name)
        {
            names.push(name);
        }
    }
    names.sort_by(|left, right| compare_migrations(left, right));
    Ok(names)
}

fn compare_migrations(left: &str, right: &str) -> Ordering {
    left.split('/')
        .zip(right.split('/'))
        .map(|(left, right)| compare_segments(left, right))
        .find(|order| order.is_ne())
        .unwrap_or_else(|| left.split('/').count().cmp(&right.split('/').count()))
}

fn compare_segments(left: &str, right: &str) -> Ordering {
    match (leading_number(left), leading_number(right)) {
        (Some(left_number), Some(right_number)) if left_number != right_number => {
            left_number.cmp(&right_number)
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        _ => left.cmp(right),
    }
}

/// The integer a segment starts with, read as `parseInt` reads it.
fn leading_number(segment: &str) -> Option<i128> {
    let segment = segment.trim_start();
    let sign = usize::from(segment.starts_with(['-', '+']));
    let digits = segment[sign..]
        .find(|character: char| !character.is_ascii_digit())
        .map_or(segment.len(), |end| sign + end);
    segment[..digits].parse().ok()
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;
    use std::fs;
    use std::path::Path;

    use tokamak::{Backup, PackageLayout, StorageBinding};

    use super::{
        BindingSettings, check, compare_migrations, leading_number, migration_names, package,
    };
    use crate::tokamak_config::{BindingConfig, BindingKind};
    use crate::wrangler_config::{self, WranglerConfig, WranglerMigrations};

    type TestResult = anyhow::Result<()>;

    /// The `bindings` option with an entry of each name, kind and backup.
    fn settings(entries: &[(&str, BindingKind, Backup)]) -> BindingSettings {
        entries
            .iter()
            .map(|&(name, kind, backup)| (name.to_owned(), BindingConfig { kind, backup }))
            .collect()
    }

    fn wrangler(root: &Path, config: &str) -> anyhow::Result<WranglerConfig> {
        let path = root.join("wrangler.json");
        fs::write(&path, config)?;
        wrangler_config::load_config(&path)
    }

    fn touch(root: &Path, files: &[&str]) -> TestResult {
        for file in files {
            let path = root.join(file);
            fs::create_dir_all(path.parent().ok_or_else(|| anyhow::anyhow!("no parent"))?)?;
            fs::write(path, format!("-- {file}"))?;
        }
        Ok(())
    }

    #[test]
    fn orders_migrations_as_wrangler_does() -> TestResult {
        let directory = tempfile::tempdir()?;
        touch(
            directory.path(),
            &[
                "10_late.sql",
                "2_early.sql",
                "0002_same.sql",
                "notes.sql",
                "readme.md",
                ".hidden.sql",
                "nested/1.sql",
            ],
        )?;

        let names = migration_names(&WranglerMigrations {
            directory: directory.path().to_owned(),
            pattern: "*.sql".to_owned(),
            table: "d1_migrations".to_owned(),
        })?;

        assert_eq!(
            names,
            ["0002_same.sql", "2_early.sql", "10_late.sql", "notes.sql"]
        );
        assert_eq!(
            [
                compare_migrations("1/b.sql", "1/a.sql"),
                compare_migrations("1", "1/a.sql"),
            ],
            [Ordering::Greater, Ordering::Less]
        );
        assert_eq!(
            [" 0042_x", "-3", "+7x", "init", "-"].map(leading_number),
            [Some(42), Some(-3), Some(7), None, None]
        );
        Ok(())
    }

    #[test]
    fn packages_bindings_and_nested_migrations() -> TestResult {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        touch(
            root,
            &[
                "drizzle/0001_a/migration.sql",
                "drizzle/0000_b/migration.sql",
                "drizzle/meta/_journal.json",
            ],
        )?;
        let config = wrangler(
            root,
            r#"{ "name": "app", "main": "worker.js",
                 "kv_namespaces": [{ "binding": "SESSION" }],
                 "r2_buckets": [{ "binding": "FILES", "bucket_name": "files" }],
                 "d1_databases": [{ "binding": "DB", "database_id": "db", "migrations_dir": "drizzle", "migrations_pattern": "drizzle/*/migration.sql" }] }"#,
        )?;
        let layout = PackageLayout::new(root.join("app"));

        let binding_settings = settings(&[
            ("SESSION", BindingKind::Kv, Backup::Excluded),
            ("DB", BindingKind::D1, Backup::Included),
        ]);

        let bindings = package(&config, &binding_settings, &layout)?;

        assert_eq!(
            bindings,
            [
                StorageBinding::Kv {
                    name: "SESSION".to_owned(),
                    id: "SESSION".to_owned(),
                    backup: Backup::Excluded,
                },
                StorageBinding::D1 {
                    name: "DB".to_owned(),
                    id: "db".to_owned(),
                    backup: Backup::Included,
                    migrations_table: "d1_migrations".to_owned(),
                    migrations: vec![
                        "0000_b/migration.sql".to_owned(),
                        "0001_a/migration.sql".to_owned()
                    ],
                },
                StorageBinding::R2 {
                    name: "FILES".to_owned(),
                    id: "files".to_owned(),
                    backup: Backup::Included,
                }
            ]
        );
        assert_eq!(
            fs::read_to_string(layout.d1_migrations("DB").join("0001_a/migration.sql"))?,
            "-- drizzle/0001_a/migration.sql"
        );
        Ok(())
    }

    #[test]
    fn rejects_unsafe_and_case_conflicting_stores() -> TestResult {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let layout = PackageLayout::new(root.join("app"));
        for (config, problem) in [
            (
                r#""d1_databases": [{ "binding": "A", "database_id": "../a" }]"#,
                "names the store `../a`",
            ),
            (
                r#""d1_databases": [{ "binding": "A", "database_id": "app" }, { "binding": "B", "database_id": "App" }]"#,
                "differ only in case",
            ),
            (
                r#""r2_buckets": [{ "binding": "A", "bucket_name": "files" }, { "binding": "B", "bucket_name": "Files" }]"#,
                "r2_buckets stores `files` and `Files` differ only in case",
            ),
        ] {
            let config = wrangler(
                root,
                &format!(r#"{{ "name": "app", "main": "worker.js", {config} }}"#),
            )?;
            let error = package(&config, &BindingSettings::new(), &layout)
                .err()
                .ok_or_else(|| anyhow::anyhow!("accepted {problem}"))?;
            assert!(error.to_string().contains(problem), "{error}");
        }
        Ok(())
    }

    #[test]
    fn rejects_settings_of_another_type_or_two_backups_for_a_store() -> TestResult {
        let directory = tempfile::tempdir()?;
        let config = wrangler(
            directory.path(),
            r#"{ "name": "app", "main": "worker.js",
                 "kv_namespaces": [{ "binding": "CACHE", "id": "cache" }, { "binding": "COPY", "id": "cache" }] }"#,
        )?;
        let kv = |backup| ("CACHE", BindingKind::Kv, backup);
        for (entries, problem) in [
            (
                vec![("CACHE", BindingKind::D1, Backup::Included)],
                "`bindings.CACHE` of the tokamak() options has another `type`",
            ),
            (
                vec![
                    kv(Backup::Excluded),
                    ("COPY", BindingKind::Kv, Backup::Included),
                ],
                "kv_namespaces store `cache` is bound with different `backup` values",
            ),
        ] {
            let error = check([&config], &settings(&entries))
                .err()
                .ok_or_else(|| anyhow::anyhow!("accepted {problem}"))?;
            assert!(error.to_string().contains(problem), "{error}");
        }
        let both = [
            kv(Backup::Excluded),
            ("COPY", BindingKind::Kv, Backup::Excluded),
        ];
        check([&config], &settings(&both))?;
        check([&config], &BindingSettings::new())
    }
}
