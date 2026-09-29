use super::{Entry, Key, KvNamespace, prefix_end};
use crate::storage::Open;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const NOW: i64 = 1_000_000;

fn namespace() -> TestResult<(tempfile::TempDir, KvNamespace)> {
    let directory = tempfile::tempdir()?;
    let namespace = KvNamespace::open(&directory.path().join("kv.sqlite"))?;
    Ok((directory, namespace))
}

fn names(keys: &[Key]) -> Vec<&str> {
    keys.iter().map(|key| key.name.as_str()).collect()
}

#[test]
fn stores_values_with_metadata() -> TestResult {
    let (_directory, namespace) = namespace()?;
    namespace.put("key", b"value", None, Some("{\"a\":1}"), NOW)?;
    namespace.put("key", b"replaced", None, None, NOW)?;

    assert_eq!(
        namespace.get("key", NOW)?,
        Some(Entry {
            value: b"replaced".to_vec(),
            metadata: None
        })
    );
    assert_eq!(namespace.get("missing", NOW)?, None);
    namespace.delete("key", NOW)?;
    assert_eq!(namespace.get("key", NOW)?, None);
    Ok(())
}

#[test]
fn stores_values_larger_than_sqlite_allocations() -> TestResult {
    let (_directory, namespace) = namespace()?;
    let value: Vec<u8> = (0..=250_u8).cycle().take(20 * 1024 * 1024).collect();

    namespace.put("large", &value, None, None, NOW)?;

    assert_eq!(
        namespace.get("large", NOW)?.map(|entry| entry.value),
        Some(value)
    );
    Ok(())
}

#[test]
fn hides_expired_entries_and_deletes_them_during_writes() -> TestResult {
    let (_directory, namespace) = namespace()?;
    namespace.put("soon", b"x", Some(NOW + 60), None, NOW)?;
    namespace.put("later", b"y", Some(NOW + 120), None, NOW)?;

    let expired = NOW + 60;
    assert_eq!(namespace.get("soon", expired)?, None);
    assert_eq!(names(&namespace.list("", "", 10, expired)?.keys), ["later"]);
    namespace.put("other", b"z", None, None, expired)?;
    assert_eq!(
        namespace.get("soon", NOW)?,
        None,
        "the write deleted the expired entry"
    );
    assert!(namespace.get("later", NOW)?.is_some());
    Ok(())
}

#[test]
fn lists_keys_in_byte_order_by_prefix_and_page() -> TestResult {
    let (_directory, namespace) = namespace()?;
    for key in ["b/2", "a", "b/1", "b/é", "b/\u{10FFFF}", "b0", "c"] {
        namespace.put(key, b"", Some(NOW + 100), Some("[1]"), NOW)?;
    }

    let prefixed = namespace.list("b/", "", 1000, NOW)?;
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
            .get_many(&["c".to_owned(), "x".to_owned()], NOW)?
            .len(),
        2
    );
    Ok(())
}

#[test]
fn bounds_a_prefix_by_its_next_string() {
    assert_eq!(prefix_end("ab").as_deref(), Some("ac"));
    assert_eq!(prefix_end("a\u{D7FF}").as_deref(), Some("a\u{E000}"));
    assert_eq!(prefix_end("a\u{10FFFF}").as_deref(), Some("b"));
    assert_eq!(prefix_end("\u{10FFFF}"), None);
    assert_eq!(prefix_end(""), None);
}
