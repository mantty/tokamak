//! R2 buckets. Each object's body is a file named by the object's version,
//! and its metadata a row in the bucket's SQLite database.
//!
//! A write syncs its new body before committing the row that names it, and
//! removes the body it replaces after, so a crash leaves only files that no
//! row names. Opening a bucket removes them.

mod body;
mod conditional;
mod failure;
mod list;
mod multipart;
mod range;

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::value::RawValue;

use super::{Location, Open, lock, remove_entries, sqlite, text};
pub(crate) use body::{Body, BodyReader, BodyWriter, Checksum};
use conditional::Conditional;
pub(crate) use failure::Failure;
use range::{Bounds, Range, header_range};

/// Objects by key, uploads in progress or finished, and the parts uploaded
/// to uploads in progress.
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS objects (
    key TEXT PRIMARY KEY NOT NULL,
    version TEXT NOT NULL,
    size INTEGER NOT NULL,
    etag TEXT NOT NULL,
    uploaded INTEGER NOT NULL,
    checksums TEXT NOT NULL,
    http_metadata TEXT NOT NULL,
    custom_metadata TEXT NOT NULL,
    storage_class TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS uploads (
    id TEXT PRIMARY KEY NOT NULL,
    key TEXT NOT NULL,
    http_metadata TEXT NOT NULL,
    custom_metadata TEXT NOT NULL,
    storage_class TEXT NOT NULL,
    state INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS parts (
    upload TEXT NOT NULL REFERENCES uploads (id),
    number INTEGER NOT NULL,
    file TEXT NOT NULL,
    size INTEGER NOT NULL,
    etag TEXT NOT NULL,
    md5 TEXT NOT NULL,
    PRIMARY KEY (upload, number)
);
";

/// An object row's columns, in the order `Object::from_row` reads them.
macro_rules! object_columns {
    () => {
        "key, version, size, etag, uploaded, checksums, http_metadata, custom_metadata, storage_class"
    };
}
pub(super) use object_columns;

const FIND_OBJECT: &str = concat!("SELECT ", object_columns!(), " FROM objects WHERE key = ?1");

const RECORD_OBJECT: &str = concat!(
    "INSERT OR REPLACE INTO objects (",
    object_columns!(),
    ") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
);

/// The largest object: 5 GiB less 5 MiB.
const MAX_OBJECT_SIZE: u64 = 5_363_466_240;
/// The longest key, in UTF-8 bytes.
const MAX_KEY_LENGTH: usize = 1024;
/// The largest custom metadata, as `serialized_length` measures it.
const MAX_METADATA_SIZE: usize = 2048;

/// An R2 bucket on its one connection.
#[derive(Debug)]
pub(crate) struct R2Bucket {
    connection: Mutex<Connection>,
    /// Object bodies, each named by its version.
    objects: PathBuf,
    /// Bodies of uploaded parts.
    parts: PathBuf,
}

/// JSON text, serialized as the value it encodes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Json(String);

impl Serialize for Json {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let value: &RawValue = serde_json::from_str(&self.0).map_err(serde::ser::Error::custom)?;
        value.serialize(serializer)
    }
}

impl Json {
    fn empty_object() -> Self {
        Self("{}".to_owned())
    }
}

impl<'de> Deserialize<'de> for Json {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Box::<RawValue>::deserialize(deserializer).map(|value| Self(value.get().to_owned()))
    }
}

/// An object's metadata, and the range of its body a read returns.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Object {
    pub(crate) key: String,
    pub(crate) version: String,
    pub(crate) size: u64,
    pub(crate) etag: String,
    /// Milliseconds since the epoch.
    pub(crate) uploaded: i64,
    pub(crate) checksums: Json,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) http_metadata: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) custom_metadata: Option<Json>,
    pub(crate) storage_class: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) range: Option<Range>,
}

/// The metadata a write gives an object.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ObjectMetadata {
    #[serde(default = "Json::empty_object")]
    http_metadata: Json,
    #[serde(default = "Json::empty_object")]
    custom_metadata: Json,
    #[serde(default = "standard_storage_class")]
    storage_class: String,
}

/// What a read asks for: conditions, and a range as bounds or as a `Range`
/// header's value.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GetRequest {
    only_if: Option<Conditional>,
    range: Option<Bounds>,
    range_header: Option<String>,
}

/// How a write records an object: with conditions, and its metadata.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PutRequest {
    only_if: Option<Conditional>,
    metadata: ObjectMetadata,
}

/// An object a read finds.
#[derive(Debug)]
pub(crate) enum Read {
    /// The object, whose metadata fails the read's conditions.
    Unmet(Object),
    /// The object, and its body in the range read.
    Body(Object, BodyReader),
}

impl Open for R2Bucket {
    const SUFFIX: &'static str = "";
    const FILES: &'static [&'static str] = &[""];

    fn open(location: &Location) -> Result<Self, String> {
        Self::open_in(&location.path, &location.scratch).map_err(text)
    }
}

impl R2Bucket {
    /// The bucket in `directory`, keeping part bodies in `parts`, after
    /// removing the bodies no row names.
    fn open_in(directory: &Path, parts: &Path) -> Result<Self, Failure> {
        let objects = directory.join("objects");
        fs::create_dir_all(&objects)?;
        fs::create_dir_all(parts)?;
        let connection = sqlite::open(&directory.join("metadata.sqlite"))?;
        connection.execute_batch(SCHEMA)?;
        let versions = names(&connection, "SELECT version FROM objects")?;
        remove_entries(&objects, |name| versions.contains(name))?;
        let files = names(&connection, "SELECT file FROM parts")?;
        remove_entries(parts, |name| files.contains(name))?;
        Ok(Self {
            connection: Mutex::new(connection),
            objects,
            parts: parts.to_path_buf(),
        })
    }

    /// The metadata of the object `key`.
    pub(crate) fn head(&self, key: &str) -> Result<Option<Object>, Failure> {
        validate_key(key)?;
        let object = find(&lock(&self.connection), key)?;
        Ok(object.map(Object::whole))
    }

    /// The object `key`, with its body in the requested range unless its
    /// metadata fails the request's conditions.
    pub(crate) fn get(&self, key: &str, request: &GetRequest) -> Result<Option<Read>, Failure> {
        validate_key(key)?;
        let connection = lock(&self.connection);
        let Some(object) = find(&connection, key)? else {
            return Ok(None);
        };
        if !meets(request.only_if.as_ref(), Some(&object)) {
            return Ok(Some(Read::Unmet(object.whole())));
        }
        let range = request.range(object.size)?;
        let body = BodyReader::open(&self.objects.join(&object.version), range)?;
        Ok(Some(Read::Body(object.with_range(range), body)))
    }

    /// A writer of a `length`-byte object body, which a put then records.
    pub(crate) fn object_writer(
        &self,
        length: u64,
        checksum: Option<Checksum>,
    ) -> Result<BodyWriter, Failure> {
        if length > MAX_OBJECT_SIZE {
            return Err(Failure::EntityTooLarge);
        }
        BodyWriter::create(&self.objects, length, checksum)
    }

    /// Record `body` as the object `key`, uploaded at `now` milliseconds
    /// since the epoch, unless the object it would replace fails the
    /// request's conditions.
    pub(crate) fn put(
        &self,
        key: &str,
        body: Body,
        request: &PutRequest,
        now: i64,
    ) -> Result<Option<Object>, Failure> {
        validate_key(key)?;
        validate_metadata(&request.metadata.custom_metadata)?;
        let object = Object::written(
            key,
            Written {
                version: body.name.clone(),
                size: body.size,
                etag: hex(&body.md5),
                checksums: body.checksums()?,
            },
            request.metadata.clone(),
            now,
        );
        let mut connection = lock(&self.connection);
        let transaction = connection.transaction()?;
        let replaced = find(&transaction, key)?;
        if !meets(request.only_if.as_ref(), replaced.as_ref()) {
            return Ok(None);
        }
        record(&transaction, &object)?;
        transaction.commit()?;
        body.keep();
        drop(connection);
        remove_files(
            &self.objects,
            replaced.map(|replaced| replaced.version).as_slice(),
        );
        Ok(Some(object))
    }

    /// Delete the objects `keys`.
    pub(crate) fn delete(&self, keys: &[String]) -> Result<(), Failure> {
        for key in keys {
            validate_key(key)?;
        }
        let mut connection = lock(&self.connection);
        let transaction = connection.transaction()?;
        let mut versions = Vec::new();
        {
            let mut delete =
                transaction.prepare("DELETE FROM objects WHERE key = ?1 RETURNING version")?;
            for key in keys {
                versions.extend(
                    delete
                        .query_row([key], |row| row.get::<_, String>(0))
                        .optional()?,
                );
            }
        }
        transaction.commit()?;
        drop(connection);
        remove_files(&self.objects, &versions);
        Ok(())
    }
}

/// The body of a written object.
struct Written {
    version: String,
    size: u64,
    etag: String,
    /// The body's checksums, as JSON.
    checksums: String,
}

impl Object {
    /// The object `key` with `body`, uploaded at `now` milliseconds since
    /// the epoch.
    fn written(key: &str, body: Written, metadata: ObjectMetadata, now: i64) -> Self {
        Self {
            key: key.to_owned(),
            version: body.version,
            size: body.size,
            etag: body.etag,
            uploaded: now,
            checksums: Json(body.checksums),
            http_metadata: Some(metadata.http_metadata),
            custom_metadata: Some(metadata.custom_metadata),
            storage_class: metadata.storage_class,
            range: None,
        }
    }

    /// The object, with its whole body as its range.
    fn whole(self) -> Self {
        let range = Range::whole(self.size);
        self.with_range(range)
    }

    fn with_range(self, range: Range) -> Self {
        Self {
            range: Some(range),
            ..self
        }
    }

    /// The object in `row`, which has `FIND_OBJECT`'s columns, with the
    /// metadata that `http` and `custom` include.
    fn from_row(row: &Row<'_>, http: bool, custom: bool) -> rusqlite::Result<Self> {
        Ok(Self {
            key: row.get(0)?,
            version: row.get(1)?,
            size: row.get(2)?,
            etag: row.get(3)?,
            uploaded: row.get(4)?,
            checksums: Json(row.get(5)?),
            http_metadata: http.then(|| row.get(6).map(Json)).transpose()?,
            custom_metadata: custom.then(|| row.get(7).map(Json)).transpose()?,
            storage_class: row.get(8)?,
            range: None,
        })
    }
}

impl GetRequest {
    /// The bytes the request selects of a `size`-byte object. A header that
    /// selects no single satisfiable range selects them all.
    fn range(&self, size: u64) -> Result<Range, Failure> {
        match (&self.range_header, &self.range) {
            (Some(header), _) => Ok(header_range(header, size).unwrap_or(Range::whole(size))),
            (None, Some(bounds)) => bounds.resolve(size),
            (None, None) => Ok(Range::whole(size)),
        }
    }
}

/// Whether `object`, or the absence of one, meets the conditions of
/// `only_if`.
fn meets(only_if: Option<&Conditional>, object: Option<&Object>) -> bool {
    let current = object.map(|object| (object.etag.as_str(), object.uploaded));
    only_if.is_none_or(|only_if| only_if.holds(current))
}

fn find(connection: &Connection, key: &str) -> rusqlite::Result<Option<Object>> {
    connection
        .prepare(FIND_OBJECT)?
        .query_row([key], |row| Object::from_row(row, true, true))
        .optional()
}

/// Store `object`, which carries its metadata, replacing any with its key.
fn record(connection: &Connection, object: &Object) -> rusqlite::Result<()> {
    connection.prepare(RECORD_OBJECT)?.execute(params![
        object.key,
        object.version,
        object.size,
        object.etag,
        object.uploaded,
        object.checksums.0,
        object.http_metadata.as_ref().map(|json| &json.0),
        object.custom_metadata.as_ref().map(|json| &json.0),
        object.storage_class,
    ])?;
    Ok(())
}

/// Remove the files `names` in `directory`, which no row names. A file left
/// behind is removed when the bucket next opens.
fn remove_files(directory: &Path, names: &[String]) {
    for name in names {
        let _ = fs::remove_file(directory.join(name));
    }
}

fn standard_storage_class() -> String {
    "Standard".to_owned()
}

/// The names in the first column of `query`'s rows.
fn names(connection: &Connection, query: &str) -> rusqlite::Result<HashSet<String>> {
    connection
        .prepare(query)?
        .query_map([], |row| row.get(0))?
        .collect()
}

fn validate_key(key: &str) -> Result<(), Failure> {
    if key.len() > MAX_KEY_LENGTH {
        return Err(Failure::InvalidObjectName);
    }
    Ok(())
}

/// Custom metadata must fit R2's limit on its names and values.
fn validate_metadata(custom: &Json) -> Result<(), Failure> {
    let fields: BTreeMap<String, String> = serde_json::from_str(&custom.0)?;
    let size: usize = fields
        .iter()
        .map(|(name, value)| serialized_length(name) + serialized_length(value))
        .sum();
    if size > MAX_METADATA_SIZE {
        return Err(Failure::MetadataTooLarge);
    }
    Ok(())
}

/// The bytes the runtime stores `text` in: one per character when every
/// character fits in a byte, otherwise two per UTF-16 unit.
fn serialized_length(text: &str) -> usize {
    if text.chars().all(|character| u32::from(character) < 256) {
        text.chars().count()
    } else {
        text.encode_utf16().count() * 2
    }
}

/// A random name for a version, an upload or a body file.
fn random_name() -> Result<String, Failure> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|error| Failure::Storage(error.to_string()))?;
    Ok(hex(&bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

#[cfg(test)]
mod tests;
