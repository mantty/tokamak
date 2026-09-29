use std::fs;
use std::time::Instant;

use serde_json::{Value as Json, json};

use super::authorizer::TRANSACTION_REFUSED;
use super::statement::{self, Param};
use super::{D1Database, Migrations, is_blank};
use crate::storage::lock;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn database() -> TestResult<(tempfile::TempDir, D1Database)> {
    let directory = tempfile::tempdir()?;
    let database = D1Database::open(&directory.path().join("d1.sqlite"))?;
    Ok((directory, database))
}

/// Run one query through the service and return its first result.
fn query(database: &D1Database, sql: &str) -> TestResult<Json> {
    let (body, _) = database.query(&json!({ "sql": sql }).to_string(), "ROWS_AND_COLUMNS");
    let response: Json = serde_json::from_str(&body)?;
    Ok(response.get(0).cloned().unwrap_or(response))
}

fn rows(database: &D1Database, sql: &str) -> TestResult<Json> {
    let result = query(database, sql)?;
    result["results"]["rows"]
        .as_array()
        .map(|rows| Json::Array(rows.clone()))
        .ok_or_else(|| format!("{sql} failed: {result}").into())
}

fn error(database: &D1Database, sql: &str) -> TestResult<String> {
    let result = query(database, sql)?;
    result["error"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("{sql} succeeded: {result}").into())
}

#[test]
fn supports_every_d1_feature() -> TestResult {
    let (_directory, database) = database()?;
    let setup = [
        "CREATE VIRTUAL TABLE docs USING fts5(body)",
        "CREATE VIRTUAL TABLE terms USING fts5vocab(docs, row)",
        "CREATE VIRTUAL TABLE boxes USING rtree(id, min_x, max_x)",
        "CREATE VIRTUAL TABLE small_boxes USING rtree_i32(id, min_x, max_x)",
        "CREATE TABLE items (id INTEGER PRIMARY KEY, price REAL, doubled REAL GENERATED ALWAYS AS (price * 2))",
        "INSERT INTO docs VALUES ('hello world'), ('goodbye world')",
        "INSERT INTO boxes VALUES (1, 0, 10)",
        "INSERT INTO items (price) VALUES (1), (2), (3)",
    ];
    for sql in setup {
        rows(&database, sql)?;
    }
    let checks = [
        (
            "SELECT body FROM docs WHERE docs MATCH 'hello'",
            json!([["hello world"]]),
        ),
        (
            "SELECT term, doc FROM terms WHERE term = 'world'",
            json!([["world", 2]]),
        ),
        (
            "SELECT id FROM boxes WHERE min_x <= 5 AND max_x >= 5",
            json!([[1]]),
        ),
        (
            "SELECT json_extract('{\"a\":[1,2]}', '$.a[1]'), json(jsonb('[1]'))",
            json!([[2, "[1]"]]),
        ),
        (
            "SELECT round(sqrt(16) + pi(), 2), floor(2.5)",
            json!([[7.14, 2.0]]),
        ),
        (
            "SELECT id, sum(price) OVER (ORDER BY id) FROM items ORDER BY id LIMIT 2",
            json!([[1, 1.0], [2, 3.0]]),
        ),
        ("SELECT doubled FROM items WHERE id = 3", json!([[6.0]])),
        (
            "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n WHERE x < 3) SELECT sum(x) FROM n",
            json!([[6]]),
        ),
    ];
    for (sql, expected) in checks {
        assert_eq!(rows(&database, sql)?, expected, "{sql}");
    }
    Ok(())
}

#[test]
fn supports_d1_statement_forms_and_pragmas() -> TestResult {
    let (_directory, database) = database()?;
    for sql in [
        "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT, extra TEXT)",
        "INSERT INTO t (name) VALUES ('a'), ('b'), ('c') ON CONFLICT DO NOTHING",
        "UPDATE t SET name = 'z' ORDER BY id DESC LIMIT 1",
        "DELETE FROM t ORDER BY id LIMIT 1",
        "ALTER TABLE t RENAME TO renamed",
        "ALTER TABLE renamed DROP COLUMN extra",
        "PRAGMA defer_foreign_keys = ON",
        "PRAGMA foreign_keys = OFF",
        "PRAGMA optimize",
    ] {
        rows(&database, sql)?;
    }
    assert_eq!(
        rows(&database, "SELECT name FROM renamed ORDER BY id")?,
        json!([["b"], ["z"]])
    );
    assert_eq!(
        rows(
            &database,
            "INSERT INTO renamed (name) VALUES ('d') RETURNING id"
        )?,
        json!([[4]])
    );
    assert_eq!(
        rows(&database, "SELECT name FROM pragma_table_info('renamed')")?,
        json!([["id"], ["name"]])
    );
    assert_eq!(
        rows(&database, "PRAGMA foreign_keys")?,
        json!([[1]]),
        "D1 runs each query in a transaction, where foreign_keys cannot change"
    );
    assert!(rows(&database, "PRAGMA table_list")?.as_array().is_some());
    Ok(())
}

#[test]
fn refuses_what_d1_refuses() -> TestResult {
    let (_directory, database) = database()?;
    rows(&database, "CREATE TABLE t (x)")?;
    for sql in [
        "ATTACH DATABASE 'other.sqlite' AS other",
        "CREATE TEMP TABLE scratch (x)",
        "CREATE TEMP VIEW recent AS SELECT 1",
        "CREATE TABLE _cf_reserved (x)",
        "ALTER TABLE t RENAME TO _CF_T",
        "PRAGMA journal_mode = DELETE",
        "PRAGMA page_size = 1024",
        "SELECT sqlite_version()",
        "CREATE VIRTUAL TABLE old USING fts4(body)",
        "CREATE VIRTUAL TABLE series USING generate_series",
    ] {
        let message = error(&database, sql)?;
        assert!(message.starts_with("not authorized"), "{sql}: {message}");
    }
    for sql in [
        "BEGIN",
        "BEGIN TRANSACTION",
        "SAVEPOINT s",
        "RELEASE s",
        "COMMIT",
    ] {
        assert_eq!(error(&database, sql)?, TRANSACTION_REFUSED, "{sql}");
    }
    Ok(())
}

#[test]
fn applies_d1_limits() -> TestResult {
    let (_directory, database) = database()?;
    let columns: Vec<String> = (0..101).map(|index| format!("c{index}")).collect();
    let wide = format!("CREATE TABLE wide ({})", columns.join(", "));
    let compound = ["SELECT 1"; 6].join(" UNION ALL ");
    let placeholders = ["?"; 101].join(", ");

    assert!(error(&database, &wide)?.starts_with("too many columns"));
    assert!(error(&database, &compound)?.starts_with("too many terms in compound SELECT"));
    assert!(
        error(&database, &format!("SELECT {placeholders}"))?.starts_with("too many SQL variables")
    );
    assert!(error(&database, "SELECT zeroblob(2000001)")?.starts_with("string or blob too big"));
    assert!(
        error(&database, &format!("SELECT '{}'", "x".repeat(100_000)))?
            .starts_with("statement too long")
    );
    rows(&database, "SELECT length(zeroblob(2000000))")?;
    Ok(())
}

#[test]
fn interrupts_a_query_past_its_deadline() -> TestResult {
    let (_directory, database) = database()?;
    let session = lock(&database.session);
    *lock(&session.access.deadline) = Some(Instant::now());

    let result = statement::query(
        &session.connection,
        "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n) SELECT count(*) FROM n",
        &[],
    );

    assert_eq!(
        result.err().as_deref(),
        Some("interrupted: SQLITE_INTERRUPT")
    );
    Ok(())
}

#[test]
fn runs_a_batch_in_one_transaction() -> TestResult {
    let (_directory, database) = database()?;
    rows(&database, "CREATE TABLE t (id INTEGER PRIMARY KEY)")?;
    let batch = json!([
        { "sql": "INSERT INTO t VALUES (?)", "params": [1] },
        { "sql": "INSERT INTO t VALUES (?)", "params": [1] },
    ]);

    let (body, _) = database.query(&batch.to_string(), "ROWS_AND_COLUMNS");

    let response: Json = serde_json::from_str(&body)?;
    assert_eq!(response["success"], json!(false));
    assert_eq!(rows(&database, "SELECT count(*) FROM t")?, json!([[0]]));
    Ok(())
}

#[test]
fn reports_results_and_metadata_as_local_d1_does() -> TestResult {
    let (_directory, database) = database()?;
    rows(
        &database,
        "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT, data BLOB)",
    )?;
    let insert =
        json!({ "sql": "INSERT INTO t (name, data) VALUES (?, ?)", "params": ["a", [1, 2]] });

    let (body, _) = database.query(&insert.to_string(), "NONE");
    let (objects, _) = database.query(
        &json!({ "sql": "SELECT * FROM t" }).to_string(),
        "ARRAY_OF_OBJECTS",
    );

    let inserted: Json = serde_json::from_str(&body)?;
    let meta = &inserted[0]["meta"];
    assert_eq!(inserted[0]["success"], json!(true));
    assert_eq!(meta["served_by"], json!("tokamak.db"));
    assert_eq!(meta["changes"], json!(1));
    assert_eq!(meta["last_row_id"], json!(1));
    assert_eq!(meta["changed_db"], json!(true));
    assert_eq!(meta["rows_written"], json!(1));
    assert!(meta["size_after"].as_u64().is_some_and(|size| size > 0));
    assert!(meta["duration"].as_f64().is_some());
    assert!(
        objects.starts_with(
            r#"[{"success":true,"results":[{"id":1,"name":"a","data":[1,2]}],"meta":{"served_by""#
        ),
        "{objects}"
    );
    Ok(())
}

#[test]
fn advances_the_bookmark_only_when_a_transaction_writes() -> TestResult {
    let (_directory, database) = database()?;
    let select = json!({ "sql": "SELECT 1" }).to_string();
    let (_, initial) = database.query(&select, "NONE");
    let (_, read) = database.query(&select, "NONE");
    let (_, created) = database.query(&json!({ "sql": "CREATE TABLE t (x)" }).to_string(), "NONE");
    let (_, failed) = database.query(
        &json!({ "sql": "INSERT INTO missing VALUES (1)" }).to_string(),
        "NONE",
    );

    assert_eq!(initial, "00000000");
    assert_eq!(read, initial);
    assert_eq!(created, "00000001");
    assert_eq!(failed, created);
    Ok(())
}

#[test]
fn rejects_batches_without_statements() -> TestResult {
    let (_directory, database) = database()?;
    let (body, _) = database.query(&json!([{ "sql": "\n  -- nothing" }]).to_string(), "NONE");

    assert_eq!(
        body,
        r#"{"success":false,"error":"No SQL statements detected."}"#
    );
    assert!(is_blank("  -- a\n\t-- b\n"));
    assert!(!is_blank("-- unindented"));
    assert!(!is_blank("  -- a\nSELECT 1"));
    Ok(())
}

#[test]
fn binds_numbers_as_reals_and_arrays_as_blobs() -> TestResult {
    let (_directory, database) = database()?;
    let session = lock(&database.session);

    let bound = statement::query(
        &session.connection,
        "SELECT typeof(?), typeof(?), typeof(?), typeof(?)",
        &[
            Param::Number(1.0),
            Param::Text("1".to_owned()),
            Param::Blob(vec![1]),
            Param::Null,
        ],
    )?;

    assert_eq!(
        serde_json::to_value(&bound.rows)?,
        json!([["real", "text", "blob", "null"]])
    );
    Ok(())
}

fn migrations(directory: &std::path::Path, files: &[(&str, &str)]) -> TestResult<Migrations> {
    let root = directory.join("migrations");
    for (name, sql) in files {
        let path = root.join(name);
        fs::create_dir_all(path.parent().ok_or("migration has no directory")?)?;
        fs::write(path, sql)?;
    }
    Ok(Migrations {
        directory: root,
        names: files.iter().map(|(name, _)| (*name).to_owned()).collect(),
        table: "d1_migrations".to_owned(),
    })
}

#[test]
fn applies_migrations_once_in_order() -> TestResult {
    let (directory, database) = database()?;
    let migrations = migrations(
        directory.path(),
        &[
            ("0001_init.sql", "-- create\nCREATE TABLE t (x);\n"),
            (
                "0002_seed/migration.sql",
                "INSERT INTO t VALUES (1);\nINSERT INTO t VALUES (2);",
            ),
        ],
    )?;

    database.migrate(&migrations)?;
    database.migrate(&migrations)?;

    assert_eq!(rows(&database, "SELECT count(*) FROM t")?, json!([[2]]));
    assert_eq!(
        rows(&database, "SELECT id, name FROM d1_migrations ORDER BY id")?,
        json!([[1, "0001_init.sql"], [2, "0002_seed/migration.sql"]])
    );
    Ok(())
}

#[test]
fn stops_at_a_failed_migration_and_rolls_it_back() -> TestResult {
    let (directory, database) = database()?;
    let migrations = migrations(
        directory.path(),
        &[
            ("0001_init.sql", "CREATE TABLE t (x);"),
            (
                "0002_broken.sql",
                "INSERT INTO t VALUES (1);\nINSERT INTO missing VALUES (1);",
            ),
            ("0003_after.sql", "CREATE TABLE after (x);"),
        ],
    )?;

    let failure = database.migrate(&migrations).err();

    assert_eq!(
        failure.as_deref(),
        Some("Migration 0002_broken.sql failed: no such table: missing: SQLITE_ERROR")
    );
    assert_eq!(rows(&database, "SELECT count(*) FROM t")?, json!([[0]]));
    assert_eq!(
        rows(&database, "SELECT name FROM d1_migrations")?,
        json!([["0001_init.sql"]])
    );
    assert!(error(&database, "SELECT * FROM after")?.starts_with("no such table"));
    Ok(())
}

#[test]
fn migrations_run_under_the_authorizer() -> TestResult {
    let (directory, database) = database()?;
    let migrations = migrations(
        directory.path(),
        &[("0001.sql", "BEGIN;\nCREATE TABLE t (x);\nCOMMIT;")],
    )?;

    let failure = database.migrate(&migrations).err();

    assert_eq!(
        failure,
        Some(format!("Migration 0001.sql failed: {TRANSACTION_REFUSED}"))
    );
    Ok(())
}
