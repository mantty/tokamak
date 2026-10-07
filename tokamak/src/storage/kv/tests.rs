use super::{Entry, Key, KvNamespace, PutOptions, Value};
use crate::storage::{Location, Open};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const NOW: i64 = 1_000_000;

fn namespace() -> TestResult<(tempfile::TempDir, KvNamespace)> {
    let directory = tempfile::tempdir()?;
    let namespace = KvNamespace::open(&Location {
        path: directory.path().join("kv.sqlite"),
        scratch: directory.path().join("scratch"),
    })?;
    Ok((directory, namespace))
}

fn names(keys: &[Key]) -> Vec<&str> {
    keys.iter().map(|key| key.name.as_str()).collect()
}

fn written(bytes: &[u8]) -> Value {
    let mut value = Value::default();
    value.write(bytes);
    value
}

fn expiring(expiration: i64) -> PutOptions {
    PutOptions {
        expiration: Some(expiration),
        ..PutOptions::default()
    }
}

#[test]
fn stores_values_with_metadata() -> TestResult {
    let (_directory, namespace) = namespace()?;
    let metadata = PutOptions {
        metadata: Some("{\"a\":1}".to_owned()),
        ..PutOptions::default()
    };
    namespace.put("key", &written(b"value"), &metadata, NOW)?;
    namespace.put("key", &written(b"replaced"), &PutOptions::default(), NOW)?;

    assert_eq!(
        namespace.get("key", None, NOW)?,
        Some(Entry {
            value: b"replaced".to_vec(),
            metadata: None
        })
    );
    assert_eq!(namespace.get("missing", None, NOW)?, None);
    namespace.delete("key", NOW)?;
    assert_eq!(namespace.get("key", None, NOW)?, None);
    Ok(())
}

#[test]
fn keeps_no_bytes_of_a_value_written_past_the_limit() -> TestResult {
    let (_directory, namespace) = namespace()?;
    let (mut value, chunk) = (Value::default(), vec![0; 1024 * 1024]);
    for _ in 0..26 {
        value.write(&chunk);
    }

    let refusal = namespace
        .put("k", &value, &PutOptions::default(), NOW)
        .err();

    assert_eq!(value.bytes.capacity(), 0);
    assert_eq!(
        refusal.as_deref(),
        Some("413 Value length of 27262976 exceeds limit of 26214400.")
    );
    Ok(())
}

#[test]
fn stores_values_larger_than_sqlite_allocations() -> TestResult {
    let (_directory, namespace) = namespace()?;
    let value: Vec<u8> = (0..=250_u8).cycle().take(20 * 1024 * 1024).collect();

    namespace.put("large", &written(&value), &PutOptions::default(), NOW)?;

    assert_eq!(
        namespace.get("large", None, NOW)?.map(|entry| entry.value),
        Some(value)
    );
    Ok(())
}

#[test]
fn hides_expired_entries_and_deletes_them_during_writes() -> TestResult {
    let (_directory, namespace) = namespace()?;
    namespace.put("soon", &written(b"x"), &expiring(NOW + 60), NOW)?;
    let ttl = PutOptions {
        expiration_ttl: Some(120),
        ..PutOptions::default()
    };
    namespace.put("later", &written(b"y"), &ttl, NOW)?;

    let expired = NOW + 60;
    assert_eq!(namespace.get("soon", None, expired)?, None);
    let listed = namespace.list("", "", 10, expired)?.keys;
    assert_eq!(names(&listed), ["later"]);
    assert_eq!(listed[0].expiration, Some(NOW + 120));
    namespace.put("other", &written(b"z"), &PutOptions::default(), expired)?;
    assert_eq!(
        namespace.get("soon", None, NOW)?,
        None,
        "the write deleted the expired entry"
    );
    assert!(namespace.get("later", None, NOW)?.is_some());
    Ok(())
}

#[test]
fn lists_keys_in_byte_order_by_prefix_and_page() -> TestResult {
    let (_directory, namespace) = namespace()?;
    let options = PutOptions {
        metadata: Some("[1]".to_owned()),
        ..expiring(NOW + 100)
    };
    for key in ["b/2", "a", "b/1", "b/é", "b/\u{10FFFF}", "b0", "c"] {
        namespace.put(key, &written(b""), &options, NOW)?;
    }

    let prefixed = namespace.list("b/", "", 0, NOW)?;
    let first = namespace.list("b/", "", 2, NOW)?;
    let rest = namespace.list("b/", first.cursor.as_deref().unwrap_or(""), 2, NOW)?;

    assert_eq!(names(&prefixed.keys), ["b/1", "b/2", "b/é", "b/\u{10FFFF}"]);
    assert_eq!(prefixed.cursor, None);
    assert_eq!(names(&first.keys), ["b/1", "b/2"]);
    assert_eq!(names(&rest.keys), ["b/é", "b/\u{10FFFF}"]);
    assert_eq!(rest.cursor, None);
    assert_eq!(
        prefixed.keys[0],
        Key {
            name: "b/1".to_owned(),
            expiration: Some(NOW + 100),
            metadata: Some("[1]".to_owned())
        }
    );
    assert_eq!(
        namespace
            .get_many(&["c".to_owned(), "x".to_owned()], None, NOW)?
            .len(),
        2
    );
    Ok(())
}
