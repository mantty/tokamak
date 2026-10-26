//! Key ranges and page cursors for listing stores in key order.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

/// The least string greater than every string beginning with `prefix`, or
/// none when no string is. Keys order by UTF-8 bytes, which is code point
/// order.
pub(super) fn prefix_end(prefix: &str) -> Option<String> {
    let mut characters: Vec<char> = prefix.chars().collect();
    while let Some(last) = characters.pop() {
        let next = (u32::from(last) + 1..=u32::from(char::MAX)).find_map(char::from_u32);
        if let Some(next) = next {
            characters.push(next);
            return Some(characters.into_iter().collect());
        }
    }
    None
}

/// The cursor to the page after `key`.
pub(super) fn cursor(key: &str) -> String {
    STANDARD.encode(key)
}

/// The key a cursor follows, or none when `cursor` encodes no key.
pub(super) fn cursor_key(cursor: &str) -> Option<String> {
    STANDARD
        .decode(cursor)
        .ok()
        .and_then(|key| String::from_utf8(key).ok())
}

#[cfg(test)]
mod tests {
    use super::{cursor, cursor_key, prefix_end};

    #[test]
    fn bounds_a_prefix_by_its_next_string() {
        for (prefix, end) in [
            ("ab", Some("ac")),
            ("a\u{D7FF}", Some("a\u{E000}")),
            ("a\u{10FFFF}", Some("b")),
            ("\u{10FFFF}", None),
            ("", None),
        ] {
            assert_eq!(prefix_end(prefix).as_deref(), end, "{prefix:?}");
        }
    }

    #[test]
    fn encodes_the_key_a_page_follows() {
        assert_eq!(cursor_key(&cursor("l/é")).as_deref(), Some("l/é"));
        assert_eq!(cursor_key("!!!"), None);
        assert_eq!(cursor_key(""), Some(String::new()));
    }
}
