//! KV namespaces, each a SQLite database of entries.
//!
//! Values are read and written with blob I/O, since a 25 MiB value exceeds
//! the largest allocation the SQLite build makes.

use std::io::{Read, Write};
use std::sync::Mutex;

use rusqlite::blob::ZeroBlob;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::keys::{cursor, cursor_key, prefix_end};
use super::{Location, Open, SQLITE_FILES, lock, sqlite};

/// Entries in key order; `value` is last so records can end in a zero blob.
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS entries (
    key TEXT PRIMARY KEY NOT NULL,
    expiration INTEGER,
    metadata TEXT,
    value BLOB NOT NULL
);
CREATE INDEX IF NOT EXISTS entries_by_expiration ON entries (expiration)
    WHERE expiration IS NOT NULL;
";

/// A KV namespace on its one connection.
#[derive(Debug)]
pub(crate) struct KvNamespace {
    connection: Mutex<Connection>,
}

/// A stored value and the JSON metadata stored with it.
#[derive(Debug, PartialEq)]
pub(crate) struct Entry {
    pub(crate) value: Vec<u8>,
    pub(crate) metadata: Option<String>,
}

/// One page of keys in key order, and the cursor to the next page.
#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct Page {
    pub(crate) keys: Vec<Key>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cursor: Option<String>,
}

/// A key in a listing, with its expiration and JSON metadata.
#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct Key {
    pub(crate) name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) expiration: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) metadata: Option<String>,
}

impl Open for KvNamespace {
    const SUFFIX: &'static str = ".sqlite";
    const FILES: &'static [&'static str] = &SQLITE_FILES;

    fn open(location: &Location) -> Result<Self, String> {
        let connection = sqlite::open(&location.path).map_err(|error| error.to_string())?;
        connection
            .execute_batch(SCHEMA)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }
}

impl KvNamespace {
    /// The unexpired entry for `key` at `now`, in seconds since the epoch.
    pub(crate) fn get(&self, key: &str, now: i64) -> Result<Option<Entry>, String> {
        read(&lock(&self.connection), key, now)
    }

    /// The unexpired entries for `keys`, in the same order.
    pub(crate) fn get_many(&self, keys: &[String], now: i64) -> Result<Vec<Option<Entry>>, String> {
        let connection = lock(&self.connection);
        keys.iter().map(|key| read(&connection, key, now)).collect()
    }

    /// Store `value` under `key`, replacing any entry, and delete the entries
    /// expired at `now`.
    pub(crate) fn put(
        &self,
        key: &str,
        value: &[u8],
        expiration: Option<i64>,
        metadata: Option<&str>,
        now: i64,
    ) -> Result<(), String> {
        let length = i32::try_from(value.len()).map_err(|_| "value too large".to_owned())?;
        let mut connection = lock(&self.connection);
        let transaction = connection.transaction().map_err(text)?;
        transaction
            .execute("DELETE FROM entries WHERE expiration <= ?1", [now])
            .map_err(text)?;
        transaction
            .execute(
                "INSERT OR REPLACE INTO entries (key, expiration, metadata, value) VALUES (?1, ?2, ?3, ?4)",
                params![key, expiration, metadata, ZeroBlob(length)],
            )
            .map_err(text)?;
        let row = transaction.last_insert_rowid();
        transaction
            .blob_open("main", "entries", "value", row, false)
            .map_err(text)?
            .write_all(value)
            .map_err(text)?;
        transaction.commit().map_err(text)
    }

    /// Delete `key`, and the entries expired at `now`.
    pub(crate) fn delete(&self, key: &str, now: i64) -> Result<(), String> {
        let mut connection = lock(&self.connection);
        let transaction = connection.transaction().map_err(text)?;
        transaction
            .execute("DELETE FROM entries WHERE expiration <= ?1", [now])
            .map_err(text)?;
        transaction
            .execute("DELETE FROM entries WHERE key = ?1", [key])
            .map_err(text)?;
        transaction.commit().map_err(text)
    }

    /// Up to `limit` unexpired keys beginning with `prefix`, after the key
    /// `page_cursor` encodes.
    pub(crate) fn list(
        &self,
        prefix: &str,
        page_cursor: &str,
        limit: usize,
        now: i64,
    ) -> Result<Page, String> {
        let after = cursor_key(page_cursor).unwrap_or_default();
        let end = prefix_end(prefix);
        let connection = lock(&self.connection);
        let mut keys = list_keys(&connection, prefix, end.as_deref(), &after, limit, now)
            .map_err(|error| error.to_string())?;
        let next = (keys.len() > limit).then(|| {
            keys.truncate(limit);
            keys.last().map(|key| cursor(&key.name))
        });
        Ok(Page {
            keys,
            cursor: next.flatten(),
        })
    }
}

fn read(connection: &Connection, key: &str, now: i64) -> Result<Option<Entry>, String> {
    let found = connection
        .query_row(
            "SELECT rowid, metadata FROM entries WHERE key = ?1 AND (expiration IS NULL OR expiration > ?2)",
            params![key, now],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()
        .map_err(text)?;
    let Some((row, metadata)) = found else {
        return Ok(None);
    };
    let mut blob = connection
        .blob_open("main", "entries", "value", row, true)
        .map_err(text)?;
    let mut value = Vec::with_capacity(blob.len());
    blob.read_to_end(&mut value).map_err(text)?;
    Ok(Some(Entry { value, metadata }))
}

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// Unexpired keys from `prefix` up to `end`, after `after`.
const LIST_BETWEEN: &str = "SELECT key, expiration, metadata FROM entries
    WHERE key >= ?1 AND key > ?2 AND (expiration IS NULL OR expiration > ?3) AND key < ?5
    ORDER BY key LIMIT ?4";

/// Unexpired keys from `prefix`, after `after`.
const LIST_FROM: &str = "SELECT key, expiration, metadata FROM entries
    WHERE key >= ?1 AND key > ?2 AND (expiration IS NULL OR expiration > ?3)
    ORDER BY key LIMIT ?4";

/// One more key than `limit`, so the caller knows whether a page follows.
fn list_keys(
    connection: &Connection,
    prefix: &str,
    end: Option<&str>,
    after: &str,
    limit: usize,
    now: i64,
) -> rusqlite::Result<Vec<Key>> {
    let limit = i64::try_from(limit).unwrap_or(i64::MAX).saturating_add(1);
    let key = |row: &rusqlite::Row<'_>| {
        Ok(Key {
            name: row.get(0)?,
            expiration: row.get(1)?,
            metadata: row.get(2)?,
        })
    };
    match end {
        Some(end) => connection
            .prepare(LIST_BETWEEN)?
            .query_map(params![prefix, after, now, limit, end], key)?
            .collect(),
        None => connection
            .prepare(LIST_FROM)?
            .query_map(params![prefix, after, now, limit], key)?
            .collect(),
    }
}

#[cfg(test)]
mod tests;
