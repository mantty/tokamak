use std::fs;

use serde_json::{Value, json};

use super::multipart::UploadedPart;
use super::{
    Body, Checksum, Failure, GetRequest, Object, ObjectMetadata, PutRequest, R2Bucket, Read,
};
use crate::storage::{Location, Open};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const NOW: i64 = 1_700_000_000_000;
const MIB: usize = 1024 * 1024;

fn bucket() -> TestResult<(tempfile::TempDir, R2Bucket)> {
    let directory = tempfile::tempdir()?;
    let bucket = open(&directory)?;
    Ok((directory, bucket))
}

fn open(directory: &tempfile::TempDir) -> Result<R2Bucket, String> {
    R2Bucket::open(&Location {
        path: directory.path().join("bucket"),
        scratch: directory.path().join("scratch"),
    })
}

/// A request parsed from JSON text, as the binding sends it.
fn request<T: serde::de::DeserializeOwned>(json: &Value) -> TestResult<T> {
    Ok(serde_json::from_str(&json.to_string())?)
}

fn metadata() -> Value {
    json!({ "httpMetadata": { "contentType": "text/plain" }, "customMetadata": { "a": "1" }, "storageClass": "Standard" })
}

fn body(bucket: &R2Bucket, bytes: &[u8], checksum: Option<Checksum>) -> TestResult<Body> {
    let mut writer = bucket.object_writer(bytes.len() as u64, checksum)?;
    writer.write(bytes)?;
    Ok(writer.finish()?)
}

fn put_request(only_if: &Value) -> TestResult<PutRequest> {
    request(&json!({ "onlyIf": only_if, "metadata": metadata() }))
}

fn put(bucket: &R2Bucket, key: &str, bytes: &[u8]) -> TestResult<Object> {
    let body = body(bucket, bytes, None)?;
    bucket
        .put(key, body, &put_request(&Value::Null)?, NOW)?
        .ok_or_else(|| "the put's conditions failed".into())
}

fn read(bucket: &R2Bucket, key: &str, get: &Value) -> TestResult<Option<Vec<u8>>> {
    let Some(Read::Body(_, mut reader)) = bucket.get(key, &request::<GetRequest>(get)?)? else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    while let Some(chunk) = reader.read()? {
        bytes.extend(chunk);
    }
    Ok(Some(bytes))
}

fn files(directory: &std::path::Path) -> TestResult<usize> {
    Ok(fs::read_dir(directory)?.count())
}

#[test]
fn stores_objects_with_their_metadata() -> TestResult {
    let (directory, bucket) = bucket()?;

    let written = put(&bucket, "a", b"hello")?;
    let head = bucket.head("a")?.ok_or("no object")?;

    assert_eq!(written.etag, "5d41402abc4b2a76b9719d911017c592");
    assert_eq!(written.version.len(), 32);
    assert_eq!(
        serde_json::to_value(&head)?,
        json!({
            "key": "a", "version": written.version, "size": 5, "etag": written.etag, "uploaded": NOW,
            "checksums": { "md5": written.etag }, "httpMetadata": { "contentType": "text/plain" },
            "customMetadata": { "a": "1" }, "storageClass": "Standard", "range": { "offset": 0, "length": 5 }
        })
    );
    assert_eq!(read(&bucket, "a", &json!({}))?, Some(b"hello".to_vec()));
    assert_eq!(bucket.head("missing")?, None);
    assert!(bucket.get("missing", &GetRequest::default())?.is_none());
    assert_eq!(files(&directory.path().join("bucket/objects"))?, 1);
    Ok(())
}

#[test]
fn reads_ranges_of_a_body() -> TestResult {
    let (_directory, bucket) = bucket()?;
    let bytes: Vec<u8> = (0..=255).cycle().take(200_000).collect();
    put(&bucket, "range", &bytes)?;

    let range = |get: Value| read(&bucket, "range", &get);
    assert_eq!(
        range(json!({ "range": { "offset": 70_000, "length": 70_000 } }))?,
        Some(bytes[70_000..140_000].to_vec())
    );
    assert_eq!(
        range(json!({ "range": { "suffix": 3 } }))?,
        Some(bytes[199_997..].to_vec())
    );
    assert_eq!(
        range(json!({ "rangeHeader": "bytes=1-2" }))?,
        Some(bytes[1..3].to_vec())
    );
    assert_eq!(
        range(json!({ "rangeHeader": "bytes=1-2,4-5" }))?,
        Some(bytes.clone())
    );
    assert_eq!(
        bucket
            .get(
                "range",
                &request(&json!({ "range": { "offset": 200_000 } }))?
            )
            .err(),
        Some(Failure::InvalidRange)
    );
    Ok(())
}

#[test]
fn replaces_an_object_whole_while_it_is_read() -> TestResult {
    let (directory, bucket) = bucket()?;
    let old = vec![1; 200_000];
    put(&bucket, "k", &old)?;
    let Some(Read::Body(_, mut reader)) = bucket.get("k", &GetRequest::default())? else {
        return Err("no body".into());
    };
    let first = reader.read()?.ok_or("empty body")?;

    put(&bucket, "k", &[2; 10])?;

    let mut bytes = first;
    while let Some(chunk) = reader.read()? {
        bytes.extend(chunk);
    }
    assert_eq!(bytes, old);
    assert_eq!(read(&bucket, "k", &json!({}))?, Some(vec![2; 10]));
    assert_eq!(files(&directory.path().join("bucket/objects"))?, 1);
    Ok(())
}

#[test]
fn writes_only_when_the_conditions_hold() -> TestResult {
    let (directory, bucket) = bucket()?;
    let first = put(&bucket, "k", b"one")?;
    let conditional = |only_if: Value| -> TestResult<Option<Object>> {
        let body = body(&bucket, b"two", None)?;
        Ok(bucket.put("k", body, &put_request(&only_if)?, NOW + 1)?)
    };

    assert_eq!(
        conditional(json!({ "etagMatches": [{ "type": "strong", "value": "x" }] }))?,
        None
    );
    assert_eq!(files(&directory.path().join("bucket/objects"))?, 1);
    assert_eq!(read(&bucket, "k", &json!({}))?, Some(b"one".to_vec()));
    let second =
        conditional(json!({ "etagMatches": [{ "type": "strong", "value": first.etag }] }))?;
    assert_eq!(second.map(|object| object.size), Some(3));
    assert_eq!(read(&bucket, "k", &json!({}))?, Some(b"two".to_vec()));
    let unmet = bucket.get(
        "k",
        &request(&json!({ "onlyIf": { "etagDoesNotMatch": [{ "type": "wildcard" }] } }))?,
    )?;
    assert!(
        matches!(unmet, Some(Read::Unmet(object)) if object.range.map(|range| range.length) == Some(3))
    );
    Ok(())
}

#[test]
fn verifies_checksums_and_limits() -> TestResult {
    let (directory, bucket) = bucket()?;
    let sha1 = |value: &str| Checksum {
        algorithm: super::body::Algorithm::Sha1,
        value: value.to_owned(),
    };
    let good = body(
        &bucket,
        b"x",
        Some(sha1("11f6ad8ec52a2984abaafd7c3b516503785c2072")),
    )?;
    let bad = body(&bucket, b"x", Some(sha1("00")))?;

    let stored = bucket
        .put("sha", good, &put_request(&Value::Null)?, NOW)?
        .ok_or("not stored")?;
    let rejected = bucket.put("sha", bad, &put_request(&Value::Null)?, NOW);

    assert_eq!(
        serde_json::to_value(&stored.checksums)?,
        json!({ "md5": "9dd4e461268c8034f5c8564e155c67a6", "sha1": "11f6ad8ec52a2984abaafd7c3b516503785c2072" })
    );
    assert!(matches!(rejected, Err(Failure::BadDigest { .. })));
    assert_eq!(files(&directory.path().join("bucket/objects"))?, 1);
    assert_eq!(
        bucket.object_writer(5_363_466_241, None).err(),
        Some(Failure::EntityTooLarge)
    );
    let long_key = body(&bucket, b"", None)?;
    assert_eq!(
        bucket
            .put(
                &"k".repeat(1025),
                long_key,
                &put_request(&Value::Null)?,
                NOW
            )
            .err(),
        Some(Failure::InvalidObjectName)
    );
    let custom = |fields: Value| -> TestResult<PutRequest> {
        let mut metadata = metadata();
        metadata["customMetadata"] = fields;
        request(&json!({ "metadata": metadata }))
    };
    let fits = body(&bucket, b"", None)?;
    let exceeds = body(&bucket, b"", None)?;
    assert!(
        bucket
            .put("m", fits, &custom(json!({ "a": "é".repeat(2047) }))?, NOW)
            .is_ok()
    );
    assert_eq!(
        bucket
            .put(
                "m",
                exceeds,
                &custom(json!({ "a": "\u{100}".repeat(1024) }))?,
                NOW
            )
            .err(),
        Some(Failure::MetadataTooLarge)
    );
    Ok(())
}

#[test]
fn removes_a_body_it_does_not_record() -> TestResult {
    let (directory, bucket) = bucket()?;
    let mut writer = bucket.object_writer(4, None)?;
    writer.write(b"ab")?;
    assert_eq!(files(&directory.path().join("bucket/objects"))?, 1);

    assert!(writer.write(b"cde").is_err());
    drop(writer);

    assert_eq!(files(&directory.path().join("bucket/objects"))?, 0);
    let mut short = bucket.object_writer(4, None)?;
    short.write(b"ab")?;
    assert!(short.finish().is_err());
    assert_eq!(files(&directory.path().join("bucket/objects"))?, 0);
    Ok(())
}

#[test]
fn deletes_objects_and_their_bodies() -> TestResult {
    let (directory, bucket) = bucket()?;
    put(&bucket, "a", b"1")?;
    put(&bucket, "b", b"2")?;

    bucket.delete(&["a".to_owned(), "b".to_owned(), "missing".to_owned()])?;

    assert_eq!(bucket.head("a")?, None);
    assert_eq!(files(&directory.path().join("bucket/objects"))?, 0);
    assert_eq!(
        bucket.delete(&["k".repeat(1025)]),
        Err(Failure::InvalidObjectName)
    );
    Ok(())
}

#[test]
fn removes_bodies_without_rows_when_it_opens() -> TestResult {
    let directory = tempfile::tempdir()?;
    let bucket = open(&directory)?;
    let kept = put(&bucket, "kept", b"1")?;
    let upload = bucket.create_upload("u", &request::<ObjectMetadata>(&metadata())?)?;
    let mut part = bucket.part_writer(1)?;
    part.write(b"p")?;
    bucket.upload_part("u", &upload, 1, part.finish()?)?;
    drop(bucket);
    fs::write(directory.path().join("bucket/objects/orphan"), "x")?;
    fs::write(directory.path().join("scratch/orphan"), "x")?;

    let bucket = open(&directory)?;

    assert_eq!(files(&directory.path().join("bucket/objects"))?, 1);
    assert!(
        directory
            .path()
            .join("bucket/objects")
            .join(&kept.version)
            .is_file()
    );
    assert_eq!(files(&directory.path().join("scratch"))?, 1);
    assert_eq!(read(&bucket, "kept", &json!({}))?, Some(b"1".to_vec()));
    Ok(())
}

fn keys(listing: &super::list::Listing) -> Vec<&str> {
    listing
        .objects
        .iter()
        .map(|object| object.key.as_str())
        .collect()
}

#[test]
fn lists_keys_in_order_rolling_up_delimited_prefixes() -> TestResult {
    let (_directory, bucket) = bucket()?;
    for key in [
        "l/a",
        "l/b/1",
        "l/b/2",
        "l/b/c/3",
        "l/b/d",
        "l/b0",
        "l/c",
        "l/c/",
        "l/c//x",
        "l/é/1",
        "l/\u{1F600}",
        "m",
    ] {
        put(&bucket, key, b"")?;
    }

    let all = bucket.list(&request(&json!({ "prefix": "l/" }))?)?;
    let grouped = bucket.list(&request(&json!({ "prefix": "l/", "delimiter": "/" }))?)?;
    let nested = bucket.list(&request(&json!({ "prefix": "l/b/", "delimiter": "/" }))?)?;
    let after = bucket.list(&request(
        &json!({ "prefix": "l/", "startAfter": "l/b/2", "delimiter": "/" }),
    )?)?;

    assert_eq!(
        keys(&all),
        [
            "l/a",
            "l/b/1",
            "l/b/2",
            "l/b/c/3",
            "l/b/d",
            "l/b0",
            "l/c",
            "l/c/",
            "l/c//x",
            "l/é/1",
            "l/\u{1F600}"
        ]
    );
    assert!(!all.truncated && all.cursor.is_none());
    assert_eq!(keys(&grouped), ["l/a", "l/b0", "l/c", "l/\u{1F600}"]);
    assert_eq!(grouped.delimited_prefixes, ["l/b/", "l/c/", "l/é/"]);
    assert_eq!(keys(&nested), ["l/b/1", "l/b/2", "l/b/d"]);
    assert_eq!(nested.delimited_prefixes, ["l/b/c/"]);
    assert_eq!(keys(&after), ["l/b0", "l/c", "l/\u{1F600}"]);
    assert_eq!(after.delimited_prefixes, ["l/b/", "l/c/", "l/é/"]);
    assert_eq!(all.objects[0].http_metadata, None);
    Ok(())
}

#[test]
fn pages_a_listing_past_whole_delimited_prefixes() -> TestResult {
    let (_directory, bucket) = bucket()?;
    for key in ["a", "b/1", "b/2", "b/3", "c", "d/1", "e"] {
        put(&bucket, key, b"")?;
    }
    let mut pages = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = bucket.list(&request(
            &json!({ "delimiter": "/", "limit": 2, "cursor": cursor }),
        )?)?;
        pages.push((keys(&page).join(","), page.delimited_prefixes.join(",")));
        cursor = page.cursor.clone();
        if !page.truncated {
            break;
        }
    }

    assert_eq!(
        pages,
        [
            ("a".to_owned(), "b/".to_owned()),
            ("c".to_owned(), "d/".to_owned()),
            ("e".to_owned(), String::new()),
        ]
    );
    Ok(())
}

#[test]
fn limits_listings() -> TestResult {
    let (_directory, bucket) = bucket()?;
    put(&bucket, "a", b"")?;

    let with_metadata = bucket.list(&request(
        &json!({ "include": ["httpMetadata"], "limit": 1000 }),
    )?)?;

    assert_eq!(
        bucket.list(&request(&json!({ "limit": 0 }))?).err(),
        Some(Failure::InvalidMaxKeys)
    );
    assert_eq!(
        bucket.list(&request(&json!({ "limit": 1001 }))?).err(),
        Some(Failure::InvalidMaxKeys)
    );
    assert_eq!(
        serde_json::to_value(&with_metadata.objects[0].http_metadata)?,
        json!({ "contentType": "text/plain" })
    );
    assert_eq!(with_metadata.objects[0].custom_metadata, None);
    Ok(())
}

fn part(
    bucket: &R2Bucket,
    key: &str,
    upload: &str,
    number: i64,
    bytes: &[u8],
) -> TestResult<UploadedPart> {
    let mut writer = bucket.part_writer(bytes.len() as u64)?;
    writer.write(bytes)?;
    let etag = bucket.upload_part(key, upload, number, writer.finish()?)?;
    request(&json!({ "partNumber": number, "etag": etag }))
}

#[test]
fn joins_uploaded_parts_in_part_order() -> TestResult {
    let (directory, bucket) = bucket()?;
    put(&bucket, "big", b"old")?;
    let upload = bucket.create_upload("big", &request::<ObjectMetadata>(&metadata())?)?;
    let second = part(&bucket, "big", &upload, 2, &vec![2; 5 * MIB])?;
    let last = part(&bucket, "big", &upload, 3, b"end")?;
    let first = part(&bucket, "big", &upload, 1, &vec![1; 5 * MIB])?;
    part(&bucket, "big", &upload, 4, b"unused")?;

    let object = bucket.complete_upload("big", &upload, &[second, first, last], NOW)?;

    assert_eq!(object.size, (10 * MIB + 3) as u64);
    assert!(
        object.etag.ends_with("-3") && object.etag.len() == 34,
        "{}",
        object.etag
    );
    assert_eq!(serde_json::to_value(&object.checksums)?, json!({}));
    let bytes = read(&bucket, "big", &json!({}))?.ok_or("no object")?;
    assert_eq!(
        (bytes[0], bytes[5 * MIB], &bytes[10 * MIB..]),
        (1, 2, &b"end"[..])
    );
    assert_eq!(files(&directory.path().join("scratch"))?, 0);
    assert_eq!(files(&directory.path().join("bucket/objects"))?, 1);
    assert_eq!(
        bucket.complete_upload("big", &upload, &[], NOW).err(),
        Some(Failure::NoSuchUpload)
    );
    assert_eq!(bucket.abort_upload("big", &upload), Ok(()));
    Ok(())
}

#[test]
fn applies_r2s_rules_to_the_parts_of_an_upload() -> TestResult {
    let (_directory, bucket) = bucket()?;
    let metadata = request::<ObjectMetadata>(&metadata())?;
    let upload = |key: &str| bucket.create_upload(key, &metadata);
    let small = upload("small")?;
    let tiny = [
        part(&bucket, "small", &small, 1, b"a")?,
        part(&bucket, "small", &small, 2, b"b")?,
    ];
    let uneven = upload("uneven")?;
    let parts = [
        part(&bucket, "uneven", &uneven, 1, &vec![0; 5 * MIB])?,
        part(&bucket, "uneven", &uneven, 2, &vec![0; 5 * MIB + 1])?,
    ];
    let replaced = upload("replaced")?;
    let stale = part(&bucket, "replaced", &replaced, 1, b"a")?;
    let fresh = part(&bucket, "replaced", &replaced, 1, b"bb")?;

    assert_eq!(
        bucket.complete_upload("small", &small, &tiny, NOW).err(),
        Some(Failure::EntityTooSmall)
    );
    assert_eq!(
        bucket.complete_upload("uneven", &uneven, &parts, NOW).err(),
        Some(Failure::BadUpload)
    );
    assert_eq!(
        bucket
            .complete_upload("replaced", &replaced, &[stale], NOW)
            .err(),
        Some(Failure::InvalidPart)
    );
    let duplicate = [
        request(&json!({ "partNumber": 1, "etag": "x" }))?,
        request(&json!({ "partNumber": 1, "etag": "x" }))?,
    ];
    assert_eq!(
        bucket
            .complete_upload("replaced", &replaced, &duplicate, NOW)
            .err(),
        Some(Failure::InternalError)
    );
    assert_eq!(
        bucket
            .complete_upload("replaced", &replaced, &[fresh], NOW)?
            .size,
        2
    );
    let empty = upload("empty")?;
    assert_eq!(
        bucket.complete_upload("empty", &empty, &[], NOW)?.etag,
        "d41d8cd98f00b204e9800998ecf8427e-0"
    );
    Ok(())
}

#[test]
fn tracks_upload_states() -> TestResult {
    let (directory, bucket) = bucket()?;
    let upload = bucket.create_upload("k", &request::<ObjectMetadata>(&metadata())?)?;
    part(&bucket, "k", &upload, 1, b"a")?;

    assert_eq!(
        bucket.abort_upload("other", &upload),
        Err(Failure::InternalError)
    );
    assert_eq!(
        bucket.abort_upload("k", "unknown"),
        Err(Failure::InternalError)
    );
    assert_eq!(
        bucket.complete_upload("k", "unknown", &[], NOW).err(),
        Some(Failure::InternalError)
    );
    bucket.abort_upload("k", &upload)?;

    assert_eq!(files(&directory.path().join("scratch"))?, 0);
    assert_eq!(bucket.abort_upload("k", &upload), Ok(()));
    assert_eq!(
        part(&bucket, "k", &upload, 2, b"b")
            .err()
            .map(|error| error.to_string()),
        Some(Failure::NoSuchUpload.to_string())
    );
    assert_eq!(
        bucket.complete_upload("k", &upload, &[], NOW).err(),
        Some(Failure::NoSuchUpload)
    );
    assert_eq!(files(&directory.path().join("scratch"))?, 0);
    Ok(())
}

#[test]
fn reports_failures_as_r2_does() {
    assert_eq!(
        Failure::InvalidObjectName.to_string(),
        "The specified object name is not valid. (10020)"
    );
    assert_eq!(
        Failure::BadDigest {
            algorithm: super::body::Algorithm::Md5,
            provided: "00".to_owned(),
            actual: "11".to_owned()
        }
        .to_string(),
        "The MD5 checksum you specified did not match what we received.\nYou provided a MD5 checksum with value: 00\nActual MD5 was: 11 (10037)"
    );
}
