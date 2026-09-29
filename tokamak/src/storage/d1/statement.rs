//! D1 statement execution over SQLite's C API, which exposes the per-statement
//! row counters that workerd's SQLite patch adds.

use std::ffi::{CStr, c_int};
use std::fmt::Write as _;
use std::ptr::{self, NonNull};
use std::slice;

use rusqlite::{Connection, ffi};

/// Output a statement read: workerd's `LIBSQL_STMTSTATUS_ROWS_READ`.
const ROWS_READ: c_int = 1025;
/// Output a statement wrote: workerd's `LIBSQL_STMTSTATUS_ROWS_WRITTEN`.
const ROWS_WRITTEN: c_int = 1026;

/// A value bound to a statement parameter.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Param {
    Null,
    Number(f64),
    Text(String),
    Blob(Vec<u8>),
}

/// A value in a result row.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Value {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

/// The rows the last statement of some SQL returned, and its row counters.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Output {
    pub(super) columns: Vec<String>,
    pub(super) rows: Vec<Vec<Value>>,
    pub(super) rows_read: u64,
    pub(super) rows_written: u64,
}

/// Run `sql` as workerd runs a D1 query: statements before the last run first
/// and take no parameters, and the last is bound to `params`.
pub(super) fn query(
    connection: &Connection,
    sql: &str,
    params: &[Param],
) -> Result<Output, String> {
    let mut remaining = sql;
    loop {
        let (statement, tail) = Statement::prepare(connection, remaining)?;
        let statement = statement.ok_or("SQL code did not contain a statement.")?;
        let tail = tail.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
        if tail.is_empty() {
            return statement.rows(params);
        }
        if statement.parameter_count() != 0 {
            return Err(
                "When executing multiple SQL statements in a single call, only the last \
                        statement can have parameters."
                    .to_owned(),
            );
        }
        statement.run()?;
        remaining = tail;
    }
}

/// Run every statement in `sql` in turn, as `sqlite3_exec` does.
pub(super) fn script(connection: &Connection, sql: &str) -> Result<(), String> {
    let mut remaining = sql;
    while !remaining.is_empty() {
        let (statement, tail) = Statement::prepare(connection, remaining)?;
        if let Some(statement) = statement {
            statement.run()?;
        } else if tail.len() == remaining.len() {
            break;
        }
        remaining = tail;
    }
    Ok(())
}

/// A prepared statement, finalized when dropped.
struct Statement<'c> {
    raw: NonNull<ffi::sqlite3_stmt>,
    connection: &'c Connection,
}

impl<'c> Statement<'c> {
    /// Prepare the first statement in `sql`, returning it (none for only
    /// whitespace or comments) and the SQL after it.
    fn prepare<'s>(
        connection: &'c Connection,
        sql: &'s str,
    ) -> Result<(Option<Self>, &'s str), String> {
        let length = c_int::try_from(sql.len()).map_err(|_| "statement too long".to_owned())?;
        let mut raw = ptr::null_mut();
        let mut tail = ptr::null();
        // SAFETY: `sql` is valid for `length` bytes, and the connection is
        // borrowed for the call.
        let code = unsafe {
            ffi::sqlite3_prepare_v3(
                handle(connection),
                sql.as_ptr().cast(),
                length,
                0,
                &raw mut raw,
                &raw mut tail,
            )
        };
        if code != ffi::SQLITE_OK {
            return Err(failure(connection, code));
        }
        // SAFETY: SQLite sets `tail` within `sql`, after the prepared statement.
        let consumed = unsafe { tail.cast::<u8>().offset_from(sql.as_ptr()) };
        let rest = usize::try_from(consumed)
            .ok()
            .and_then(|consumed| sql.get(consumed..))
            .ok_or("SQLite returned an invalid statement end")?;
        let statement = NonNull::new(raw).map(|raw| Self { raw, connection });
        Ok((statement, rest))
    }

    fn parameter_count(&self) -> usize {
        // SAFETY: `raw` is a live statement.
        let count = unsafe { ffi::sqlite3_bind_parameter_count(self.raw.as_ptr()) };
        usize::try_from(count).unwrap_or(0)
    }

    fn bind(&self, params: &[Param]) -> Result<(), String> {
        if params.len() != self.parameter_count() {
            return Err("Wrong number of parameter bindings for SQL query.".to_owned());
        }
        for (index, param) in (1..).zip(params) {
            let code = bind_param(self.raw, index, param)?;
            if code != ffi::SQLITE_OK {
                return Err(failure(self.connection, code));
            }
        }
        Ok(())
    }

    /// Advance to the next row, returning whether there is one.
    fn step(&self) -> Result<bool, String> {
        // SAFETY: `raw` is a live statement.
        match unsafe { ffi::sqlite3_step(self.raw.as_ptr()) } {
            ffi::SQLITE_ROW => Ok(true),
            ffi::SQLITE_DONE => Ok(false),
            code => Err(failure(self.connection, code)),
        }
    }

    fn run(self) -> Result<(), String> {
        while self.step()? {}
        Ok(())
    }

    fn rows(self, params: &[Param]) -> Result<Output, String> {
        self.bind(params)?;
        // SAFETY: `raw` is a live statement.
        let count = unsafe { ffi::sqlite3_column_count(self.raw.as_ptr()) };
        let columns = (0..count).map(|index| self.column_name(index)).collect();
        let mut rows = Vec::new();
        while self.step()? {
            rows.push((0..count).map(|index| self.value(index)).collect());
        }
        Ok(Output {
            columns,
            rows,
            rows_read: self.status(ROWS_READ),
            rows_written: self.status(ROWS_WRITTEN),
        })
    }

    fn column_name(&self, index: c_int) -> String {
        // SAFETY: `raw` is a live statement and `index` a column of it; SQLite
        // returns a NUL-terminated name or null when out of memory.
        let name = unsafe { ffi::sqlite3_column_name(self.raw.as_ptr(), index) };
        if name.is_null() {
            return String::new();
        }
        // SAFETY: `name` is NUL-terminated and valid until the next call on
        // this column.
        unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned()
    }

    fn value(&self, index: c_int) -> Value {
        let statement = self.raw.as_ptr();
        // SAFETY: `raw` is on a row and `index` is a column of it. Text and
        // blob pointers are valid for the byte counts SQLite reports.
        unsafe {
            match ffi::sqlite3_column_type(statement, index) {
                ffi::SQLITE_INTEGER => Value::Integer(ffi::sqlite3_column_int64(statement, index)),
                ffi::SQLITE_FLOAT => Value::Real(ffi::sqlite3_column_double(statement, index)),
                ffi::SQLITE_TEXT => {
                    let text = ffi::sqlite3_column_text(statement, index);
                    let bytes = column_bytes(text, ffi::sqlite3_column_bytes(statement, index));
                    Value::Text(String::from_utf8_lossy(bytes).into_owned())
                }
                ffi::SQLITE_BLOB => {
                    let blob = ffi::sqlite3_column_blob(statement, index);
                    let bytes =
                        column_bytes(blob.cast(), ffi::sqlite3_column_bytes(statement, index));
                    Value::Blob(bytes.to_vec())
                }
                _ => Value::Null,
            }
        }
    }

    fn status(&self, counter: c_int) -> u64 {
        // SAFETY: `raw` is a live statement.
        let value = unsafe { ffi::sqlite3_stmt_status(self.raw.as_ptr(), counter, 0) };
        u64::try_from(value).unwrap_or(0)
    }
}

impl Drop for Statement<'_> {
    fn drop(&mut self) {
        // SAFETY: `raw` is a live statement, finalized exactly once.
        unsafe { ffi::sqlite3_finalize(self.raw.as_ptr()) };
    }
}

/// Bind `param` to the parameter at `index` of `statement`, returning
/// SQLite's result code.
fn bind_param(
    statement: NonNull<ffi::sqlite3_stmt>,
    index: c_int,
    param: &Param,
) -> Result<c_int, String> {
    let statement = statement.as_ptr();
    let length = match param {
        Param::Text(text) => bind_length(text.len())?,
        Param::Blob(blob) => bind_length(blob.len())?,
        Param::Null | Param::Number(_) => 0,
    };
    let copy = ffi::SQLITE_TRANSIENT();
    // SAFETY: `statement` is a live statement, `index` is a parameter of it,
    // and SQLite copies text and blobs before returning.
    let code = unsafe {
        match param {
            Param::Null => ffi::sqlite3_bind_null(statement, index),
            Param::Number(number) => ffi::sqlite3_bind_double(statement, index, *number),
            Param::Text(text) => {
                ffi::sqlite3_bind_text(statement, index, text.as_ptr().cast(), length, copy)
            }
            Param::Blob(blob) => {
                ffi::sqlite3_bind_blob(statement, index, blob.as_ptr().cast(), length, copy)
            }
        }
    };
    Ok(code)
}

/// The bytes of a text or blob column.
///
/// # Safety
///
/// `data` must be null or valid for `length` bytes while the result is used.
unsafe fn column_bytes<'a>(data: *const u8, length: c_int) -> &'a [u8] {
    let length = usize::try_from(length).unwrap_or(0);
    if data.is_null() || length == 0 {
        return &[];
    }
    // SAFETY: the caller guarantees `data` is valid for `length` bytes.
    unsafe { slice::from_raw_parts(data, length) }
}

fn bind_length(length: usize) -> Result<c_int, String> {
    c_int::try_from(length).map_err(|_| "string or blob too big: SQLITE_TOOBIG".to_owned())
}

fn handle(connection: &Connection) -> *mut ffi::sqlite3 {
    // SAFETY: the handle is used only while `connection` is borrowed.
    unsafe { connection.handle() }
}

/// Describe a failed call as workerd does: SQLite's message, the offset of
/// the error in the SQL, and the result code's name.
fn failure(connection: &Connection, code: c_int) -> String {
    let database = handle(connection);
    // SAFETY: SQLite returns a NUL-terminated message owned by the connection.
    let message = unsafe { CStr::from_ptr(ffi::sqlite3_errmsg(database)) };
    // SAFETY: `database` is a live connection.
    let (offset, extended) = unsafe {
        (
            ffi::sqlite3_error_offset(database),
            ffi::sqlite3_extended_errcode(database),
        )
    };
    describe(&message.to_string_lossy(), offset, code & 0xff, extended)
}

fn describe(message: &str, offset: c_int, code: c_int, extended: c_int) -> String {
    let mut text = message.to_owned();
    if offset != -1 {
        let _ = write!(text, " at offset {offset}");
    }
    let _ = match name(PRIMARY_CODES, code) {
        Some(name) => write!(text, ": {name}"),
        None => write!(text, ": SQLITE_UNKNOWN_ERROR_CODE({code})"),
    };
    if extended != code
        && let Some(name) = name(EXTENDED_CODES, extended)
    {
        let _ = write!(text, " (extended: {name})");
    }
    text
}

fn name(codes: &[(c_int, &'static str)], code: c_int) -> Option<&'static str> {
    codes
        .iter()
        .find_map(|(value, name)| (*value == code).then_some(*name))
}

macro_rules! named {
    ($($code:ident),* $(,)?) => {
        &[$((ffi::$code, stringify!($code))),*]
    };
}

/// The result codes workerd names in its error messages.
const PRIMARY_CODES: &[(c_int, &str)] = named![
    SQLITE_OK,
    SQLITE_ERROR,
    SQLITE_INTERNAL,
    SQLITE_PERM,
    SQLITE_ABORT,
    SQLITE_BUSY,
    SQLITE_LOCKED,
    SQLITE_NOMEM,
    SQLITE_READONLY,
    SQLITE_INTERRUPT,
    SQLITE_IOERR,
    SQLITE_CORRUPT,
    SQLITE_NOTFOUND,
    SQLITE_FULL,
    SQLITE_CANTOPEN,
    SQLITE_PROTOCOL,
    SQLITE_EMPTY,
    SQLITE_SCHEMA,
    SQLITE_TOOBIG,
    SQLITE_CONSTRAINT,
    SQLITE_MISMATCH,
    SQLITE_MISUSE,
    SQLITE_NOLFS,
    SQLITE_AUTH,
    SQLITE_FORMAT,
    SQLITE_RANGE,
    SQLITE_NOTADB,
    SQLITE_NOTICE,
    SQLITE_WARNING,
    SQLITE_ROW,
    SQLITE_DONE,
];

/// The extended result codes workerd names in its error messages.
const EXTENDED_CODES: &[(c_int, &str)] = named![
    SQLITE_ABORT_ROLLBACK,
    SQLITE_AUTH_USER,
    SQLITE_BUSY_RECOVERY,
    SQLITE_BUSY_SNAPSHOT,
    SQLITE_BUSY_TIMEOUT,
    SQLITE_CANTOPEN_CONVPATH,
    SQLITE_CANTOPEN_DIRTYWAL,
    SQLITE_CANTOPEN_FULLPATH,
    SQLITE_CANTOPEN_ISDIR,
    SQLITE_CANTOPEN_NOTEMPDIR,
    SQLITE_CANTOPEN_SYMLINK,
    SQLITE_CONSTRAINT_CHECK,
    SQLITE_CONSTRAINT_COMMITHOOK,
    SQLITE_CONSTRAINT_DATATYPE,
    SQLITE_CONSTRAINT_FOREIGNKEY,
    SQLITE_CONSTRAINT_FUNCTION,
    SQLITE_CONSTRAINT_NOTNULL,
    SQLITE_CONSTRAINT_PINNED,
    SQLITE_CONSTRAINT_PRIMARYKEY,
    SQLITE_CONSTRAINT_ROWID,
    SQLITE_CONSTRAINT_TRIGGER,
    SQLITE_CONSTRAINT_UNIQUE,
    SQLITE_CONSTRAINT_VTAB,
    SQLITE_CORRUPT_INDEX,
    SQLITE_CORRUPT_SEQUENCE,
    SQLITE_CORRUPT_VTAB,
    SQLITE_ERROR_MISSING_COLLSEQ,
    SQLITE_ERROR_RETRY,
    SQLITE_ERROR_SNAPSHOT,
    SQLITE_IOERR_ACCESS,
    SQLITE_IOERR_AUTH,
    SQLITE_IOERR_BEGIN_ATOMIC,
    SQLITE_IOERR_BLOCKED,
    SQLITE_IOERR_CHECKRESERVEDLOCK,
    SQLITE_IOERR_CLOSE,
    SQLITE_IOERR_COMMIT_ATOMIC,
    SQLITE_IOERR_CONVPATH,
    SQLITE_IOERR_CORRUPTFS,
    SQLITE_IOERR_DATA,
    SQLITE_IOERR_DELETE,
    SQLITE_IOERR_DELETE_NOENT,
    SQLITE_IOERR_DIR_CLOSE,
    SQLITE_IOERR_DIR_FSYNC,
    SQLITE_IOERR_FSTAT,
    SQLITE_IOERR_FSYNC,
    SQLITE_IOERR_GETTEMPPATH,
    SQLITE_IOERR_LOCK,
    SQLITE_IOERR_MMAP,
    SQLITE_IOERR_NOMEM,
    SQLITE_IOERR_RDLOCK,
    SQLITE_IOERR_READ,
    SQLITE_IOERR_ROLLBACK_ATOMIC,
    SQLITE_IOERR_SEEK,
    SQLITE_IOERR_SHMLOCK,
    SQLITE_IOERR_SHMMAP,
    SQLITE_IOERR_SHMOPEN,
    SQLITE_IOERR_SHMSIZE,
    SQLITE_IOERR_SHORT_READ,
    SQLITE_IOERR_TRUNCATE,
    SQLITE_IOERR_UNLOCK,
    SQLITE_IOERR_VNODE,
    SQLITE_IOERR_WRITE,
    SQLITE_LOCKED_SHAREDCACHE,
    SQLITE_LOCKED_VTAB,
    SQLITE_NOTICE_RECOVER_ROLLBACK,
    SQLITE_NOTICE_RECOVER_WAL,
    SQLITE_OK_LOAD_PERMANENTLY,
    SQLITE_READONLY_CANTINIT,
    SQLITE_READONLY_CANTLOCK,
    SQLITE_READONLY_DBMOVED,
    SQLITE_READONLY_DIRECTORY,
    SQLITE_READONLY_RECOVERY,
    SQLITE_READONLY_ROLLBACK,
    SQLITE_WARNING_AUTOINDEX,
];

#[cfg(test)]
mod tests {
    use super::{Output, Param, Value, describe, query, script};
    use crate::storage::sqlite;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn database() -> Result<(tempfile::TempDir, rusqlite::Connection), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let connection = sqlite::open(&directory.path().join("d1.sqlite"))?;
        Ok((directory, connection))
    }

    #[test]
    fn returns_the_last_statements_rows_and_counters() -> TestResult {
        let (_directory, connection) = database()?;
        script(
            &connection,
            "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT, data BLOB);",
        )?;

        let inserted = query(
            &connection,
            "INSERT INTO t (name, data) VALUES (?, ?)",
            &[Param::Text("a".to_owned()), Param::Blob(vec![1, 2])],
        )?;
        let rows = query(
            &connection,
            "INSERT INTO t (name) VALUES ('b'); SELECT id, name, data, 1.5 AS real FROM t WHERE id > ?",
            &[Param::Number(0.0)],
        )?;

        assert_eq!(inserted.rows_written, 1);
        assert_eq!(rows.columns, ["id", "name", "data", "real"]);
        assert_eq!(
            rows.rows,
            [
                vec![
                    Value::Integer(1),
                    Value::Text("a".to_owned()),
                    Value::Blob(vec![1, 2]),
                    Value::Real(1.5)
                ],
                vec![
                    Value::Integer(2),
                    Value::Text("b".to_owned()),
                    Value::Null,
                    Value::Real(1.5)
                ],
            ]
        );
        assert_eq!(rows.rows_read, 2);
        Ok(())
    }

    #[test]
    fn refuses_parameters_before_the_last_statement() -> TestResult {
        let (_directory, connection) = database()?;

        let error = query(&connection, "SELECT ?; SELECT 1", &[]).err();
        let missing = query(&connection, "SELECT ?", &[]).err();
        let empty = query(&connection, "  -- nothing", &[]).err();

        assert_eq!(
            error.as_deref(),
            Some(
                "When executing multiple SQL statements in a single call, only the last \
                 statement can have parameters."
            )
        );
        assert_eq!(
            missing.as_deref(),
            Some("Wrong number of parameter bindings for SQL query.")
        );
        assert_eq!(
            empty.as_deref(),
            Some("SQL code did not contain a statement.")
        );
        Ok(())
    }

    #[test]
    fn describes_errors_as_workerd_does() -> TestResult {
        let (_directory, connection) = database()?;
        script(&connection, "CREATE TABLE t (id INTEGER PRIMARY KEY)")?;
        query(&connection, "INSERT INTO t VALUES (1)", &[])?;

        let missing = query(&connection, "SELECT * FROM missing", &[]).err();
        let syntax = query(&connection, "SELECT * FORM t", &[]).err();
        let duplicate = query(&connection, "INSERT INTO t VALUES (1)", &[]).err();

        assert_eq!(
            missing.as_deref(),
            Some("no such table: missing: SQLITE_ERROR")
        );
        assert_eq!(
            syntax.as_deref(),
            Some("near \"FORM\": syntax error at offset 9: SQLITE_ERROR")
        );
        assert_eq!(
            duplicate.as_deref(),
            Some(
                "UNIQUE constraint failed: t.id: SQLITE_CONSTRAINT (extended: \
                 SQLITE_CONSTRAINT_PRIMARYKEY)"
            )
        );
        assert_eq!(
            describe("oops", -1, 99, 99),
            "oops: SQLITE_UNKNOWN_ERROR_CODE(99)"
        );
        Ok(())
    }

    #[test]
    fn runs_scripts_with_comments_between_statements() -> TestResult {
        let (_directory, connection) = database()?;

        script(
            &connection,
            "-- setup\nCREATE TABLE t (x);\n/* seed */ INSERT INTO t VALUES (1);\n-- end",
        )?;

        let rows = query(&connection, "SELECT x FROM t", &[])?;
        assert_eq!(
            rows,
            Output {
                columns: vec!["x".to_owned()],
                rows: vec![vec![Value::Integer(1)]],
                rows_read: 1,
                rows_written: 0,
            }
        );
        Ok(())
    }
}
