//! KV namespaces, each a SQLite database of entries. A namespace refuses
//! what the KV service refuses, with its HTTP status and message.
//!
//! Values are read and written with blob I/O, since a 25 MiB value exceeds
//! the largest allocation the SQLite build makes.

use std::io::{Read, Write};
use std::sync::Mutex;

use rusqlite::blob::ZeroBlob;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::keys::{cursor, cursor_key, prefix_end};
use super::{Location, Open, SQLITE_FILES, lock, sqlite, text};

// Lengths are in UTF-8 bytes, but a bulk read's total is in UTF-16 units of
// its values decoded as text, and is at most one value's length.
const MAX_KEY_LENGTH: usize = 512;
const MAX_VALUE_LENGTH: usize = 25 * 1024 * 1024;
const MAX_METADATA_LENGTH: usize = 1024;
const MAX_LIST_KEYS: i64 = 1000;
const MAX_BULK_KEYS: usize = 100;
// Times are in seconds.
const MIN_CACHE_TTL: i64 = 30;
const MIN_EXPIRATION_TTL: i64 = 60;

/// A value written in chunks, whose bytes are kept only within the limit.
#[derive(Debug, Default)]
pub(crate) struct Value {
    bytes: Vec<u8>,
    length: usize,
}

impl Value {
    pub(crate) fn write(&mut self, chunk: &[u8]) {
        self.length += chunk.len();
        if self.length > MAX_VALUE_LENGTH {
            self.bytes = Vec::new();
        } else {
            self.bytes.extend_from_slice(chunk);
        }
    }
}

/// What `put` asks of a value besides its key and bytes: a TTL, or else an
/// expiration in seconds since the epoch, and JSON metadata.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PutOptions {
    pub(crate) expiration_ttl: Option<i64>,
    pub(crate) expiration: Option<i64>,
    pub(crate) metadata: Option<String>,
}

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
        let connection = sqlite::open(&location.path).map_err(text)?;
        connection.execute_batch(SCHEMA).map_err(text)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }
}

impl KvNamespace {
    /// The unexpired entry for `key` at `now`, in seconds since the epoch.
    pub(crate) fn get(
        &self,
        key: &str,
        cache_ttl: Option<i64>,
        now: i64,
    ) -> Result<Option<Entry>, String> {
        validate_key(key)?;
        validate_cache_ttl(cache_ttl)?;
        read(&lock(&self.connection), key, now)
    }

    /// The unexpired entries for `keys`, in the same order.
    pub(crate) fn get_many(
        &self,
        keys: &[String],
        cache_ttl: Option<i64>,
        now: i64,
    ) -> Result<Vec<Option<Entry>>, String> {
        check(keys.len() <= MAX_BULK_KEYS, 400, || {
            format!("You can request a maximum of {MAX_BULK_KEYS} keys")
        })?;
        check(!keys.is_empty(), 400, || {
            "You must request a minimum of 1 key".to_owned()
        })?;
        for key in keys {
            validate_bulk_key(key)?;
            validate_cache_ttl(cache_ttl)?;
        }
        let connection = lock(&self.connection);
        let reads = keys.iter().map(|key| read(&connection, key, now));
        let entries: Vec<_> = reads.collect::<Result<_, _>>()?;
        let text_length =
            |entry: &Entry| String::from_utf8_lossy(&entry.value).encode_utf16().count();
        let lengths = entries.iter().flatten().map(text_length);
        check(lengths.sum::<usize>() <= MAX_VALUE_LENGTH, 413, || {
            "Total size of request exceeds the limit of 25MB".to_owned()
        })?;
        Ok(entries)
    }

    /// Store `value` under `key`, replacing any entry, and delete the entries
    /// expired at `now`.
    pub(crate) fn put(
        &self,
        key: &str,
        value: &Value,
        options: &PutOptions,
        now: i64,
    ) -> Result<(), String> {
        validate_key(key)?;
        let expiration = expiration(options, now)?;
        let metadata = options.metadata.as_deref().map_or(0, str::len);
        check(metadata <= MAX_METADATA_LENGTH, 413, || {
            format!("Metadata length of {metadata} exceeds limit of {MAX_METADATA_LENGTH}.")
        })?;
        let length = value.length;
        check(length <= MAX_VALUE_LENGTH, 413, || {
            format!("Value length of {length} exceeds limit of {MAX_VALUE_LENGTH}.")
        })?;
        let length = i32::try_from(length).map_err(text)?;
        let mut connection = lock(&self.connection);
        let transaction = connection.transaction().map_err(text)?;
        transaction
            .execute("DELETE FROM entries WHERE expiration <= ?1", [now])
            .map_err(text)?;
        transaction
            .execute(
                "INSERT OR REPLACE INTO entries (key, expiration, metadata, value) VALUES (?1, ?2, ?3, ?4)",
                params![key, expiration, options.metadata, ZeroBlob(length)],
            )
            .map_err(text)?;
        let row = transaction.last_insert_rowid();
        transaction
            .blob_open("main", "entries", "value", row, false)
            .map_err(text)?
            .write_all(&value.bytes)
            .map_err(text)?;
        transaction.commit().map_err(text)
    }

    /// Delete `key`, and the entries expired at `now`.
    pub(crate) fn delete(&self, key: &str, now: i64) -> Result<(), String> {
        validate_key(key)?;
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
    /// `page_cursor` encodes. A limit below one asks for a full page.
    pub(crate) fn list(
        &self,
        prefix: &str,
        page_cursor: &str,
        limit: i64,
        now: i64,
    ) -> Result<Page, String> {
        let limit = if limit > 0 { limit } else { MAX_LIST_KEYS };
        check(limit <= MAX_LIST_KEYS, 400, || {
            format!(
                "Invalid key_count_limit of {limit}. Please specify an integer less than {MAX_LIST_KEYS}."
            )
        })?;
        validate_key(prefix)?;
        let limit = usize::try_from(limit).map_err(text)?;
        let after = cursor_key(page_cursor).unwrap_or_default();
        let end = prefix_end(prefix);
        let connection = lock(&self.connection);
        let mut keys =
            list_keys(&connection, prefix, end.as_deref(), &after, limit, now).map_err(text)?;
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

/// Nothing when `valid`, else the service's refusal: its HTTP status and
/// message.
fn check(valid: bool, status: u16, message: impl FnOnce() -> String) -> Result<(), String> {
    if valid {
        Ok(())
    } else {
        Err(format!("{status} {}", message()))
    }
}

fn validate_key(key: &str) -> Result<(), String> {
    let length = key.len();
    check(length <= MAX_KEY_LENGTH, 414, || {
        format!("UTF-8 encoded length of {length} exceeds key length limit of {MAX_KEY_LENGTH}.")
    })
}

/// A key a bulk read names must also be a usable name.
fn validate_bulk_key(key: &str) -> Result<(), String> {
    check(!key.is_empty(), 400, || {
        "Key names must not be empty".to_owned()
    })?;
    check(key != "." && key != "..", 400, || {
        format!("Illegal key name \"{key}\". Please use a different name.")
    })?;
    validate_key(key)
}

fn validate_cache_ttl(cache_ttl: Option<i64>) -> Result<(), String> {
    let ttl = cache_ttl.unwrap_or(MIN_CACHE_TTL);
    check(ttl >= MIN_CACHE_TTL, 400, || {
        format!("Invalid cache_ttl of {ttl}. Cache TTL must be at least {MIN_CACHE_TTL}.")
    })
}

/// The expiration `options` ask for at `now`: a TTL, or else a time.
fn expiration(options: &PutOptions, now: i64) -> Result<Option<i64>, String> {
    if let Some(ttl) = options.expiration_ttl {
        let invalid = format!("Invalid expiration_ttl of {ttl}.");
        check(ttl > 0, 400, || {
            format!("{invalid} Please specify integer greater than 0.")
        })?;
        check(ttl >= MIN_EXPIRATION_TTL, 400, || {
            format!("{invalid} Expiration TTL must be at least {MIN_EXPIRATION_TTL}.")
        })?;
        return Ok(Some(now + ttl));
    }
    if let Some(at) = options.expiration {
        let invalid = format!("Invalid expiration of {at}.");
        check(at > now, 400, || {
            format!(
                "{invalid} Please specify integer greater than the current number of seconds since the UNIX epoch."
            )
        })?;
        check(at >= now + MIN_EXPIRATION_TTL, 400, || {
            format!(
                "{invalid} Expiration times must be at least {MIN_EXPIRATION_TTL} seconds in the future."
            )
        })?;
    }
    Ok(options.expiration)
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
