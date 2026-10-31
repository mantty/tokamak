use hyper::header::HeaderMap;

use crate::typed::StringPairs;

/// `headers` as JavaScript header pairs. Workerd exposes HTTP header bytes as
/// UTF-8, including replacement characters for invalid sequences, and keeps
/// repeated fields as separate pairs.
pub(crate) fn pairs(headers: &HeaderMap) -> StringPairs {
    let pairs = headers.iter().map(|(name, value)| {
        let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
        (name.as_str().to_owned(), value)
    });
    StringPairs(pairs.collect())
}
