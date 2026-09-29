//! SQLite connections for device stores.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

/// Open the store database at `path`, creating it when absent.
///
/// A store has one connection. Exclusive locking, set before WAL mode, keeps
/// the WAL index in memory, so SQLite creates no shared-memory file. Every
/// commit is synced to disk before it returns.
pub(super) fn open(path: &Path) -> rusqlite::Result<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.pragma_update(None, "locking_mode", "EXCLUSIVE")?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    Ok(connection)
}

#[cfg(test)]
mod tests {
    use super::open;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn opens_a_durable_wal_database_without_shared_memory() -> TestResult {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("store.sqlite");
        let connection = open(&path)?;
        connection.execute_batch("CREATE TABLE t (x); INSERT INTO t VALUES (1);")?;

        let mode: String = connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
        let synchronous: i64 =
            connection.pragma_query_value(None, "synchronous", |row| row.get(0))?;
        assert_eq!(mode, "wal");
        assert_eq!(synchronous, 2);
        assert!(directory.path().join("store.sqlite-wal").is_file());
        assert!(!directory.path().join("store.sqlite-shm").exists());
        drop(connection);

        let reopened = open(&path)?;
        let count: i64 = reopened.query_row("SELECT count(*) FROM t", [], |row| row.get(0))?;
        assert_eq!(count, 1);
        Ok(())
    }
}
