//! In-memory nodes and character devices for the virtual filesystem.

use std::collections::btree_map::{BTreeMap, Entry};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};

use super::path::{checked_size, lock, read_bytes, read_memory, write_memory};
use super::{Bundle, DirectoryEntry, Error, ErrorKind, NodeType, Result, Stat};

#[derive(Clone)]
pub(super) struct OpenFile {
    pub(super) node: Node,
    pub(super) position: u64,
    pub(super) read: bool,
    pub(super) write: bool,
    pub(super) append: bool,
}

#[derive(Clone)]
pub(super) enum Node {
    File(FileNode),
    Directory(Directory),
    Symlink(String),
}

impl Node {
    pub(super) fn kind(&self) -> NodeType {
        match self {
            Self::File(_) => NodeType::File,
            Self::Directory(_) => NodeType::Directory,
            Self::Symlink(_) => NodeType::Symlink,
        }
    }

    pub(super) fn as_directory(&self) -> Option<Directory> {
        match self {
            Self::Directory(directory) => Some(directory.clone()),
            _ => None,
        }
    }

    pub(super) fn stat(&self) -> Result<Stat> {
        match self {
            Self::File(file) => file.stat(),
            Self::Directory(directory) => Ok(Stat {
                kind: NodeType::Directory,
                size: 0,
                writable: directory.writable(),
                device: false,
            }),
            Self::Symlink(target) => Ok(Stat {
                kind: NodeType::Symlink,
                size: target.len() as u64,
                writable: false,
                device: false,
            }),
        }
    }

    pub(super) fn read_all(&self, path: &str) -> Result<Vec<u8>> {
        match self {
            Self::File(file) => {
                let size = file.size()?;
                let length =
                    usize::try_from(size).map_err(|_| Error::new(ErrorKind::FileTooLarge, path))?;
                file.read(0, length)
            }
            Self::Directory(_) => Err(Error::new(ErrorKind::IsDirectory, path)),
            Self::Symlink(_) => Err(Error::new(ErrorKind::Unsupported, path)),
        }
    }

    pub(super) fn read_dir(&self, path: &str) -> Result<Vec<DirectoryEntry>> {
        match self {
            Self::Directory(directory) => directory.entries(),
            _ => Err(Error::new(ErrorKind::NotDirectory, path)),
        }
    }

    pub(super) fn read(&self, offset: u64, length: usize) -> Result<Vec<u8>> {
        match self {
            Self::File(file) => file.read(offset, length),
            Self::Directory(_) => Err(Error::new(ErrorKind::IsDirectory, "")),
            Self::Symlink(_) => Err(Error::new(ErrorKind::Unsupported, "")),
        }
    }

    pub(super) fn write(&self, offset: u64, data: &[u8], append: bool) -> Result<usize> {
        match self {
            Self::File(file) => file.write(offset, data, append),
            Self::Directory(_) => Err(Error::new(ErrorKind::IsDirectory, "")),
            Self::Symlink(_) => Err(Error::new(ErrorKind::Unsupported, "")),
        }
    }

    pub(super) fn is_device(&self) -> bool {
        matches!(self, Self::File(FileNode::Device(_)))
    }

    pub(super) fn size(&self) -> Result<u64> {
        match self {
            Self::File(file) => file.size(),
            Self::Directory(_) => Ok(0),
            Self::Symlink(target) => Ok(target.len() as u64),
        }
    }

    pub(super) fn resize(&self, size: u64, path: &str) -> Result<()> {
        match self {
            Self::File(file) => file.resize(size, path),
            Self::Directory(_) => Err(Error::new(ErrorKind::IsDirectory, path)),
            Self::Symlink(_) => Err(Error::new(ErrorKind::Unsupported, path)),
        }
    }
}

#[derive(Clone)]
pub(super) enum Directory {
    Memory {
        entries: Arc<Mutex<BTreeMap<String, Node>>>,
        writable: bool,
    },
    Bundle {
        bundle: Bundle,
        relative: PathBuf,
    },
    Static {
        entries: Arc<BTreeMap<String, Node>>,
    },
}

impl Directory {
    pub(super) fn memory(writable: bool) -> Self {
        Self::Memory {
            entries: Arc::new(Mutex::new(BTreeMap::new())),
            writable,
        }
    }

    pub(super) fn with_entries(writable: bool, entries: BTreeMap<String, Node>) -> Self {
        Self::Memory {
            entries: Arc::new(Mutex::new(entries)),
            writable,
        }
    }

    pub(super) fn bundle(bundle: Bundle, relative: PathBuf) -> Self {
        Self::Bundle { bundle, relative }
    }

    pub(super) fn lookup(&self, name: &str) -> Result<Option<Node>> {
        match self {
            Self::Memory { entries, .. } => Ok(lock(entries).get(name).cloned()),
            Self::Static { entries } => Ok(entries.get(name).cloned()),
            Self::Bundle { bundle, relative } => {
                let child = relative.join(name);
                bundle.node(&child)
            }
        }
    }

    pub(super) fn entries(&self) -> Result<Vec<DirectoryEntry>> {
        let mut entries = match self {
            Self::Memory { entries, .. } => lock(entries).iter().map(directory_entry).collect(),
            Self::Static { entries } => entries.iter().map(directory_entry).collect(),
            Self::Bundle { bundle, relative } => bundle.entries(relative)?,
        };
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(entries)
    }

    pub(super) fn insert(&self, name: String, node: Node) -> Result<()> {
        let Self::Memory {
            entries,
            writable: true,
        } = self
        else {
            return Err(Error::new(ErrorKind::ReadOnly, name));
        };
        match lock(entries).entry(name) {
            Entry::Occupied(entry) => Err(Error::new(ErrorKind::AlreadyExists, entry.key())),
            Entry::Vacant(entry) => {
                entry.insert(node);
                Ok(())
            }
        }
    }

    pub(super) fn remove(&self, name: &str) -> Result<Option<Node>> {
        match self {
            Self::Memory { entries, writable } if *writable => Ok(lock(entries).remove(name)),
            _ => Err(Error::new(ErrorKind::ReadOnly, name)),
        }
    }

    pub(super) fn is_empty(&self) -> Result<bool> {
        Ok(self.entries()?.is_empty())
    }

    pub(super) fn writable(&self) -> bool {
        match self {
            Self::Memory { writable, .. } => *writable,
            Self::Bundle { .. } | Self::Static { .. } => false,
        }
    }
}

pub(super) fn directory_entry((name, node): (&String, &Node)) -> DirectoryEntry {
    DirectoryEntry {
        name: name.clone(),
        kind: node.kind(),
        device: node.is_device(),
    }
}

#[derive(Clone)]
pub(super) enum FileNode {
    Memory(Arc<Mutex<Vec<u8>>>),
    Bundle { bundle: Bundle, relative: PathBuf },
    Device(Device),
}

impl FileNode {
    pub(super) fn memory() -> Self {
        Self::Memory(Arc::new(Mutex::new(Vec::new())))
    }

    pub(super) fn stat(&self) -> Result<Stat> {
        match self {
            Self::Memory(data) => Ok(Stat {
                kind: NodeType::File,
                size: lock(data).len() as u64,
                writable: true,
                device: false,
            }),
            Self::Bundle { bundle, relative } => {
                let metadata = bundle.metadata(relative)?;
                Ok(Stat {
                    kind: NodeType::File,
                    size: metadata.len(),
                    writable: false,
                    device: false,
                })
            }
            Self::Device(_) => Ok(Stat {
                kind: NodeType::File,
                size: 0,
                writable: true,
                device: true,
            }),
        }
    }

    pub(super) fn size(&self) -> Result<u64> {
        Ok(self.stat()?.size)
    }

    pub(super) fn writable(&self) -> bool {
        !matches!(self, Self::Bundle { .. })
    }

    pub(super) fn is_device(&self) -> bool {
        matches!(self, Self::Device(_))
    }

    pub(super) fn read(&self, offset: u64, length: usize) -> Result<Vec<u8>> {
        match self {
            Self::Memory(data) => Ok(read_memory(data, offset, length)),
            Self::Bundle { bundle, relative } => {
                let data = bundle.read(relative)?;
                Ok(read_bytes(&data, offset, length))
            }
            Self::Device(device) => device.read(length),
        }
    }

    pub(super) fn write(&self, offset: u64, data: &[u8], append: bool) -> Result<usize> {
        match self {
            Self::Memory(bytes) => write_memory(bytes, offset, data, append),
            Self::Bundle { .. } => Err(Error::new(ErrorKind::ReadOnly, "")),
            Self::Device(device) => device.write(data),
        }
    }

    pub(super) fn resize(&self, size: u64, path: &str) -> Result<()> {
        match self {
            Self::Memory(data) => {
                let size = checked_size(size, path)?;
                lock(data).resize(size, 0);
                Ok(())
            }
            Self::Bundle { .. } => Err(Error::new(ErrorKind::ReadOnly, path)),
            Self::Device(Device::Null | Device::Zero) => Ok(()),
            Self::Device(Device::Full | Device::Random) => {
                Err(Error::new(ErrorKind::NotPermitted, path))
            }
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Device {
    Null,
    Zero,
    Full,
    Random,
}

impl Device {
    pub(super) fn read(self, length: usize) -> Result<Vec<u8>> {
        match self {
            Self::Null => Ok(Vec::new()),
            Self::Zero | Self::Full => Ok(vec![0; length]),
            Self::Random => {
                let mut data = vec![0; length];
                getrandom::fill(&mut data).map_err(|error| {
                    Error::with_message(ErrorKind::Entropy, "", error.to_string())
                })?;
                Ok(data)
            }
        }
    }

    pub(super) fn write(self, data: &[u8]) -> Result<usize> {
        match self {
            Self::Null | Self::Zero => Ok(data.len()),
            Self::Full => Err(Error::new(ErrorKind::NoSpace, "")),
            Self::Random => Err(Error::new(ErrorKind::NotPermitted, "")),
        }
    }
}

pub(super) fn shared_devices() -> Directory {
    static DEVICES: LazyLock<Directory> = LazyLock::new(|| Directory::Static {
        entries: Arc::new(BTreeMap::from([
            (
                "full".to_owned(),
                Node::File(FileNode::Device(Device::Full)),
            ),
            (
                "null".to_owned(),
                Node::File(FileNode::Device(Device::Null)),
            ),
            (
                "random".to_owned(),
                Node::File(FileNode::Device(Device::Random)),
            ),
            (
                "zero".to_owned(),
                Node::File(FileNode::Device(Device::Zero)),
            ),
        ])),
    });
    DEVICES.clone()
}
