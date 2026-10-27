//! Virtual path and memory-buffer helpers.

use std::sync::Mutex;

use super::{Error, ErrorKind, MAX_FILE_SIZE, MAX_PATH_LENGTH, MAX_PATH_SEGMENTS, Result};
use crate::lock;

pub(super) fn absolute_components(path: &str) -> Result<Vec<String>> {
    if !path.starts_with('/') {
        return Err(Error::new(ErrorKind::InvalidPath));
    }
    target_components(&[], path)
}

pub(super) fn target_components(base: &[String], target: &str) -> Result<Vec<String>> {
    if target.contains('\0') || target.chars().count() > MAX_PATH_LENGTH {
        return Err(Error::new(ErrorKind::InvalidPath));
    }
    let mut components = if target.starts_with('/') {
        Vec::new()
    } else {
        base.to_vec()
    };
    apply_components(&mut components, target)?;
    Ok(components)
}

fn apply_components(components: &mut Vec<String>, target: &str) -> Result<()> {
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if components.pop().is_none() {
                    return Err(Error::new(ErrorKind::InvalidPath));
                }
            }
            part => {
                components.push(part.to_owned());
                if components.len() > MAX_PATH_SEGMENTS {
                    return Err(Error::new(ErrorKind::InvalidPath));
                }
            }
        }
    }
    Ok(())
}

/// Whether names hold `byte` percent-encoded.
pub(crate) const fn escaped_in_names(byte: u8) -> bool {
    byte < 0x20
        || byte >= 0x7F
        || matches!(
            byte,
            b' ' | b'"' | b'#' | b'/' | b'<' | b'>' | b'?' | b'^' | b'`' | b'{' | b'}'
        )
}

/// Appends `byte` to `text` as an uppercase percent escape.
pub(crate) fn push_escape(text: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    text.push('%');
    text.push(char::from(HEX[usize::from(byte >> 4)]));
    text.push(char::from(HEX[usize::from(byte & 0xF)]));
}

/// The stored name of the host file name `name`.
pub(super) fn encoded_name(name: &str) -> String {
    let mut encoded = String::with_capacity(name.len());
    for &byte in name.as_bytes() {
        if escaped_in_names(byte) {
            push_escape(&mut encoded, byte);
        } else {
            encoded.push(char::from(byte));
        }
    }
    encoded
}

/// The byte the percent escape at `index` of `bytes` encodes, if one is there.
pub(crate) fn escape_at(bytes: &[u8], index: usize) -> Option<u8> {
    let hex = bytes
        .get(index + 1..index + 3)
        .filter(|_| bytes[index] == b'%')?;
    u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()
}

/// The host file name the stored name `name` spells, if it is valid UTF-8.
pub(super) fn decoded_name(name: &str) -> Option<String> {
    let bytes = name.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        let escaped = escape_at(bytes, index);
        decoded.push(escaped.unwrap_or(byte));
        index += if escaped.is_some() { 3 } else { 1 };
    }
    String::from_utf8(decoded).ok()
}

pub(crate) fn join_child(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}

pub(super) fn path_from_components(components: &[String]) -> String {
    format!("/{}", components.join("/"))
}

/// `path` without empty or `.` segments, as workerd names what it lists.
pub(crate) fn normalized_path(path: &str) -> Result<String> {
    Ok(path_from_components(&absolute_components(path)?))
}

pub(super) fn checked_size(size: u64) -> Result<usize> {
    if size > MAX_FILE_SIZE {
        return Err(Error::new(ErrorKind::FileTooLarge));
    }
    usize::try_from(size).map_err(|_| Error::new(ErrorKind::FileTooLarge))
}

pub(super) fn random_suffix(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    bytes
        .iter()
        .map(|&byte| char::from(ALPHABET[usize::from(byte) % ALPHABET.len()]))
        .collect()
}

pub(super) fn read_memory(data: &Mutex<Vec<u8>>, offset: u64, length: usize) -> Vec<u8> {
    let data = lock(data);
    read_bytes(&data, offset, length)
}

pub(super) fn read_bytes(data: &[u8], offset: u64, length: usize) -> Vec<u8> {
    let start = usize::try_from(offset).unwrap_or(usize::MAX);
    if start >= data.len() {
        return Vec::new();
    }
    let end = start.saturating_add(length).min(data.len());
    data[start..end].to_vec()
}

pub(super) fn write_memory(
    data: &Mutex<Vec<u8>>,
    offset: u64,
    input: &[u8],
    append: bool,
) -> Result<usize> {
    let mut data = lock(data);
    let start = if append {
        data.len()
    } else {
        usize::try_from(offset).map_err(|_| Error::new(ErrorKind::FileTooLarge))?
    };
    let end = start
        .checked_add(input.len())
        .ok_or_else(|| Error::new(ErrorKind::FileTooLarge))?;
    if end as u64 > MAX_FILE_SIZE {
        return Err(Error::new(ErrorKind::FileTooLarge));
    }
    if end > data.len() {
        data.resize(end, 0);
    }
    data[start..end].copy_from_slice(input);
    Ok(input.len())
}
