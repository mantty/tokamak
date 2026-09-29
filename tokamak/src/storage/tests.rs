use std::fs;
use std::path::Path;

use super::Storage;
use crate::env_vars::StorageBinding;
use crate::packaging::PackageLayout;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn d1(name: &str, id: &str, migrations: &[&str]) -> StorageBinding {
    StorageBinding::D1 {
        name: name.to_owned(),
        id: id.to_owned(),
        migrations_table: "d1_migrations".to_owned(),
        migrations: migrations.iter().map(|name| (*name).to_owned()).collect(),
    }
}

fn touch(root: &Path, files: &[&str]) -> TestResult {
    for file in files {
        let path = root.join(file);
        fs::create_dir_all(path.parent().ok_or("file has no directory")?)?;
        fs::write(path, "")?;
    }
    Ok(())
}

#[test]
fn deletes_stores_no_binding_names() -> TestResult {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    touch(
        root,
        &[
            "d1/kept.sqlite",
            "d1/kept.sqlite-wal",
            "d1/removed.sqlite",
            "d1/removed.sqlite-wal",
            "d1/removed.sqlite-shm",
            "d1/stray.txt",
            "d1/nested/file",
        ],
    )?;

    Storage::open(
        root,
        &PackageLayout::new(root.join("app")),
        &[d1("DB", "kept", &[])],
    )?;

    for kept in ["d1/kept.sqlite", "d1/kept.sqlite-wal"] {
        assert!(root.join(kept).exists(), "{kept}");
    }
    for removed in [
        "d1/removed.sqlite",
        "d1/removed.sqlite-wal",
        "d1/removed.sqlite-shm",
        "d1/stray.txt",
        "d1/nested",
    ] {
        assert!(!root.join(removed).exists(), "{removed}");
    }
    Ok(())
}

#[test]
fn rejects_an_identifier_unsafe_as_a_file_name() -> TestResult {
    let directory = tempfile::tempdir()?;

    let result = Storage::open(
        directory.path(),
        &PackageLayout::new(directory.path()),
        &[d1("DB", "../escape", &[])],
    );

    assert!(result.is_err());
    Ok(())
}

#[test]
fn shares_a_database_between_bindings_naming_it() -> TestResult {
    let directory = tempfile::tempdir()?;
    let storage = Storage::open(
        directory.path(),
        &PackageLayout::new(directory.path()),
        &[d1("DB", "app", &[]), d1("READER", "app", &[])],
    )?;

    storage.d1_query("DB", r#"{"sql":"CREATE TABLE t (x)"}"#, "NONE");
    let (body, bookmark) =
        storage.d1_query("READER", r#"{"sql":"SELECT count(*) FROM t"}"#, "NONE");

    assert!(body.contains(r#""results":[{"count(*)":0}]"#), "{body}");
    assert_eq!(bookmark, "00000001");
    assert_eq!(
        storage.installed(),
        r#"[{"name":"DB","type":"d1"},{"name":"READER","type":"d1"}]"#
    );
    Ok(())
}

#[test]
fn fails_every_query_after_a_failed_migration() -> TestResult {
    let directory = tempfile::tempdir()?;
    let app = PackageLayout::new(directory.path().join("app"));
    fs::create_dir_all(app.d1_migrations("DB"))?;
    fs::write(app.d1_migrations("DB").join("0001.sql"), "CREATE TABLE t (")?;
    let storage = Storage::open(
        &directory.path().join("storage"),
        &app,
        &[d1("DB", "app", &["0001.sql"])],
    )?;

    let (first, _) = storage.d1_query("DB", r#"{"sql":"SELECT 1"}"#, "NONE");
    fs::write(
        app.d1_migrations("DB").join("0001.sql"),
        "CREATE TABLE t (x);",
    )?;
    let (second, _) = storage.d1_query("DB", r#"{"sql":"SELECT 1"}"#, "NONE");

    assert_eq!(first, second);
    assert!(
        first.starts_with(r#"{"success":false,"error":"Migration 0001.sql failed: "#),
        "{first}"
    );
    Ok(())
}

#[test]
fn keeps_data_across_processes() -> TestResult {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let bindings = [d1("DB", "app", &[])];
    let first = Storage::open(root, &PackageLayout::new(root), &bindings)?;
    first.d1_query(
        "DB",
        r#"{"sql":"CREATE TABLE t (x); INSERT INTO t VALUES (1)"}"#,
        "NONE",
    );
    drop(first);

    let second = Storage::open(root, &PackageLayout::new(root), &bindings)?;
    let (body, _) = second.d1_query("DB", r#"{"sql":"SELECT x FROM t"}"#, "NONE");

    assert!(body.contains(r#""results":[{"x":1}]"#), "{body}");
    Ok(())
}
