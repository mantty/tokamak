use reqwest::header::HeaderMap;
use serde::{Serializer, ser::SerializeSeq};

/// Workerd exposes HTTP header bytes as UTF-8, including replacement characters
/// for invalid sequences. Keep repeated fields as separate JS header pairs.
pub(crate) fn serialize<S: Serializer>(
    headers: &HeaderMap,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut pairs = serializer.serialize_seq(Some(headers.len()))?;
    for (name, value) in headers {
        pairs.serialize_element(&(name.as_str(), String::from_utf8_lossy(value.as_bytes())))?;
    }
    pairs.end()
}
