//! D1 databases: the service a `D1Database` binding sends queries to, run
//! over SQLite as local D1 runs them.

mod authorizer;
mod statement;

use std::cell::Cell;
use std::collections::BTreeSet;
use std::ffi::c_int;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rusqlite::config::DbConfig;
use rusqlite::hooks::{AuthContext, Authorization};
use rusqlite::limits::Limit;
use rusqlite::{Connection, TransactionState};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};

use super::{lock, sqlite};
use authorizer::Decision;
use statement::{Output, Param, Value};

/// The longest a query runs before it is interrupted.
const QUERY_TIME_LIMIT: Duration = Duration::from_secs(30);

/// Virtual machine instructions between checks of the query time limit.
const TIME_LIMIT_CHECK_INTERVAL: c_int = 10_000;

/// Per-connection limits: workerd's, with D1's documented string, blob and
/// row size in place of workerd's larger one.
const LIMITS: [(Limit, i32); 12] = [
    (Limit::SQLITE_LIMIT_LENGTH, 2_000_000),
    (Limit::SQLITE_LIMIT_SQL_LENGTH, 100_000),
    (Limit::SQLITE_LIMIT_COLUMN, 100),
    (Limit::SQLITE_LIMIT_EXPR_DEPTH, 100),
    (Limit::SQLITE_LIMIT_COMPOUND_SELECT, 5),
    (Limit::SQLITE_LIMIT_VDBE_OP, 25_000),
    (Limit::SQLITE_LIMIT_FUNCTION_ARG, 127),
    (Limit::SQLITE_LIMIT_ATTACHED, 0),
    (Limit::SQLITE_LIMIT_LIKE_PATTERN_LENGTH, 50),
    (Limit::SQLITE_LIMIT_VARIABLE_NUMBER, 100),
    (Limit::SQLITE_LIMIT_TRIGGER_DEPTH, 10),
    (Limit::SQLITE_LIMIT_WORKER_THREADS, 0),
];

/// The `served_by` value in query metadata.
const SERVED_BY: &str = "tokamak.db";

/// A D1 binding's packaged migrations.
#[derive(Clone, Debug)]
pub(crate) struct Migrations {
    /// The binding's packaged migrations directory.
    pub(crate) directory: PathBuf,
    /// Migrations, relative to `directory`, in the order they apply.
    pub(crate) names: Vec<String>,
    /// Table recording applied migrations.
    pub(crate) table: String,
}

/// A D1 database on its one connection.
#[derive(Debug)]
pub(crate) struct D1Database {
    session: Mutex<Session>,
}

#[derive(Debug)]
struct Session {
    connection: Connection,
    access: Arc<Access>,
    /// The database's `user_version`: the count of committed write
    /// transactions, which bookmarks show.
    version: Cell<i32>,
}

/// What the authorizer and progress handler share with the session.
#[derive(Debug, Default)]
struct Access {
    /// Set while tokamak's own statements run.
    trusted: AtomicBool,
    /// The message for the last refusal that has one.
    refusal: Mutex<Option<&'static str>>,
    /// When the running query is interrupted.
    deadline: Mutex<Option<Instant>>,
}

impl D1Database {
    /// Open the database at `path`, creating it when absent.
    pub(crate) fn open(path: &Path) -> Result<Self, String> {
        let open = || -> rusqlite::Result<Session> {
            let connection = sqlite::open(path)?;
            let version = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
            let access = Arc::new(Access::default());
            secure(&connection, &access)?;
            Ok(Session {
                connection,
                access,
                version: Cell::new(version),
            })
        };
        let session = open().map_err(|error| error.to_string())?;
        Ok(Self {
            session: Mutex::new(session),
        })
    }

    /// Answer a request to the D1 service's `/query` or `/execute` endpoint.
    ///
    /// `body` is the JSON of one query or several, which run in one
    /// transaction. Returns the response body and the database's bookmark.
    pub(crate) fn query(&self, body: &str, format: &str) -> (String, String) {
        let format = Format::parse(format);
        let request = serde_json::from_str::<Request>(body).map_err(|error| error.to_string());
        let (response, bookmark) = {
            let session = lock(&self.session);
            let response =
                request.and_then(|request| session.batch(request.into_queries(), format));
            (response, session.bookmark())
        };
        let body = match response {
            Ok(results) => serde_json::to_string(&results)
                .unwrap_or_else(|error| failure_body(&error.to_string())),
            Err(error) => failure_body(&error),
        };
        (body, bookmark)
    }

    /// Apply the migrations not yet recorded in their table, in order, each
    /// in its own transaction, as Wrangler applies them.
    pub(crate) fn migrate(&self, migrations: &Migrations) -> Result<(), String> {
        if migrations.names.is_empty() {
            return Ok(());
        }
        let session = lock(&self.session);
        let table = quote_identifier(&migrations.table);
        session.transaction(|session| {
            session.untrusted(|connection| {
                statement::query(connection, &create_migrations_table(&table), &[])
            })
        })?;
        let applied = session.applied_migrations(&table)?;
        for name in &migrations.names {
            if !applied.contains(name) {
                session
                    .apply_migration(&migrations.directory, name, &table)
                    .map_err(|error| format!("Migration {name} failed: {error}"))?;
            }
        }
        Ok(())
    }
}

/// The body of a failed D1 service response.
pub(crate) fn failure_body(error: &str) -> String {
    let failure = Failure {
        success: false,
        error,
    };
    serde_json::to_string(&failure).unwrap_or_else(|_| {
        r#"{"success":false,"error":"D1 failed to describe an error"}"#.to_owned()
    })
}

/// Set workerd's protections: defensive mode, limits, the authorizer and the
/// query time limit.
fn secure(connection: &Connection, access: &Arc<Access>) -> rusqlite::Result<()> {
    connection.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    for (limit, value) in LIMITS {
        connection.set_limit(limit, value)?;
    }
    let authorizer = Arc::clone(access);
    connection.authorizer(Some(move |context: AuthContext<'_>| {
        authorizer.authorize(&context)
    }))?;
    let deadline = Arc::clone(access);
    connection.progress_handler(TIME_LIMIT_CHECK_INTERVAL, Some(move || deadline.expired()))
}

impl Access {
    fn authorize(&self, context: &AuthContext<'_>) -> Authorization {
        if self.trusted.load(Ordering::Relaxed) {
            return Authorization::Allow;
        }
        match authorizer::authorize(context) {
            Decision::Allow => Authorization::Allow,
            Decision::Deny => Authorization::Deny,
            Decision::Refuse(message) => {
                *lock(&self.refusal) = Some(message);
                Authorization::Deny
            }
        }
    }

    fn expired(&self) -> bool {
        lock(&self.deadline).is_some_and(|deadline| Instant::now() >= deadline)
    }
}

impl Session {
    fn batch(&self, queries: Vec<Query>, format: Format) -> Result<Vec<QueryResult>, String> {
        let queries: Vec<Query> = queries
            .into_iter()
            .filter(|query| !is_blank(&query.sql))
            .collect();
        if queries.is_empty() {
            return Err("No SQL statements detected.".to_owned());
        }
        self.transaction(|session| {
            queries
                .iter()
                .map(|query| session.run(query, format))
                .collect()
        })
    }

    fn run(&self, query: &Query, format: Format) -> Result<QueryResult, String> {
        let params = query
            .params
            .iter()
            .flatten()
            .map(param)
            .collect::<Result<Vec<_>, _>>()?;
        let size_before = self.size()?;
        let changes_before = self.connection.total_changes();
        let last_row_id_before = self.connection.last_insert_rowid();
        let started = Instant::now();
        let output =
            self.untrusted(|connection| statement::query(connection, &query.sql, &params))?;
        let duration = started.elapsed().as_secs_f64() * 1000.0;
        let size_after = self.size()?;
        let changes = self.connection.total_changes() - changes_before;
        let last_row_id = self.connection.last_insert_rowid();
        Ok(QueryResult {
            success: true,
            meta: Meta {
                served_by: SERVED_BY,
                duration,
                changes,
                last_row_id,
                changed_db: changes != 0
                    || last_row_id != last_row_id_before
                    || size_after != size_before,
                size_after,
                rows_read: output.rows_read,
                rows_written: output.rows_written,
            },
            results: Results { format, output },
        })
    }

    /// Run `work` in a transaction, committing when it succeeds.
    fn transaction<T>(&self, work: impl FnOnce(&Self) -> Result<T, String>) -> Result<T, String> {
        self.trusted_batch("BEGIN")?;
        let result = work(self).and_then(|value| self.commit().map(|()| value));
        if result.is_err() && !self.connection.is_autocommit() {
            let _ = self.trusted_batch("ROLLBACK");
        }
        result
    }

    /// Commit, advancing the version when the transaction wrote.
    fn commit(&self) -> Result<(), String> {
        let wrote =
            self.connection.transaction_state(Some("main")).ok() == Some(TransactionState::Write);
        let version = if wrote {
            self.version.get().wrapping_add(1)
        } else {
            self.version.get()
        };
        if wrote {
            self.trusted(|connection| connection.pragma_update(None, "user_version", version))
                .map_err(|error| error.to_string())?;
        }
        self.trusted_batch("COMMIT")?;
        self.version.set(version);
        Ok(())
    }

    /// A bookmark: the version as hexadecimal, which sorts as it increases.
    fn bookmark(&self) -> String {
        format!("{:08x}", self.version.get().cast_unsigned())
    }

    fn size(&self) -> Result<u64, String> {
        let bytes: i64 = self
            .trusted(|connection| {
                connection.query_row(
                    "SELECT page_count * page_size FROM pragma_page_count(), pragma_page_size()",
                    [],
                    |row| row.get(0),
                )
            })
            .map_err(|error| error.to_string())?;
        Ok(u64::try_from(bytes).unwrap_or(0))
    }

    fn applied_migrations(&self, table: &str) -> Result<BTreeSet<String>, String> {
        let applied = self.untrusted(|connection| {
            statement::query(connection, &format!("SELECT name FROM {table}"), &[])
        })?;
        Ok(applied
            .rows
            .into_iter()
            .flatten()
            .filter_map(|value| match value {
                Value::Text(name) => Some(name),
                _ => None,
            })
            .collect())
    }

    /// Apply the migration `name` in `directory` and record it in `table`, as
    /// one transaction.
    fn apply_migration(&self, directory: &Path, name: &str, table: &str) -> Result<(), String> {
        let sql = fs::read_to_string(directory.join(name)).map_err(|error| error.to_string())?;
        let script = format!(
            "{sql}\nINSERT INTO {table} (name)\nvalues ('{}');",
            name.replace('\'', "''")
        );
        self.transaction(|session| {
            session.untrusted(|connection| statement::script(connection, &script))
        })
    }

    fn trusted_batch(&self, sql: &str) -> Result<(), String> {
        self.trusted(|connection| connection.execute_batch(sql))
            .map_err(|error| error.to_string())
    }

    fn trusted<T>(&self, work: impl FnOnce(&Connection) -> T) -> T {
        self.access.trusted.store(true, Ordering::Relaxed);
        let result = work(&self.connection);
        self.access.trusted.store(false, Ordering::Relaxed);
        result
    }

    /// Run application SQL under the authorizer and the query time limit.
    fn untrusted<T>(
        &self,
        work: impl FnOnce(&Connection) -> Result<T, String>,
    ) -> Result<T, String> {
        lock(&self.access.refusal).take();
        *lock(&self.access.deadline) = Some(Instant::now() + QUERY_TIME_LIMIT);
        let result = work(&self.connection);
        *lock(&self.access.deadline) = None;
        let refusal = lock(&self.access.refusal).take();
        result.map_err(|error| refusal.map_or(error, str::to_owned))
    }
}

/// Wrangler's migrations table.
fn create_migrations_table(table: &str) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS {table}(\n\t\tid         INTEGER PRIMARY KEY AUTOINCREMENT,\n\t\tname       TEXT UNIQUE,\n\t\tapplied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP NOT NULL\n);"
    )
}

fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Whether `sql` is empty once indented `--` comment lines are removed, as
/// Miniflare's D1 service skips queries (`/^\s+--.*/gm`).
fn is_blank(sql: &str) -> bool {
    let mut rest = sql;
    let mut line_start = true;
    while let Some(character) = rest.chars().next() {
        let indent = rest.len() - rest.trim_start_matches(is_space).len();
        if line_start && indent > 0 && rest[indent..].starts_with("--") {
            let comment = &rest[indent..];
            rest = &comment[comment.find(is_line_terminator).unwrap_or(comment.len())..];
            line_start = false;
            continue;
        }
        if !is_space(character) {
            return false;
        }
        line_start = is_line_terminator(character);
        rest = &rest[character.len_utf8()..];
    }
    true
}

/// JavaScript's `\s`.
fn is_space(character: char) -> bool {
    matches!(
        character,
        '\t' | '\n' | '\x0b' | '\x0c' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

fn is_line_terminator(character: char) -> bool {
    matches!(character, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// A statement parameter from its JSON form: blobs are arrays of bytes.
fn param(value: &serde_json::Value) -> Result<Param, String> {
    match value {
        serde_json::Value::Null => Ok(Param::Null),
        serde_json::Value::Number(number) => number
            .as_f64()
            .map(Param::Number)
            .ok_or_else(|| format!("Type 'number' not supported for value '{number}'")),
        serde_json::Value::String(text) => Ok(Param::Text(text.clone())),
        serde_json::Value::Array(bytes) => bytes
            .iter()
            .map(|byte| byte.as_u64().and_then(|byte| u8::try_from(byte).ok()))
            .collect::<Option<Vec<u8>>>()
            .map(Param::Blob)
            .ok_or_else(|| "Type 'object' not supported for value 'array'".to_owned()),
        other => Err(format!(
            "Type '{}' not supported for value '{other}'",
            json_type(other)
        )),
    }
}

fn json_type(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Bool(_) => "boolean",
        _ => "object",
    }
}

/// One query or several.
#[derive(Deserialize)]
#[serde(untagged)]
enum Request {
    Batch(Vec<Query>),
    Single(Query),
}

impl Request {
    fn into_queries(self) -> Vec<Query> {
        match self {
            Self::Batch(queries) => queries,
            Self::Single(query) => vec![query],
        }
    }
}

#[derive(Deserialize)]
struct Query {
    sql: String,
    #[serde(default)]
    params: Option<Vec<serde_json::Value>>,
}

/// How result rows are returned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Format {
    /// An object per row, also used for `NONE` and unknown formats.
    ArrayOfObjects,
    /// Column names and an array per row.
    RowsAndColumns,
}

impl Format {
    fn parse(format: &str) -> Self {
        if format == "ROWS_AND_COLUMNS" {
            Self::RowsAndColumns
        } else {
            Self::ArrayOfObjects
        }
    }
}

#[derive(Serialize)]
struct QueryResult {
    success: bool,
    results: Results,
    meta: Meta,
}

#[derive(Serialize)]
struct Failure<'a> {
    success: bool,
    error: &'a str,
}

#[derive(Serialize)]
struct Meta {
    served_by: &'static str,
    duration: f64,
    changes: u64,
    last_row_id: i64,
    changed_db: bool,
    size_after: u64,
    rows_read: u64,
    rows_written: u64,
}

struct Results {
    format: Format,
    output: Output,
}

impl Serialize for Results {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let Output { columns, rows, .. } = &self.output;
        match self.format {
            Format::RowsAndColumns => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("columns", columns)?;
                map.serialize_entry("rows", rows)?;
                map.end()
            }
            Format::ArrayOfObjects => {
                serializer.collect_seq(rows.iter().map(|row| RowObject { columns, row }))
            }
        }
    }
}

/// A row as an object whose keys follow the column order.
struct RowObject<'a> {
    columns: &'a [String],
    row: &'a [Value],
}

impl Serialize for RowObject<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.columns.iter().zip(self.row))
    }
}

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Null => serializer.serialize_unit(),
            Self::Integer(integer) => serializer.serialize_i64(*integer),
            Self::Real(real) => serializer.serialize_f64(*real),
            Self::Text(text) => serializer.serialize_str(text),
            Self::Blob(bytes) => serializer.collect_seq(bytes),
        }
    }
}

#[cfg(test)]
mod tests;
