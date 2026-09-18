//! Virtual path and memory-buffer helpers.

use std::sync::{Mutex, MutexGuard, PoisonError};

use super::{Error, ErrorKind, MAX_FILE_SIZE, MAX_PATH_LENGTH, MAX_PATH_SEGMENTS, Result};

pub(super) fn absolute_components(path: &str) -> Result<Vec<String>> {
    if !path.starts_with('/') || path.contains('\0') || path.chars().count() > MAX_PATH_LENGTH {
        return Err(Error::new(ErrorKind::InvalidPath, path));
    }
    let mut components = Vec::new();
    apply_components(&mut components, path.split('/'), path)?;
    Ok(components)
}

pub(super) fn target_components(base: &[String], target: &str, path: &str) -> Result<Vec<String>> {
    if target.contains('\0') || target.chars().count() > MAX_PATH_LENGTH {
        return Err(Error::new(ErrorKind::InvalidPath, path));
    }
    let mut components = if target.starts_with('/') {
        Vec::new()
    } else {
        base.to_vec()
    };
    apply_components(&mut components, target.split('/'), path)?;
    Ok(components)
}

pub(super) fn apply_components<'a>(
    components: &mut Vec<String>,
    parts: impl IntoIterator<Item = &'a str>,
    path: &str,
) -> Result<()> {
    for part in parts {
        match part {
            "" | "." => {}
            ".." => {
                if components.pop().is_none() {
                    return Err(Error::new(ErrorKind::InvalidPath, path));
                }
            }
            part if part.contains('\\') => {
                return Err(Error::new(ErrorKind::InvalidPath, path));
            }
            part => {
                components.push(part.to_owned());
                if components.len() > MAX_PATH_SEGMENTS {
                    return Err(Error::new(ErrorKind::InvalidPath, path));
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn join_child(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}

pub(super) fn path_from_components(components: &[String]) -> String {
    if components.is_empty() {
        "/".to_owned()
    } else {
        format!("/{}", components.join("/"))
    }
}

pub(super) fn checked_size(size: u64, path: &str) -> Result<usize> {
    if size > MAX_FILE_SIZE {
        return Err(Error::new(ErrorKind::FileTooLarge, path));
    }
    usize::try_from(size).map_err(|_| Error::new(ErrorKind::FileTooLarge, path))
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
        usize::try_from(offset).map_err(|_| Error::new(ErrorKind::FileTooLarge, ""))?
    };
    let end = start
        .checked_add(input.len())
        .ok_or_else(|| Error::new(ErrorKind::FileTooLarge, ""))?;
    if end as u64 > MAX_FILE_SIZE {
        return Err(Error::new(ErrorKind::FileTooLarge, ""));
    }
    if end > data.len() {
        data.resize(end, 0);
    }
    data[start..end].copy_from_slice(input);
    Ok(input.len())
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
