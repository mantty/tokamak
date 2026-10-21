//! Length-delimited native routing metadata and opaque endpoint payloads.

use std::io;

use serde::{Deserialize, Serialize};

#[derive(Default, Deserialize, Serialize)]
pub(crate) struct Header {
    pub r#type: String,
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overflow: Option<String>,
    #[serde(default)]
    pub failed: bool,
    #[serde(default)]
    pub done: bool,
}

pub(crate) fn encode(header: &Header, payload: &[u8]) -> io::Result<Vec<u8>> {
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

pub(crate) fn decode(bytes: &[u8]) -> io::Result<(Header, &[u8])> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "Invalid native message length");
    let length = u32::from_le_bytes(
        bytes
            .get(..4)
            .ok_or_else(invalid)?
            .try_into()
            .map_err(|_| invalid())?,
    ) as usize;
    let metadata = u32::from_le_bytes(
        bytes
            .get(4..8)
            .ok_or_else(invalid)?
            .try_into()
            .map_err(|_| invalid())?,
    ) as usize;
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
