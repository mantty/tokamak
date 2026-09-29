//! Listing a bucket's objects in key order, with the keys that share a
//! delimited prefix rolled up into that prefix.

use rusqlite::{Connection, Statement};
use serde::{Deserialize, Serialize};

use super::super::keys::{cursor, cursor_key, prefix_end};
use super::{Failure, Object, R2Bucket, lock, object_columns};

/// Objects from a key onwards, in key order.
const OBJECTS_FROM: &str = concat!(
    "SELECT ",
    object_columns!(),
    " FROM objects WHERE key >= ?1 ORDER BY key"
);

/// Objects listed without `limit`.
const DEFAULT_LIMIT: usize = 1000;
/// Objects listed at most with metadata.
const METADATA_LIMIT: usize = 100;

/// What a listing asks for.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListRequest {
    limit: Option<usize>,
    #[serde(default)]
    prefix: String,
    cursor: Option<String>,
    delimiter: Option<String>,
    start_after: Option<String>,
    #[serde(default)]
    include: Vec<Include>,
}

/// Metadata a listing includes.
#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum Include {
    HttpMetadata,
    CustomMetadata,
}

/// A page of a listing.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Listing {
    pub(crate) objects: Vec<Object>,
    pub(crate) truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cursor: Option<String>,
    pub(crate) delimited_prefixes: Vec<String>,
}

/// An object, or a delimited prefix.
enum Entry {
    Object(Object),
    Prefix(String),
}

/// Where a scan starts: at `key`, or just after it when `exclusive`.
struct Start {
    key: String,
    exclusive: bool,
}

impl R2Bucket {
    /// A page of the objects and delimited prefixes the request selects.
    pub(crate) fn list(&self, request: &ListRequest) -> Result<Listing, Failure> {
        let limit = request.limit()?;
        let connection = lock(&self.connection);
        let mut entries = request.entries(&connection, limit + 1)?;
        let mut listing = Listing {
            truncated: entries.len() > limit,
            ..Listing::default()
        };
        entries.truncate(limit);
        if let Some(last) = entries.last().filter(|_| listing.truncated) {
            listing.cursor = Some(cursor(&last.last_key(&connection)?));
        }
        for entry in entries {
            match entry {
                Entry::Object(object) => listing.objects.push(object),
                Entry::Prefix(prefix) => listing.delimited_prefixes.push(prefix),
            }
        }
        Ok(listing)
    }
}

impl ListRequest {
    fn limit(&self) -> Result<usize, Failure> {
        let limit = match self.limit {
            None => DEFAULT_LIMIT,
            Some(limit @ 1..=DEFAULT_LIMIT) => limit,
            Some(_) => return Err(Failure::InvalidMaxKeys),
        };
        Ok(if self.include.is_empty() {
            limit
        } else {
            limit.min(METADATA_LIMIT)
        })
    }

    /// Up to `count` entries in key order. The scan seeks past each
    /// delimited prefix, so it reads one row for each entry.
    fn entries(&self, connection: &Connection, count: usize) -> rusqlite::Result<Vec<Entry>> {
        let mut statement = connection.prepare(OBJECTS_FROM)?;
        let mut entries = Vec::new();
        let mut start = Some(self.start());
        while let Some(from) = start.take() {
            start = self.scan(&mut statement, &from, count, &mut entries)?;
        }
        Ok(entries)
    }

    /// Add entries from `start` up to `count`, returning where the scan
    /// continues after a delimited prefix.
    fn scan(
        &self,
        statement: &mut Statement<'_>,
        start: &Start,
        count: usize,
        entries: &mut Vec<Entry>,
    ) -> rusqlite::Result<Option<Start>> {
        let mut rows = statement.query([&start.key])?;
        while let Some(row) = rows.next()? {
            let key: String = row.get(0)?;
            if entries.len() == count || !key.starts_with(&self.prefix) {
                return Ok(None);
            }
            if start.exclusive && key == start.key {
                continue;
            }
            let Some(prefix) = self.delimited_prefix(&key) else {
                let http = self.includes(&Include::HttpMetadata);
                let custom = self.includes(&Include::CustomMetadata);
                entries.push(Entry::Object(Object::from_row(row, http, custom)?));
                continue;
            };
            let next = prefix_end(&prefix).map(|key| Start {
                key,
                exclusive: false,
            });
            entries.push(Entry::Prefix(prefix));
            return Ok(next);
        }
        Ok(None)
    }

    /// The first key the listing may include: the prefix, or just after the
    /// later of `startAfter` and the key the cursor follows.
    fn start(&self) -> Start {
        let after = [
            self.start_after.clone(),
            self.cursor.as_deref().and_then(cursor_key),
        ]
        .into_iter()
        .flatten()
        .max();
        match after {
            Some(key) if key >= self.prefix => Start {
                key,
                exclusive: true,
            },
            _ => Start {
                key: self.prefix.clone(),
                exclusive: false,
            },
        }
    }

    /// The delimited prefix `key` rolls up into: the key up to and including
    /// the first delimiter after the listing's prefix.
    fn delimited_prefix(&self, key: &str) -> Option<String> {
        let delimiter = self
            .delimiter
            .as_deref()
            .filter(|delimiter| !delimiter.is_empty())?;
        let end = self.prefix.len() + key[self.prefix.len()..].find(delimiter)? + delimiter.len();
        Some(key[..end].to_owned())
    }

    fn includes(&self, metadata: &Include) -> bool {
        self.include.contains(metadata)
    }
}

impl Entry {
    /// The last key the entry covers, which the next page follows.
    fn last_key(&self, connection: &Connection) -> rusqlite::Result<String> {
        match self {
            Self::Object(object) => Ok(object.key.clone()),
            Self::Prefix(prefix) => last_key(connection, prefix),
        }
    }
}

/// The greatest key beginning with `prefix`, which some key does.
fn last_key(connection: &Connection, prefix: &str) -> rusqlite::Result<String> {
    match prefix_end(prefix) {
        Some(end) => connection
            .prepare("SELECT max(key) FROM objects WHERE key >= ?1 AND key < ?2")?
            .query_row([prefix, &end], |row| row.get(0)),
        None => connection
            .prepare("SELECT max(key) FROM objects WHERE key >= ?1")?
            .query_row([prefix], |row| row.get(0)),
    }
}
