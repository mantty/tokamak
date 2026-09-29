//! The storage bindings packaged into an app, with D1 migrations.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};
use tokamak::{
    PackageLayout, StorageBinding, WranglerConfig, WranglerMigrations, WranglerStorage,
    store_id_problem,
};
use walkdir::WalkDir;

use super::support::{copy_file, glob_matches, slash_path};

/// The storage bindings `wrangler` declares, after copying each D1
/// database's migrations into `layout`.
pub(crate) fn package(
    wrangler: &WranglerConfig,
    layout: &PackageLayout,
) -> Result<Vec<StorageBinding>> {
    validate(wrangler)?;
    wrangler
        .storage
        .iter()
        .map(|binding| package_binding(binding, layout))
        .collect()
}

fn package_binding(binding: &WranglerStorage, layout: &PackageLayout) -> Result<StorageBinding> {
    Ok(match binding {
        WranglerStorage::Kv { name, id } => StorageBinding::Kv {
            name: name.clone(),
            id: id.clone(),
        },
        WranglerStorage::D1 {
            name,
            id,
            migrations,
        } => StorageBinding::D1 {
            name: name.clone(),
            id: id.clone(),
            migrations_table: migrations.table.clone(),
            migrations: package_migrations(name, migrations, layout)?,
        },
        WranglerStorage::R2 { name, id } => StorageBinding::R2 {
            name: name.clone(),
            id: id.clone(),
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

/// Binding names must be unique, and a kind's store identifiers must be
/// file names that no file system confuses.
fn validate(wrangler: &WranglerConfig) -> Result<()> {
    let mut names: BTreeSet<&str> = wrangler.vars.keys().map(String::as_str).collect();
    names.extend(
        wrangler
            .assets
            .as_ref()
            .map(|assets| assets.binding.as_str()),
    );
    let mut stores: BTreeMap<(&str, String), &str> = BTreeMap::new();
    for binding in &wrangler.storage {
        let (kind, name, store) = (binding.kind(), binding.name(), binding.store());
        if !names.insert(name) {
            bail!("more than one binding is named {name}");
        }
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
    use std::fs;
    use std::path::Path;

    use tokamak::{PackageLayout, StorageBinding, WranglerMigrations};

    use super::{compare_migrations, leading_number, migration_names, package};

    type TestResult = anyhow::Result<()>;

    fn wrangler(root: &Path, config: &str) -> anyhow::Result<tokamak::WranglerConfig> {
        let path = root.join("wrangler.json");
        fs::write(&path, config)?;
        Ok(tokamak::load_wrangler_config(&path)?)
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
        assert!(compare_migrations("1/b.sql", "1/a.sql").is_gt());
        assert!(compare_migrations("1", "1/a.sql").is_lt());
        assert_eq!(leading_number(" 0042_x"), Some(42));
        assert_eq!(leading_number("-3"), Some(-3));
        assert_eq!(leading_number("+7x"), Some(7));
        assert_eq!(leading_number("init"), None);
        assert_eq!(leading_number("-"), None);
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

        let bindings = package(&config, &layout)?;

        assert_eq!(
            bindings,
            [
                StorageBinding::Kv {
                    name: "SESSION".to_owned(),
                    id: "SESSION".to_owned(),
                },
                StorageBinding::D1 {
                    name: "DB".to_owned(),
                    id: "db".to_owned(),
                    migrations_table: "d1_migrations".to_owned(),
                    migrations: vec![
                        "0000_b/migration.sql".to_owned(),
                        "0001_a/migration.sql".to_owned()
                    ],
                },
                StorageBinding::R2 {
                    name: "FILES".to_owned(),
                    id: "files".to_owned(),
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
    fn rejects_conflicting_names_and_unsafe_stores() -> TestResult {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let layout = PackageLayout::new(root.join("app"));
        for (config, problem) in [
            (
                r#""vars": { "DB": "x" }, "d1_databases": [{ "binding": "DB" }]"#,
                "more than one binding is named DB",
            ),
            (
                r#""d1_databases": [{ "binding": "A", "database_id": "../a" }]"#,
                "names the store `../a`",
            ),
            (
                r#""d1_databases": [{ "binding": "A", "database_id": "app" }, { "binding": "B", "database_id": "App" }]"#,
                "differ only in case",
            ),
            (
                r#""kv_namespaces": [{ "binding": "A", "id": "x" }], "d1_databases": [{ "binding": "A", "database_id": "y" }]"#,
                "more than one binding is named A",
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
            let error = package(&config, &layout)
                .err()
                .ok_or_else(|| anyhow::anyhow!("accepted {problem}"))?;
            assert!(error.to_string().contains(problem), "{error}");
        }
        Ok(())
    }
}
