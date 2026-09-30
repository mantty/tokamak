//! Multipart uploads: parts uploaded separately, then joined into one object.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, BufWriter, ErrorKind};
use std::path::Path;

use md5::{Digest, Md5};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;

use super::body::Pending;
use super::{
    Body, BodyWriter, Failure, Json, Object, ObjectMetadata, R2Bucket, Written, find, hex, lock,
    random_name, record, remove_files, validate_key, validate_metadata,
};

/// An upload's states.
const IN_PROGRESS: i64 = 0;
const COMPLETED: i64 = 1;
const ABORTED: i64 = 2;

/// The smallest part but the last.
const MIN_PART_SIZE: u64 = 5 * 1024 * 1024;
/// The largest part.
const MAX_PART_SIZE: u64 = 5 * 1024 * 1024 * 1024;
/// Bytes a join buffers between reading parts and writing the object.
const JOIN_BUFFER: usize = 1024 * 1024;

/// A part a completion names.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UploadedPart {
    part_number: i64,
    etag: String,
}

/// A part stored for an upload.
#[derive(Debug, PartialEq, Eq)]
struct Part {
    number: i64,
    file: String,
    size: u64,
    etag: String,
    md5: Vec<u8>,
}

/// An upload's metadata, and the parts a completion joins, in order.
struct Completion {
    metadata: ObjectMetadata,
    parts: Vec<Part>,
}

impl R2Bucket {
    /// Start an upload of the object `key`, returning the upload's ID.
    pub(crate) fn create_upload(
        &self,
        key: &str,
        metadata: &ObjectMetadata,
    ) -> Result<String, Failure> {
        validate_key(key)?;
        validate_metadata(&metadata.custom_metadata)?;
        let id = random_name()?;
        lock(&self.connection).execute(
            "INSERT INTO uploads (id, key, http_metadata, custom_metadata, storage_class, state)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                key,
                metadata.http_metadata.0,
                metadata.custom_metadata.0,
                metadata.storage_class,
                IN_PROGRESS
            ],
        )?;
        Ok(id)
    }

    /// A writer of a `length`-byte part body, which an upload then records.
    pub(crate) fn part_writer(&self, length: u64) -> Result<BodyWriter, Failure> {
        if length > MAX_PART_SIZE {
            return Err(Failure::EntityTooLarge);
        }
        BodyWriter::create(&self.parts, length, None)
    }

    /// Record `body` as part `number` of the upload `id` of `key`, replacing
    /// any part with that number, and return the part's etag.
    pub(crate) fn upload_part(
        &self,
        key: &str,
        id: &str,
        number: i64,
        body: Body,
    ) -> Result<String, Failure> {
        validate_key(key)?;
        let etag = random_name()?;
        let mut connection = lock(&self.connection);
        let transaction = connection.transaction()?;
        if upload_state(&transaction, key, id)? != Some(IN_PROGRESS) {
            return Err(Failure::NoSuchUpload);
        }
        let replaced: Option<String> = transaction
            .query_row(
                "SELECT file FROM parts WHERE upload = ?1 AND number = ?2",
                params![id, number],
                |row| row.get(0),
            )
            .optional()?;
        transaction.execute(
            "INSERT OR REPLACE INTO parts (upload, number, file, size, etag, md5)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, number, body.name, body.size, etag, body.md5],
        )?;
        transaction.commit()?;
        body.keep();
        drop(connection);
        remove_files(&self.parts, replaced.as_slice());
        Ok(etag)
    }

    /// Join the `selected` parts of the upload `id` of `key` into the object
    /// `key`, uploaded at `now` milliseconds since the epoch.
    ///
    /// The parts are joined outside the bucket's lock, then the completion is
    /// checked again before the object is recorded.
    pub(crate) fn complete_upload(
        &self,
        key: &str,
        id: &str,
        selected: &[UploadedPart],
        now: i64,
    ) -> Result<Object, Failure> {
        validate_key(key)?;
        let planned = completion(&lock(&self.connection), key, id, selected)?;
        let version = random_name()?;
        let joined = join(&self.objects.join(&version), &self.parts, &planned.parts)?;
        let mut connection = lock(&self.connection);
        let transaction = connection.transaction()?;
        if completion(&transaction, key, id, selected)?.parts != planned.parts {
            return Err(Failure::InvalidPart);
        }
        let object = planned.object(key, version, now);
        let replaced = find(&transaction, key)?;
        record(&transaction, &object)?;
        let files = finish_upload(&transaction, id, COMPLETED)?;
        transaction.commit()?;
        joined.keep();
        drop(connection);
        remove_files(
            &self.objects,
            replaced.map(|replaced| replaced.version).as_slice(),
        );
        remove_files(&self.parts, &files);
        Ok(object)
    }

    /// Abort the upload `id` of `key`, deleting its parts.
    pub(crate) fn abort_upload(&self, key: &str, id: &str) -> Result<(), Failure> {
        validate_key(key)?;
        let mut connection = lock(&self.connection);
        let transaction = connection.transaction()?;
        match upload_state(&transaction, key, id)? {
            None => return Err(Failure::InternalError),
            Some(IN_PROGRESS) => {}
            Some(_) => return Ok(()),
        }
        let files = finish_upload(&transaction, id, ABORTED)?;
        transaction.commit()?;
        drop(connection);
        remove_files(&self.parts, &files);
        Ok(())
    }
}

impl Completion {
    /// The object the parts make. Its etag is the MD5 of the parts' MD5s,
    /// and the number of parts.
    fn object(self, key: &str, version: String, now: i64) -> Object {
        let digests: Vec<u8> = self
            .parts
            .iter()
            .flat_map(|part| part.md5.clone())
            .collect();
        let body = Written {
            version,
            size: self.parts.iter().map(|part| part.size).sum(),
            etag: format!("{}-{}", hex(&Md5::digest(&digests)), self.parts.len()),
            checksums: "{}".to_owned(),
        };
        Object::written(key, body, self.metadata, now)
    }
}

/// The state of the upload `id` of `key`, if there is one.
fn upload_state(connection: &Connection, key: &str, id: &str) -> rusqlite::Result<Option<i64>> {
    connection
        .query_row(
            "SELECT state FROM uploads WHERE id = ?1 AND key = ?2",
            [id, key],
            |row| row.get(0),
        )
        .optional()
}

/// Set the upload `id` to `state`, deleting its parts and returning their
/// files.
fn finish_upload(connection: &Connection, id: &str, state: i64) -> rusqlite::Result<Vec<String>> {
    connection.execute(
        "UPDATE uploads SET state = ?2 WHERE id = ?1",
        params![id, state],
    )?;
    connection
        .prepare("DELETE FROM parts WHERE upload = ?1 RETURNING file")?
        .query_map([id], |row| row.get(0))?
        .collect()
}

/// The metadata of an upload in progress, and the parts `selected` names in
/// part order, once they meet R2's rules: each names an uploaded part, and
/// every part but the last is at least `MIN_PART_SIZE` and the same size,
/// which the last does not exceed.
fn completion(
    connection: &Connection,
    key: &str,
    id: &str,
    selected: &[UploadedPart],
) -> Result<Completion, Failure> {
    let upload = connection
        .query_row(
            "SELECT http_metadata, custom_metadata, storage_class, state FROM uploads
                WHERE id = ?1 AND key = ?2",
            [id, key],
            |row| {
                let metadata = ObjectMetadata {
                    http_metadata: Json(row.get(0)?),
                    custom_metadata: Json(row.get(1)?),
                    storage_class: row.get(2)?,
                };
                Ok((metadata, row.get::<_, i64>(3)?))
            },
        )
        .optional()?;
    let Some((metadata, state)) = upload else {
        return Err(Failure::InternalError);
    };
    if state != IN_PROGRESS {
        return Err(Failure::NoSuchUpload);
    }
    let mut parts = selected_parts(connection, id, selected)?;
    let leading = parts.len().saturating_sub(1);
    if parts[..leading]
        .iter()
        .any(|part| part.size < MIN_PART_SIZE)
    {
        return Err(Failure::EntityTooSmall);
    }
    parts.sort_by_key(|part| part.number);
    if !uniform(&parts) {
        return Err(Failure::BadUpload);
    }
    Ok(Completion { metadata, parts })
}

/// The uploaded parts `selected` names, in the order it names them.
fn selected_parts(
    connection: &Connection,
    id: &str,
    selected: &[UploadedPart],
) -> Result<Vec<Part>, Failure> {
    let mut numbers = HashSet::new();
    if !selected.iter().all(|part| numbers.insert(part.part_number)) {
        return Err(Failure::InternalError);
    }
    let mut uploaded = uploaded_parts(connection, id)?;
    selected
        .iter()
        .map(|part| {
            uploaded
                .remove(&part.part_number)
                .filter(|uploaded| uploaded.etag == part.etag)
                .ok_or(Failure::InvalidPart)
        })
        .collect()
}

fn uploaded_parts(connection: &Connection, id: &str) -> rusqlite::Result<HashMap<i64, Part>> {
    connection
        .prepare("SELECT number, file, size, etag, md5 FROM parts WHERE upload = ?1")?
        .query_map([id], |row| {
            let part = Part {
                number: row.get(0)?,
                file: row.get(1)?,
                size: row.get(2)?,
                etag: row.get(3)?,
                md5: row.get(4)?,
            };
            Ok((part.number, part))
        })?
        .collect()
}

/// Whether every part but the last has one size, which the last does not
/// exceed.
fn uniform(parts: &[Part]) -> bool {
    let Some((last, leading)) = parts.split_last() else {
        return true;
    };
    let Some(first) = leading.first() else {
        return true;
    };
    leading.iter().all(|part| part.size == first.size) && last.size <= first.size
}

/// Join the bodies of `parts` in `directory` into a new file at `path`,
/// synced to disk, which is removed unless kept.
fn join(path: &Path, directory: &Path, parts: &[Part]) -> Result<Pending, Failure> {
    let (file, pending) = Pending::create(path.to_path_buf())?;
    let mut output = BufWriter::with_capacity(JOIN_BUFFER, file);
    for part in parts {
        let mut input = File::open(directory.join(&part.file)).map_err(|error| {
            if error.kind() == ErrorKind::NotFound {
                Failure::InvalidPart
            } else {
                Failure::from(error)
            }
        })?;
        io::copy(&mut input, &mut output)?;
    }
    let file = output
        .into_inner()
        .map_err(io::IntoInnerError::into_error)?;
    pending.sync(&file)?;
    Ok(pending)
}
