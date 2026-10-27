//! The request-scoped virtual filesystem exposed to Workers.

mod bundle;
mod nodes;
mod path;
#[cfg(test)]
mod tests;
mod virtual_filesystem;

#[cfg(test)]
use path::join_child;
pub(super) use path::{escape_at, escaped_in_names, push_escape};
pub(super) use virtual_filesystem::MAX_FILE_SIZE;
pub use virtual_filesystem::{
    Bundle, CopyOptions, DirectoryEntry, Error, ErrorKind, MAX_PATH_LENGTH, MAX_PATH_SEGMENTS,
    NodeType, OpenOptions, Result, Stat, VirtualFileSystem,
};
