#![allow(clippy::missing_errors_doc)]

//! A small in-memory filesystem for the tokamak Worker contract.

use std::collections::BTreeMap;
use std::fmt::{self, Display, Formatter};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use indexmap::IndexMap;

use super::nodes::{Directory, FileNode, Node, OpenFile, shared_devices};
use super::path::{
    absolute_components, join_child, path_from_components, random_suffix, target_components,
};

/// Maximum size of one virtual file.
pub const MAX_FILE_SIZE: u64 = 128 * 1024 * 1024;
/// Largest explicit position a non-appending write accepts: `u32::MAX`.
const MAX_WRITE_POSITION: u64 = 0xFFFF_FFFF;
/// Maximum length of a virtual path in Unicode scalar values.
pub const MAX_PATH_LENGTH: usize = 4096;
/// Maximum number of non-empty path segments.
pub const MAX_PATH_SEGMENTS: usize = 48;

/// The kind of a virtual node.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeType {
    /// A regular file.
    File,
    /// A directory.
    Directory,
    /// A symbolic link.
    Symlink,
}

/// A filesystem error category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    /// The path or descriptor does not exist.
    NotFound,
    /// A path component is not a directory.
    NotDirectory,
    /// A directory was used as a file.
    IsDirectory,
    /// An entry already exists.
    AlreadyExists,
    /// A directory is not empty.
    NotEmpty,
    /// The operation is not permitted.
    NotPermitted,
    /// The operation would exceed the file-size limit, reported as `EPERM`
    /// as workerd reports it.
    FileTooLarge,
    /// The operation would exceed the available virtual space.
    NoSpace,
    /// The path is invalid or escapes the virtual root.
    InvalidPath,
    /// The input is invalid for the requested operation.
    InvalidInput,
    /// A descriptor is invalid or already closed.
    InvalidDescriptor,
    /// Too many symbolic links were followed.
    SymlinkLoop,
    /// The operation is not implemented by this filesystem.
    Unsupported,
    /// The host bundle could not be read.
    Host,
    /// The random source failed.
    Entropy,
    /// The filesystem failed internally, reported without a code.
    Internal,
}

impl ErrorKind {
    /// Return the Node-style error code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::NotFound => "ENOENT",
            Self::NotDirectory => "ENOTDIR",
            Self::IsDirectory => "EISDIR",
            Self::AlreadyExists => "EEXIST",
            Self::NotEmpty => "ENOTEMPTY",
            Self::NotPermitted | Self::FileTooLarge => "EPERM",
            Self::NoSpace => "ENOSPC",
            Self::InvalidPath | Self::InvalidInput => "EINVAL",
            Self::InvalidDescriptor => "EBADF",
            Self::SymlinkLoop => "ELOOP",
            Self::Unsupported => "ENOSYS",
            Self::Host | Self::Entropy => "EIO",
            Self::Internal => "",
        }
    }

    /// The kind's message, and whether the system call and path follow it as
    /// they follow libuv's messages; workerd gives some kinds its own message.
    const fn message(self) -> (&'static str, bool) {
        match self {
            Self::NotFound => ("no such file or directory", true),
            Self::NotDirectory => ("not a directory", true),
            Self::IsDirectory => ("illegal operation on a directory", true),
            Self::AlreadyExists => ("file already exists", true),
            Self::NotEmpty => ("directory not empty", true),
            Self::NotPermitted => ("operation not permitted", true),
            Self::FileTooLarge => ("File size limit exceeded", false),
            Self::NoSpace => ("no space left on device", true),
            Self::InvalidPath => ("Invalid path", false),
            Self::InvalidInput => ("invalid argument", true),
            Self::InvalidDescriptor => ("bad file descriptor", true),
            Self::SymlinkLoop => ("symlink depth exceeded", false),
            Self::Unsupported => ("function not implemented", true),
            Self::Host | Self::Entropy => ("input/output error", true),
            Self::Internal => ("internal error", false),
        }
    }
}

/// A virtual filesystem failure, as workerd's `node:fs` reports it.
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    /// The path workerd reports with the failure, or empty for none.
    path: String,
    /// A message in place of the kind's.
    message: Option<String>,
}

impl Error {
    pub(crate) const fn new(kind: ErrorKind) -> Self {
        Self {
            kind,
            path: String::new(),
            message: None,
        }
    }

    pub(super) fn with_message(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            path: String::new(),
            message: Some(message.into()),
        }
    }

    pub(super) fn from_io(error: &std::io::Error) -> Self {
        let kind = match error.kind() {
            std::io::ErrorKind::NotFound => ErrorKind::NotFound,
            std::io::ErrorKind::PermissionDenied => ErrorKind::NotPermitted,
            std::io::ErrorKind::AlreadyExists => ErrorKind::AlreadyExists,
            std::io::ErrorKind::InvalidInput => ErrorKind::InvalidInput,
            _ => ErrorKind::Host,
        };
        Self::with_message(kind, error.to_string())
    }

    /// This error reported with `path`, as workerd reports some failures.
    #[must_use]
    pub(crate) fn at(self, path: impl Display) -> Self {
        Self {
            path: path.to_string(),
            ..self
        }
    }

    /// Return the error category.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Return the Node-style error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.kind.code()
    }

    /// Return the path workerd reports with the error, or an empty string.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The message of this failure in the system call `syscall`.
    #[must_use]
    pub fn message(&self, syscall: &str) -> String {
        if let Some(message) = &self.message {
            return message.clone();
        }
        match self.kind.message() {
            (message, true) if !self.path.is_empty() => {
                format!("{message}, {syscall} '{}'", self.path)
            }
            (message, _) => message.to_owned(),
        }
    }
}

impl Display for Error {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code(), self.message("vfs"))
    }
}

impl std::error::Error for Error {}

/// Result type used by the virtual filesystem.
pub type Result<T> = std::result::Result<T, Error>;

/// Metadata returned for a virtual node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Stat {
    /// Node kind.
    pub kind: NodeType,
    /// File length in bytes.
    pub size: u64,
    /// Whether the node accepts ordinary writes.
    pub writable: bool,
    /// Whether the file is a synthetic device.
    pub device: bool,
}

/// One entry returned when reading a directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryEntry {
    /// Entry name, without its parent path.
    pub name: String,
    /// Entry kind.
    pub kind: NodeType,
    /// Whether the entry is a synthetic character device.
    pub device: bool,
}

/// The immutable bundle directory shared by virtual filesystem instances.
#[derive(Clone, Debug)]
pub struct Bundle {
    pub(super) root: Arc<Path>,
}

impl Bundle {
    /// Create a bundle view over a tokamak `bundle` directory.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: Arc::from(root.into()),
        }
    }
}

/// Options used when opening a virtual file.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpenOptions {
    /// Open for reading.
    pub read: bool,
    /// Open for writing.
    pub write: bool,
    /// Place writes at the end of the file.
    pub append: bool,
    /// Create a missing file.
    pub create: bool,
    /// Fail if the file already exists.
    pub exclusive: bool,
    /// Truncate an existing file after opening.
    pub truncate: bool,
    /// Follow the final symbolic link.
    pub follow_symlinks: bool,
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            read: true,
            write: false,
            append: false,
            create: false,
            exclusive: false,
            truncate: false,
            follow_symlinks: true,
        }
    }
}

/// Options used when copying a file or directory tree.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CopyOptions {
    /// Copy directories recursively.
    pub recursive: bool,
    /// Replace an existing destination.
    pub force: bool,
    /// Fail when the destination already exists.
    pub error_on_exist: bool,
    /// Follow symbolic links in the source.
    pub dereference: bool,
}

/// A request-owned virtual filesystem.
pub struct VirtualFileSystem {
    root: Directory,
    descriptors: BTreeMap<u32, OpenFile>,
    next_descriptor: u32,
    working_directory: String,
}

impl VirtualFileSystem {
    /// Create a filesystem with a shared readonly bundle and fresh `/tmp`.
    #[must_use]
    pub fn new(bundle: Bundle) -> Self {
        let tmp = Directory::memory();
        let bundle = Node::Directory(Directory::bundle(bundle, PathBuf::new()));
        let dev = Node::Directory(shared_devices());
        let root = Directory::with_entries(
            false,
            IndexMap::from([
                ("bundle".to_owned(), bundle),
                ("tmp".to_owned(), Node::Directory(tmp)),
                ("dev".to_owned(), dev),
            ]),
        );
        Self {
            root,
            descriptors: BTreeMap::new(),
            next_descriptor: 3,
            working_directory: "/bundle".to_owned(),
        }
    }

    /// The directory relative paths resolve against.
    #[must_use]
    pub fn working_directory(&self) -> &str {
        &self.working_directory
    }

    /// Make the directory `path` names, relative to the working directory,
    /// the working directory. Errors name the directory without its leading
    /// slash.
    pub fn change_directory(&mut self, path: &str) -> Result<()> {
        if path.is_empty() {
            return Err(Error::new(ErrorKind::NotFound));
        }
        let mut components = if path.starts_with('/') {
            Vec::new()
        } else {
            absolute_components(&self.working_directory)?
        };
        for part in path.split('/') {
            match part {
                "" | "." => {}
                ".." => components
                    .pop()
                    .map(drop)
                    .ok_or_else(|| Error::new(ErrorKind::Internal))?,
                part => components.push(part.to_owned()),
            }
        }
        let named = components.join("/");
        match self.resolve(&path_from_components(&components), true) {
            Ok(Node::Directory(_)) => {
                self.working_directory = path_from_components(&components);
                Ok(())
            }
            Ok(_) => Err(Error::new(ErrorKind::NotDirectory).at(named)),
            Err(error) if error.kind() == ErrorKind::NotDirectory => {
                Err(Error::new(ErrorKind::NotFound).at(named))
            }
            Err(error) => Err(error.at(named)),
        }
    }

    /// Read a file completely.
    pub fn read_file(&self, path: &str) -> Result<Vec<u8>> {
        self.resolve(path, true)
            .map_err(|error| missing_at(error, path))?
            .read_all()
    }

    /// Return metadata, following the final symbolic link.
    pub fn stat(&self, path: &str) -> Result<Stat> {
        self.resolve(path, true)?.stat()
    }

    /// Return metadata without following the final symbolic link.
    pub fn lstat(&self, path: &str) -> Result<Stat> {
        self.resolve(path, false)?.stat()
    }

    /// List a directory in the order its entries are stored.
    pub fn read_dir(&self, path: &str) -> Result<Vec<DirectoryEntry>> {
        self.listed_directory(path)?.entries()
    }

    /// List a directory tree depth first, each directory in its stored order,
    /// descending into symbolic links to directories under the link's path.
    pub fn walk(&self, path: &str) -> Result<Vec<(String, DirectoryEntry)>> {
        let directory = self.listed_directory(path)?;
        let mut entries = Vec::new();
        self.walk_directory(path, &directory, &mut vec![path.to_owned()], &mut entries)?;
        Ok(entries)
    }

    /// The directory `path` names without passing through a symbolic link.
    /// Failures name `path`.
    fn listed_directory(&self, path: &str) -> Result<Directory> {
        self.listed_directory_components(path)
            .map_err(|error| error.at(path))
    }

    fn listed_directory_components(&self, path: &str) -> Result<Directory> {
        let components = absolute_components(path)?;
        let mut directory = self.root.clone();
        for (index, name) in components.iter().enumerate() {
            let final_component = index + 1 == components.len();
            directory = match directory.lookup(name)? {
                Some(Node::Directory(child)) => child,
                Some(Node::Symlink(_)) if final_component => {
                    return Err(Error::new(ErrorKind::InvalidInput));
                }
                Some(Node::File(_)) if final_component => {
                    return Err(Error::new(ErrorKind::NotDirectory));
                }
                _ => return Err(Error::new(ErrorKind::NotFound)),
            };
        }
        Ok(directory)
    }

    /// Check whether a path satisfies the requested access mode.
    pub fn access(&self, path: &str, mode: u32) -> Result<()> {
        if mode & !7 != 0 {
            return Err(Error::new(ErrorKind::InvalidInput));
        }
        if mode & 1 != 0 {
            return Err(Error::new(ErrorKind::NotFound));
        }
        let stat = self.stat(path)?;
        if mode & 2 != 0 && !stat.writable {
            return Err(Error::new(ErrorKind::NotFound));
        }
        Ok(())
    }

    /// Create a directory, leaving an existing directory alone.
    pub fn mkdir(&mut self, path: &str, recursive: bool) -> Result<()> {
        let components = absolute_components(path)?;
        match self.resolve(path, false) {
            Ok(Node::Directory(_)) => return Ok(()),
            Ok(_) => return Err(Error::new(ErrorKind::AlreadyExists)),
            Err(error) if error.kind() != ErrorKind::NotFound => return Err(error),
            Err(_) => {}
        }
        if recursive {
            return self.mkdir_recursive(&components);
        }
        let (base, name) = split_name(path)?;
        let parent = self.directory(base).map_err(|error| error.at(&name))?;
        parent.insert(name, Node::Directory(Directory::memory()))
    }

    /// Write `data` as the content of the file `path` names, creating the file
    /// when it is missing, as workerd's `writeAll` does: `append` writes after
    /// the existing content, and `exclusive` fails when the file exists.
    pub fn write_file(
        &mut self,
        path: &str,
        data: &[u8],
        append: bool,
        exclusive: bool,
    ) -> Result<()> {
        let node = match self.resolve(path, true) {
            Ok(_) if exclusive => return Err(Error::new(ErrorKind::AlreadyExists)),
            Ok(node) => node,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                let (base, name) = split_name(path)?;
                let parent = self
                    .directory(base)
                    .map_err(|error| missing_at(error, path))?;
                if !parent.writable() {
                    return Err(Error::new(ErrorKind::NotPermitted));
                }
                let node = Node::File(FileNode::memory());
                node.write(0, data, false)?;
                return parent.insert(name, node);
            }
            Err(error) => return Err(error),
        };
        let file = node.file()?;
        if !file.writable() {
            return Err(Error::new(ErrorKind::NotPermitted));
        }
        if append {
            file.write(0, data, true)?;
        } else {
            file.resize(0)?;
            file.write(0, data, false)?;
        }
        Ok(())
    }

    /// Remove the entry `path` names without following a final symbolic
    /// link, as workerd's `rm` does: a missing entry is left alone unless
    /// `directory_only`, and a directory that is not empty needs `recursive`.
    pub fn remove(&mut self, path: &str, recursive: bool, directory_only: bool) -> Result<()> {
        let (base, name) = split_name(path)?;
        let parent = self.directory(base).map_err(|error| error.at(&name))?;
        if !parent.writable() {
            return Err(Error::new(ErrorKind::NotPermitted));
        }
        let node = parent.lookup(&name)?;
        if directory_only {
            match node {
                None => return Err(Error::new(ErrorKind::NotFound).at(&name)),
                Some(Node::Directory(_)) => {}
                Some(_) => return Err(Error::new(ErrorKind::NotDirectory)),
            }
        }
        match node {
            Some(Node::Directory(directory)) if !recursive && !directory.is_empty()? => {
                Err(Error::new(ErrorKind::NotEmpty).at(&name))
            }
            Some(_) => parent.remove(&name).map(drop),
            None => Ok(()),
        }
    }

    /// Remove a regular file or symbolic link.
    pub fn remove_file(&mut self, path: &str) -> Result<()> {
        let not_found =
            |name: &str| Error::with_message(ErrorKind::NotFound, "File not found").at(name);
        let (base, name) = split_name(path)?;
        let parent = self.directory(base).map_err(|error| match error.kind() {
            ErrorKind::NotFound => not_found(&name),
            ErrorKind::NotDirectory => {
                Error::with_message(ErrorKind::NotDirectory, "Parent path is not a directory")
            }
            _ => error,
        })?;
        match parent.lookup(&name)? {
            None => Err(not_found(&name)),
            Some(Node::Directory(_)) => Err(Error::with_message(
                ErrorKind::IsDirectory,
                "Cannot unlink a directory",
            )),
            Some(_) => parent.remove(&name).map(drop),
        }
    }

    /// Move the node `from` names, following symbolic links, to `to`,
    /// replacing what `to` names without following symbolic links. A
    /// directory replaces only an empty directory, and anything else only a
    /// non-directory.
    pub fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        let replaced = self.node_without_links(to)?;
        let to_components = absolute_components(to)?;
        let from_components = absolute_components(from)?;
        if replaced.is_some() && to_components == from_components {
            return Ok(());
        }
        let (parent, name) = self.containing_directory(to)?;
        let (source_parent, source_name) = self.containing_directory(from)?;
        if !source_parent.writable() {
            return Err(Error::new(ErrorKind::NotPermitted));
        }
        let source = self
            .lookup(from)?
            .ok_or_else(|| Error::new(ErrorKind::NotFound).at(from))?;
        if matches!(source, Node::Directory(_)) {
            if to_components.len() > from_components.len()
                && to_components.starts_with(&from_components)
            {
                return Err(Error::new(ErrorKind::InvalidInput));
            }
            check_directory_replacement(replaced.as_ref())?;
        } else {
            check_file_replacement(replaced.as_ref())?;
        }
        replace_entry(&parent, name, replaced.is_some(), source)?;
        source_parent.remove(&source_name)?;
        Ok(())
    }

    /// Copy the file `from` names, following symbolic links, to `to`,
    /// replacing the non-directory `to` names without following symbolic
    /// links. A device copies as an empty file.
    pub fn copy_file(&mut self, from: &str, to: &str) -> Result<()> {
        let replaced = self.node_without_links(to)?;
        let (parent, name) = self.containing_directory(to)?;
        let source = self
            .lookup(from)?
            .ok_or_else(|| Error::new(ErrorKind::NotFound).at(from))?;
        let Node::File(file) = source else {
            return Err(Error::new(ErrorKind::IsDirectory));
        };
        check_file_replacement(replaced.as_ref())?;
        let copy = Node::File(file.copied()?);
        replace_entry(&parent, name, replaced.is_some(), copy)
    }

    /// Copy a file or directory tree.
    pub fn copy(&mut self, from: &str, to: &str, options: CopyOptions) -> Result<()> {
        let source = self
            .resolve(from, options.dereference)
            .map_err(|error| missing_at(error, from))?;
        let target = self.copy_target(from, to, &source)?;
        let target_exists = self.resolve(&target, false).is_ok();
        if target_exists && options.error_on_exist {
            return Err(Error::new(ErrorKind::AlreadyExists));
        }
        if target_exists && !options.force {
            return Ok(());
        }
        if matches!(source, Node::Directory(_)) && !options.recursive {
            return Err(Error::new(ErrorKind::IsDirectory));
        }
        let node = self.copy_node(&source, from, options.dereference)?;
        let (parent, name) = self.parent(&target)?;
        if target_exists {
            parent.remove(&name)?;
        }
        parent.insert(name, node)
    }

    /// Create a hard link at `new` to what `existing` names without following
    /// a final symbolic link.
    pub fn link(&mut self, existing: &str, new: &str) -> Result<()> {
        let (parent, name) = self.link_parent(new, existing)?;
        let source = self
            .resolve(existing, false)
            .map_err(|error| match error.kind() {
                ErrorKind::NotFound => {
                    Error::with_message(ErrorKind::NotFound, "File not found").at(existing)
                }
                _ => error,
            })?;
        if matches!(source, Node::Directory(_)) {
            return Err(Error::with_message(
                ErrorKind::NotPermitted,
                "Cannot hardlink to a directory",
            ));
        }
        parent.insert(name, source)
    }

    /// The directory a link at `path` to `target` goes in, and its name,
    /// failing as workerd's `link` does when `path` already exists.
    fn link_parent(&self, path: &str, target: &str) -> Result<(Directory, String)> {
        match self.resolve(path, true) {
            Ok(_) => {
                return Err(Error::with_message(
                    ErrorKind::AlreadyExists,
                    "File already exists",
                ));
            }
            Err(error) if error.kind() != ErrorKind::NotFound => return Err(error),
            Err(_) => {}
        }
        self.parent(path).map_err(|error| match error.kind() {
            ErrorKind::NotFound => {
                Error::with_message(ErrorKind::NotFound, "Directory does not exist").at(target)
            }
            ErrorKind::NotDirectory => {
                Error::with_message(ErrorKind::InvalidInput, "Not a directory")
            }
            _ => error,
        })
    }

    /// Create a unique temporary directory below an existing parent.
    pub fn make_temp_dir(&mut self, prefix: &str) -> Result<String> {
        let (mut components, name) = split_name(prefix)?;
        let parent = self
            .directory(components.clone())
            .map_err(|error| missing_at(error, &name))?;
        let mut suffix = [0_u8; 6];
        for _ in 0..100 {
            getrandom::fill(&mut suffix)
                .map_err(|error| Error::with_message(ErrorKind::Entropy, error.to_string()))?;
            let candidate = format!("{name}{}", random_suffix(&suffix));
            if parent.lookup(&candidate)?.is_none() {
                parent.insert(candidate.clone(), Node::Directory(Directory::memory()))?;
                components.push(candidate);
                return Ok(path_from_components(&components));
            }
        }
        Err(Error::new(ErrorKind::AlreadyExists))
    }

    /// Create a symbolic link.
    pub fn symlink(&mut self, target: &str, path: &str) -> Result<()> {
        if target.contains('\0') {
            return Err(Error::new(ErrorKind::InvalidInput));
        }
        let (parent, name) = self.link_parent(path, target)?;
        parent.insert(name, Node::Symlink(target.to_owned()))
    }

    /// Read a symbolic link target.
    pub fn read_link(&self, path: &str) -> Result<String> {
        match self
            .resolve(path, false)
            .map_err(|error| missing_at(error, path))?
        {
            Node::Symlink(target) => Ok(target),
            _ => Err(Error::new(ErrorKind::InvalidInput)),
        }
    }

    /// The target of the symbolic link `path` names, or else `path`
    /// normalized, as workerd's `realpath` reports it: only a final symbolic
    /// link is read, and only once.
    pub fn realpath(&self, path: &str) -> Result<String> {
        match self
            .resolve(path, false)
            .map_err(|error| missing_at(error, path))?
        {
            Node::Symlink(target) => Ok(target),
            _ => Ok(path_from_components(&absolute_components(path)?)),
        }
    }

    /// Open a file and return its descriptor. Failures name `path`.
    pub fn open(&mut self, path: &str, options: OpenOptions) -> Result<u32> {
        self.open_file(path, options)
            .map_err(|error| error.at(path))
    }

    fn open_file(&mut self, path: &str, options: OpenOptions) -> Result<u32> {
        if !options.read && !options.write {
            return Err(Error::new(ErrorKind::InvalidInput));
        }
        let (node, existed) = match self.resolve(path, options.follow_symlinks) {
            Ok(node) => (node, true),
            Err(error) if error.kind() == ErrorKind::NotFound && options.create => {
                let (parent, name) = self.parent(path)?;
                if parent.lookup(&name)?.is_some() {
                    return Err(Error::new(ErrorKind::NotFound));
                }
                let node = Node::File(FileNode::memory());
                parent.insert(name, node.clone())?;
                (node, false)
            }
            Err(error) => return Err(error),
        };
        if existed && options.exclusive {
            return Err(Error::new(ErrorKind::AlreadyExists));
        }
        let file = node.file()?;
        if options.write && !file.writable() {
            return Err(Error::new(ErrorKind::NotPermitted));
        }
        if options.truncate && options.write {
            file.resize(0)?;
        }
        let descriptor = self.next_descriptor;
        self.next_descriptor = self
            .next_descriptor
            .checked_add(1)
            .ok_or_else(|| Error::new(ErrorKind::InvalidDescriptor))?;
        self.descriptors.insert(
            descriptor,
            OpenFile {
                node,
                position: 0,
                read: options.read,
                write: options.write,
                append: options.append,
            },
        );
        Ok(descriptor)
    }

    /// Close a descriptor; closing one that is not open does nothing.
    pub fn close(&mut self, descriptor: u32) {
        self.descriptors.remove(&descriptor);
    }

    /// Return descriptor metadata.
    pub fn fstat(&self, descriptor: u32) -> Result<Stat> {
        self.descriptor(descriptor)?.node.stat()
    }

    /// Read from a descriptor. `position` does not move the descriptor cursor.
    pub fn read(
        &mut self,
        descriptor: u32,
        length: usize,
        position: Option<u64>,
    ) -> Result<Vec<u8>> {
        let open = self.descriptor_mut(descriptor)?;
        if !open.read {
            return Err(Error::new(ErrorKind::NotPermitted));
        }
        let offset = position.unwrap_or(open.position);
        let bytes = open.node.read(offset, length)?;
        if position.is_none() {
            open.position = offset.saturating_add(bytes.len() as u64);
        }
        Ok(bytes)
    }

    /// Write to a descriptor. `position` does not move the descriptor cursor.
    pub fn write(&mut self, descriptor: u32, data: &[u8], position: Option<u64>) -> Result<usize> {
        let open = self.descriptor_mut(descriptor)?;
        if !open.write {
            return Err(Error::new(ErrorKind::NotPermitted));
        }
        if !open.append && position.is_some_and(|position| position > MAX_WRITE_POSITION) {
            return Err(Error::with_message(
                ErrorKind::InvalidInput,
                "position out of range",
            ));
        }
        let offset = position.unwrap_or(open.position);
        let written = open.node.write(offset, data, open.append)?;
        if position.is_none() {
            open.position = if open.append {
                open.node.size()?
            } else {
                offset.saturating_add(written as u64)
            };
        }
        Ok(written)
    }

    /// Read from a descriptor into multiple buffers.
    pub fn readv(
        &mut self,
        descriptor: u32,
        lengths: &[usize],
        position: Option<u64>,
    ) -> Result<Vec<Vec<u8>>> {
        self.descriptor(descriptor)?;
        let mut offset = position;
        let mut result = Vec::with_capacity(lengths.len());
        for &length in lengths {
            let bytes = self.read(descriptor, length, offset)?;
            if let Some(value) = &mut offset {
                *value = value.saturating_add(bytes.len() as u64);
            }
            result.push(bytes);
        }
        Ok(result)
    }

    /// Write multiple buffers to a descriptor.
    pub fn writev(
        &mut self,
        descriptor: u32,
        buffers: &[Vec<u8>],
        position: Option<u64>,
    ) -> Result<usize> {
        self.descriptor(descriptor)?;
        let mut offset = position;
        let mut written = 0;
        for buffer in buffers {
            let count = self.write(descriptor, buffer, offset)?;
            written += count;
            if let Some(value) = &mut offset {
                *value = value.saturating_add(count as u64);
            }
        }
        Ok(written)
    }

    /// Truncate a path.
    pub fn truncate(&mut self, path: &str, size: u64) -> Result<()> {
        self.resolve(path, true)
            .map_err(|error| missing_at(error, path))?
            .resize(size)
    }

    /// Truncate a descriptor.
    pub fn ftruncate(&mut self, descriptor: u32, size: u64) -> Result<()> {
        let open = self.descriptor(descriptor)?;
        if !open.write {
            return Err(Error::new(ErrorKind::NotPermitted));
        }
        open.node.resize(size)
    }

    fn descriptor(&self, descriptor: u32) -> Result<&OpenFile> {
        self.descriptors
            .get(&descriptor)
            .ok_or_else(|| Error::new(ErrorKind::InvalidDescriptor))
    }

    fn descriptor_mut(&mut self, descriptor: u32) -> Result<&mut OpenFile> {
        self.descriptors
            .get_mut(&descriptor)
            .ok_or_else(|| Error::new(ErrorKind::InvalidDescriptor))
    }

    fn mkdir_recursive(&mut self, components: &[String]) -> Result<()> {
        let mut current = self.root.clone();
        for component in components {
            current = match current.lookup(component)? {
                Some(Node::Directory(directory)) => directory,
                Some(_) => return Err(Error::new(ErrorKind::NotDirectory)),
                None => {
                    let child = Directory::memory();
                    current.insert(component.clone(), Node::Directory(child.clone()))?;
                    child
                }
            };
        }
        Ok(())
    }

    /// Appends the tree below `directory`, reached at `path`, to `entries`.
    /// `ancestors` holds the real paths of the directories being walked, which
    /// a symbolic link may not lead back to.
    fn walk_directory(
        &self,
        path: &str,
        directory: &Directory,
        ancestors: &mut Vec<String>,
        entries: &mut Vec<(String, DirectoryEntry)>,
    ) -> Result<()> {
        for entry in directory.entries()? {
            let child = join_child(path, &entry.name);
            entries.push((child.clone(), entry));
            let Ok(Node::Directory(child_directory)) = self.resolve(&child, true) else {
                continue;
            };
            let real_path = path_from_components(&self.resolve_with_path(&child, true)?.1);
            if ancestors.contains(&real_path) {
                return Err(Error::new(ErrorKind::SymlinkLoop));
            }
            ancestors.push(real_path);
            self.walk_directory(&child, &child_directory, ancestors, entries)?;
            ancestors.pop();
        }
        Ok(())
    }

    fn copy_target(&self, from: &str, to: &str, source: &Node) -> Result<String> {
        let destination = self.resolve(to, true);
        if destination
            .as_ref()
            .is_ok_and(|node| matches!(node, Node::Directory(_)))
        {
            let name = absolute_components(from)?
                .pop()
                .ok_or_else(|| Error::new(ErrorKind::InvalidInput))?;
            return Ok(join_child(to, &name));
        }
        if matches!(source, Node::Directory(_)) && destination.is_err() {
            let (_, name) = self.parent(to)?;
            if name.is_empty() {
                return Err(Error::new(ErrorKind::InvalidInput));
            }
        }
        Ok(to.to_owned())
    }

    /// A copy of `node`, which `path` names. A directory holds itself once a
    /// symbolic link to it is renamed into it; copying one ends at the path
    /// segment limit.
    fn copy_node(&self, node: &Node, path: &str, dereference: bool) -> Result<Node> {
        match node {
            Node::File(file) if file.is_device() => Err(Error::new(ErrorKind::Unsupported)),
            Node::File(file) => Ok(Node::File(file.copied()?)),
            Node::Symlink(_) if !dereference => Ok(node.clone()),
            Node::Symlink(_) => {
                let resolved = self.resolve(path, true)?;
                self.copy_node(&resolved, path, dereference)
            }
            Node::Directory(directory) => {
                let target = Directory::memory();
                for entry in directory.entries()? {
                    let child_path = join_child(path, &entry.name);
                    absolute_components(&child_path)?;
                    let child = directory
                        .lookup(&entry.name)?
                        .ok_or_else(|| Error::new(ErrorKind::NotFound))?;
                    target.insert(
                        entry.name,
                        self.copy_node(&child, &child_path, dereference)?,
                    )?;
                }
                Ok(Node::Directory(target))
            }
        }
    }

    /// The directory holding the last component of `path`, and that
    /// component's name.
    fn parent(&self, path: &str) -> Result<(Directory, String)> {
        let (base, name) = split_name(path)?;
        Ok((self.directory(base)?, name))
    }

    /// The directory `components` name, following symbolic links.
    fn directory(&self, components: Vec<String>) -> Result<Directory> {
        let (node, _) = self.resolve_components(components, true)?;
        node.as_directory()
            .ok_or_else(|| Error::new(ErrorKind::NotDirectory))
    }

    /// The directory holding the last component of `path`, and that
    /// component's name; `/` is held by itself under an empty name. A missing
    /// directory is reported under the name.
    fn containing_directory(&self, path: &str) -> Result<(Directory, String)> {
        let mut components = absolute_components(path)?;
        let name = components.pop().unwrap_or_default();
        match self.lookup_components(components)? {
            Some(Node::Directory(directory)) => Ok((directory, name)),
            Some(_) => Err(Error::new(ErrorKind::NotDirectory)),
            None => Err(Error::new(ErrorKind::NotFound).at(name)),
        }
    }

    /// The node `path` names, following symbolic links, or `None` when it,
    /// or a directory on the way to it, does not exist.
    fn lookup(&self, path: &str) -> Result<Option<Node>> {
        self.lookup_components(absolute_components(path)?)
    }

    /// The node `path` names without following any symbolic link, or `None`
    /// when a component on the way to it is missing or is not a directory.
    fn node_without_links(&self, path: &str) -> Result<Option<Node>> {
        let mut node = Node::Directory(self.root.clone());
        for name in absolute_components(path)? {
            let Node::Directory(directory) = node else {
                return Ok(None);
            };
            let Some(child) = directory.lookup(&name)? else {
                return Ok(None);
            };
            node = child;
        }
        Ok(Some(node))
    }

    fn lookup_components(&self, components: Vec<String>) -> Result<Option<Node>> {
        match self.resolve_components(components, true) {
            Ok((node, _)) => Ok(Some(node)),
            Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotDirectory) => {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    fn resolve(&self, path: &str, follow_final: bool) -> Result<Node> {
        self.resolve_with_path(path, follow_final)
            .map(|(node, _)| node)
    }

    fn resolve_with_path(&self, path: &str, follow_final: bool) -> Result<(Node, Vec<String>)> {
        self.resolve_components(absolute_components(path)?, follow_final)
    }

    fn resolve_components(
        &self,
        mut pending: Vec<String>,
        follow_final: bool,
    ) -> Result<(Node, Vec<String>)> {
        let root = Node::Directory(self.root.clone());
        let mut current = root.clone();
        let mut resolved = Vec::new();
        let mut index = 0;
        let mut symlinks = 0;
        loop {
            if index == pending.len() {
                return Ok((current, resolved));
            }
            let name = pending[index].clone();
            let directory = current
                .as_directory()
                .ok_or_else(|| Error::new(ErrorKind::NotDirectory))?;
            let child = directory
                .lookup(&name)?
                .ok_or_else(|| Error::new(ErrorKind::NotFound))?;
            let final_component = index + 1 == pending.len();
            if let Node::Symlink(target) = &child
                && (follow_final || !final_component)
            {
                symlinks += 1;
                if symlinks > 40 {
                    return Err(Error::new(ErrorKind::SymlinkLoop));
                }
                let mut next = target_components(&resolved, target)?;
                next.extend_from_slice(&pending[index + 1..]);
                pending = next;
                current = root.clone();
                resolved.clear();
                index = 0;
                continue;
            }
            current = child;
            resolved.push(name);
            index += 1;
        }
    }
}

/// The components of the directory holding the last component of `path`,
/// and that component's name, as workerd splits a path it creates or removes
/// an entry at.
fn split_name(path: &str) -> Result<(Vec<String>, String)> {
    let mut components = absolute_components(path)?;
    let name = components
        .pop()
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput))?;
    Ok((components, name))
}

/// `error`, naming `path` when nothing exists there, as workerd reports a
/// missing path.
fn missing_at(error: Error, path: &str) -> Error {
    match error.kind() {
        ErrorKind::NotFound => error.at(path),
        _ => error,
    }
}

/// Rejects replacing the directory `replaced` with a non-directory.
fn check_file_replacement(replaced: Option<&Node>) -> Result<()> {
    match replaced {
        Some(Node::Directory(_)) => Err(Error::new(ErrorKind::IsDirectory)),
        _ => Ok(()),
    }
}

/// Rejects replacing a non-directory or a non-empty directory `replaced`
/// with a directory.
fn check_directory_replacement(replaced: Option<&Node>) -> Result<()> {
    match replaced {
        Some(Node::Directory(directory)) if !directory.is_empty()? => {
            Err(Error::new(ErrorKind::NotEmpty))
        }
        Some(Node::File(_) | Node::Symlink(_)) => Err(Error::new(ErrorKind::NotDirectory)),
        _ => Ok(()),
    }
}

/// Puts `node` in `parent` under `name`, first removing the entry `name`
/// holds when `replaced`.
fn replace_entry(parent: &Directory, name: String, replaced: bool, node: Node) -> Result<()> {
    if !parent.writable() {
        return Err(Error::new(ErrorKind::NotPermitted));
    }
    if replaced {
        parent.remove(&name)?;
    }
    parent.insert(name, node)
}
