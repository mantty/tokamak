//! Length-delimited native routing metadata and opaque endpoint payloads.

use std::io;

use serde::{Deserialize, Serialize};
use tokamak_macros::JsType;

/// A message's routing header.
#[derive(Default, Deserialize, Serialize, JsType)]
#[serde(rename = "MessageHeader")]
pub struct Header {
    /// What the message is, such as `call`, `event` or `result`.
    pub r#type: String,
    /// The call or stream the message belongs to.
    pub id: u64,
    /// The plugin a call or subscription is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin: Option<String>,
    /// The plugin method a call or subscription runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// The event an `event` message carries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// What a subscription does with values that arrive faster than they are
    /// read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overflow: Option<String>,
    /// Whether a result or chunk carries a failure.
    #[serde(default)]
    pub failed: bool,
    /// Whether a result or chunk is the last of its call or stream.
    #[serde(default)]
    pub done: bool,
}

/// The message of `header` and `payload`: its length, the header's length,
/// the header's JSON and the payload.
///
/// # Errors
///
/// Fails when the message is longer than 4 GiB.
pub fn encode(header: &Header, payload: &[u8]) -> io::Result<Vec<u8>> {
    let metadata = serde_json::to_vec(header)?;
    let length = 4usize
        .checked_add(metadata.len())
        .and_then(|size| size.checked_add(payload.len()))
        .and_then(|size| u32::try_from(size).ok())
        .ok_or_else(|| io::Error::other("Native message exceeds 4 GiB"))?;
    let metadata_len = u32::try_from(metadata.len()).map_err(io::Error::other)?;
    let mut message = Vec::with_capacity(length as usize + 4);
    message.extend_from_slice(&length.to_le_bytes());
    message.extend_from_slice(&metadata_len.to_le_bytes());
    message.extend_from_slice(&metadata);
    message.extend_from_slice(payload);
    Ok(message)
}

/// The header and payload of the message `bytes`.
///
/// # Errors
///
/// Fails when the lengths `bytes` gives are wrong or its header is not a
/// header's JSON.
pub fn decode(bytes: &[u8]) -> io::Result<(Header, &[u8])> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "Invalid native message length");
    let length = u32::from_le_bytes(*bytes.first_chunk::<4>().ok_or_else(invalid)?) as usize;
    let metadata = u32::from_le_bytes(*bytes[4..].first_chunk::<4>().ok_or_else(invalid)?) as usize;
    if length != bytes.len() - 4 || metadata > length.saturating_sub(4) {
        return Err(invalid());
    }
    let header = serde_json::from_slice(&bytes[8..8 + metadata])?;
    Ok((header, &bytes[8 + metadata..]))
}

#[cfg(test)]
mod tests {
    use super::{Header, decode, encode};

    #[test]
    fn preserves_opaque_bytes_and_completion_metadata() -> Result<(), Box<dyn std::error::Error>> {
        let header = Header {
            r#type: "result".into(),
            id: 42,
            done: true,
            ..Header::default()
        };
        let payload = [0, 255, 0xc0, 0xaf, 4];
        let encoded = encode(&header, &payload)?;
        let (decoded, bytes) = decode(&encoded)?;
        assert_eq!(decoded.id, 42);
        assert!(decoded.done);
        assert_eq!(bytes, payload);
        assert!(decode(&encoded[..encoded.len() - 1]).is_err());
        Ok(())
    }
}
