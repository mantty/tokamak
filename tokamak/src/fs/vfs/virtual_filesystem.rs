#![allow(clippy::missing_errors_doc)]

//! A small in-memory filesystem for the tokamak Worker contract.

use std::collections::BTreeMap;
use std::fmt::{self, Display, Formatter};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use indexmap::IndexMap;

use super::nodes::{Directory, FileNode, Node, OpenFile, shared_devices};
use super::path::{
    absolute_components, join_child, normalized_path, path_from_components, random_suffix,
    target_components,
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
    /// The operation failed without a more specific cause.
    Failed,
    /// A failure workerd reports as a plain `Error` without a code, such as
    /// an internal one.
    Internal,
    /// `cp` met an existing destination it may not replace.
    CopyExists,
    /// `cp` was asked to copy a directory into itself.
    CopyIntoItself,
    /// `cp` was asked to copy a non-directory onto a directory.
    CopyNonDirectoryToDirectory,
    /// `cp` was asked to copy a directory onto a non-directory.
    CopyDirectoryToNonDirectory,
    /// `cp` was asked to copy a directory without `recursive`.
    CopyDirectory,
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
            Self::Host | Self::Entropy | Self::Failed => "EIO",
            Self::Internal => "",
            Self::CopyExists => "ERR_FS_CP_EEXIST",
            Self::CopyIntoItself => "ERR_FS_CP_EINVAL",
            Self::CopyNonDirectoryToDirectory => "ERR_FS_CP_NON_DIR_TO_DIR",
            Self::CopyDirectoryToNonDirectory => "ERR_FS_CP_DIR_TO_NON_DIR",
            Self::CopyDirectory => "ERR_FS_EISDIR",
        }
    }

    /// Whether workerd reports the kind as a failed system call, with
    /// `syscall` and, when one is named, `path`. Other kinds are plain
    /// errors carrying only their code.
    #[must_use]
    pub const fn is_system_error(self) -> bool {
        !matches!(
            self,
            Self::Internal
                | Self::CopyExists
                | Self::CopyIntoItself
                | Self::CopyNonDirectoryToDirectory
                | Self::CopyDirectoryToNonDirectory
                | Self::CopyDirectory
        )
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
            Self::Host | Self::Entropy | Self::Failed => ("input/output error", true),
            Self::Internal => ("internal error", false),
            Self::CopyExists => ("File already exists", false),
            Self::CopyIntoItself => ("Cannot copy a directory into itself", false),
            Self::CopyNonDirectoryToDirectory => ("Cannot copy non-directory to directory", false),
            Self::CopyDirectoryToNonDirectory => ("Cannot copy directory to non-directory", false),
            Self::CopyDirectory => ("Expected a file but found a directory", false),
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
    /// Entries are named below `path` normalized.
    pub fn walk(&self, path: &str) -> Result<Vec<(String, DirectoryEntry)>> {
        let directory = self.listed_directory(path)?;
        let base = normalized_path(path)?;
        let mut entries = Vec::new();
        self.walk_directory(&base, &directory, &mut vec![base.clone()], &mut entries)?;
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

    /// Create a directory, leaving an existing directory alone. A recursive
    /// call also creates missing parents and, as workerd's does, returns the
    /// first parent it created.
    pub fn mkdir(&mut self, path: &str, recursive: bool) -> Result<Option<String>> {
        let components = absolute_components(path)?;
        if self.is_existing_directory(path)? {
            return Ok(None);
        }
        if recursive {
            return self.mkdir_recursive(&components);
        }
        let (base, name) = split_name(path)?;
        let parent = self.directory(base).map_err(|error| match error.kind() {
            ErrorKind::NotDirectory => error,
            _ => error.at(&name),
        })?;
        parent.insert(name, Node::Directory(Directory::memory()))?;
        Ok(None)
    }

    /// Write `data` as the content of the file `path` names, creating the file
    /// when it is missing, as workerd's `writeAll` does: `append` writes after
    /// the existing content, and `exclusive` fails when the file exists.
    /// Returns the number of bytes written.
    pub fn write_file(
        &mut self,
        path: &str,
        data: &[u8],
        append: bool,
        exclusive: bool,
    ) -> Result<usize> {
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
                let written = node.write(0, data, false)?;
                parent.insert(name, node)?;
                return Ok(written);
            }
            Err(error) => return Err(error),
        };
        let file = node.file()?;
        if !file.writable() {
            return Err(Error::new(ErrorKind::NotPermitted));
        }
        if !append {
            file.resize(0)?;
        }
        file.write(0, data, append)
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
        if replaced.is_some() && to == from {
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

    /// Copy what `from` names to `to`, as workerd's `cp` does. With
    /// `options.dereference`, symbolic links `from` and `to` name are
    /// followed, but not those inside a copied directory. A directory is
    /// copied into an existing directory entry by entry, and missing parents
    /// of `to` are created.
    pub fn copy(&mut self, from: &str, to: &str, options: CopyOptions) -> Result<()> {
        if from == to {
            return Err(Error::with_message(
                ErrorKind::Internal,
                "Source and destination paths must not be the same",
            ));
        }
        let prefix = if from.ends_with('/') {
            from.to_owned()
        } else {
            format!("{from}/")
        };
        if to.len() > prefix.len() && to.starts_with(&prefix) {
            return Err(Error::with_message(
                ErrorKind::CopyIntoItself,
                format!("Cannot copy '{from}' to a subdirectory of itself, '{to}'"),
            ));
        }
        let Some(source) = self.copy_lookup(from, options.dereference)? else {
            return Err(Error::new(ErrorKind::NotFound).at(from));
        };
        let destination = self.copy_lookup(to, options.dereference)?;
        match (source, destination) {
            (Node::Directory(_), _) if !options.recursive => {
                Err(Error::new(ErrorKind::CopyDirectory))
            }
            (Node::Directory(_), Some(Node::File(_))) => {
                Err(Error::new(ErrorKind::CopyDirectoryToNonDirectory))
            }
            (Node::Directory(_), Some(Node::Symlink(_)))
            | (Node::Symlink(_), Some(Node::Directory(_))) => {
                Err(Error::new(ErrorKind::NotDirectory))
            }
            (Node::Directory(source), Some(Node::Directory(destination))) => {
                self.copy_entries(&source, &destination, options, 0)
            }
            (Node::Directory(source), None) => {
                match self.open_creating(&absolute_components(to)?)? {
                    Some(Node::Directory(destination)) => {
                        self.copy_entries(&source, &destination, options, 0)
                    }
                    Some(_) => Err(Error::new(ErrorKind::NotDirectory)),
                    None => Err(Error::new(ErrorKind::InvalidInput)),
                }
            }
            (Node::File(_), Some(Node::Directory(_))) => {
                Err(Error::new(ErrorKind::CopyNonDirectoryToDirectory))
            }
            (_, Some(_)) if !options.force => {
                if options.error_on_exist {
                    return Err(Error::new(ErrorKind::CopyExists));
                }
                Ok(())
            }
            // workerd copies the destination link here, which leaves it as it
            // was.
            (Node::Symlink(_), Some(existing @ Node::Symlink(_))) => self.copy_entry(&existing, to),
            (node, _) => self.copy_entry(&node, to),
        }
    }

    /// What `path` names for `cp`, following symbolic links when `follow`:
    /// `None` when nothing does, or, when not following, a symbolic link is
    /// on the way.
    fn copy_lookup(&self, path: &str, follow: bool) -> Result<Option<Node>> {
        if follow {
            self.lookup(path)
        } else {
            self.node_without_links(path)
        }
    }

    /// Puts a copy of the file or symbolic link `node` at `path`, replacing
    /// what is there and creating missing parent directories.
    fn copy_entry(&self, node: &Node, path: &str) -> Result<()> {
        let (base, name) = split_name(path)?;
        let parent = match self.open_creating(&base)? {
            Some(Node::Directory(parent)) => parent,
            Some(_) => return Err(Error::new(ErrorKind::NotDirectory)),
            None => return Err(Error::new(ErrorKind::InvalidInput)),
        };
        parent.remove(&name)?;
        parent.insert(name, copied(node)?)
    }

    /// Copies the entries of `source` into `destination`, as workerd copies a
    /// directory: an entry replaces an existing file or symbolic link only
    /// with `options.force`, and a directory is copied into an existing
    /// directory. Existing symbolic links are followed with
    /// `options.dereference`. `depth` counts the directories above; a
    /// directory holds itself once a symbolic link to it is renamed into
    /// it, and copying one ends at the path segment limit.
    fn copy_entries(
        &self,
        source: &Directory,
        destination: &Directory,
        options: CopyOptions,
        depth: usize,
    ) -> Result<()> {
        if !destination.writable() {
            return Err(Error::with_message(
                ErrorKind::NotPermitted,
                "Destination directory is not writable",
            ));
        }
        if source.same(destination) {
            return Err(Error::with_message(
                ErrorKind::InvalidInput,
                "Source and destination directories are the same",
            ));
        }
        if depth > MAX_PATH_SEGMENTS {
            return Err(Error::new(ErrorKind::InvalidPath));
        }
        let mut entries = Vec::new();
        for entry in source.entries()? {
            if let Some(node) = source.lookup(&entry.name)? {
                entries.push((entry.name, node));
            }
        }
        for (name, node) in entries {
            let existing = match destination.lookup(&name)? {
                Some(Node::Symlink(target)) if options.dereference => self.lookup(&target)?,
                existing => existing,
            };
            let conflict =
                |kind, message: &str| Err(Error::with_message(kind, format!("{message}: {name}")));
            match (node, existing) {
                (node @ (Node::File(_) | Node::Symlink(_)), Some(existing))
                    if node.same(&existing) => {}
                (Node::File(_), Some(Node::Directory(_))) => {
                    return conflict(ErrorKind::IsDirectory, "Cannot copy file to directory");
                }
                (Node::Symlink(_), Some(Node::Directory(_))) => {
                    return conflict(ErrorKind::IsDirectory, "Cannot copy link to directory");
                }
                (Node::Directory(_), Some(Node::File(_))) => {
                    return conflict(ErrorKind::NotDirectory, "Cannot copy directory to file");
                }
                (Node::Directory(_), Some(Node::Symlink(_))) => {
                    return conflict(
                        ErrorKind::IsDirectory,
                        "Cannot copy directory to symbolic link",
                    );
                }
                (Node::Directory(directory), Some(Node::Directory(existing))) => {
                    self.copy_entries(&directory, &existing, options, depth + 1)?;
                }
                (Node::Directory(directory), None) => {
                    let child = Directory::memory();
                    destination.insert(name, Node::Directory(child.clone()))?;
                    self.copy_entries(&directory, &child, options, depth + 1)?;
                }
                (node, Some(_)) if options.force => {
                    destination.remove(&name)?;
                    destination.insert(name, copied(&node)?)?;
                }
                (_, Some(_)) if options.error_on_exist => {
                    return conflict(ErrorKind::AlreadyExists, "Destination already exists");
                }
                (_, Some(_)) => {}
                (node, None) => destination.insert(name, copied(&node)?)?,
            }
        }
        Ok(())
    }

    /// What `components` name, following symbolic links and creating missing
    /// directories, as workerd's `tryOpen` with `createAs` a directory does:
    /// `None` when a file or a symbolic link that leads nowhere is on the
    /// way, or a final symbolic link leads nowhere.
    fn open_creating(&self, components: &[String]) -> Result<Option<Node>> {
        let mut directory = self.root.clone();
        for (index, name) in components.iter().enumerate() {
            let node = match directory.lookup(name)? {
                None => return create_directories(&directory, &components[index..]).map(Some),
                Some(Node::Symlink(_)) => {
                    match self.lookup_components(components[..=index].to_vec())? {
                        Some(node) => node,
                        None => return Ok(None),
                    }
                }
                Some(node) => node,
            };
            if index + 1 == components.len() {
                return Ok(Some(node));
            }
            let Node::Directory(child) = node else {
                return Ok(None);
            };
            directory = child;
        }
        Ok(Some(Node::Directory(directory)))
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
        let (base, name) = split_name(path)?;
        if name.is_empty() {
            return Err(Error::with_message(
                ErrorKind::InvalidInput,
                "Invalid filename",
            ));
        }
        let parent = self.directory(base).map_err(|error| match error.kind() {
            ErrorKind::NotFound => {
                Error::with_message(ErrorKind::NotFound, "Directory does not exist").at(target)
            }
            ErrorKind::NotDirectory => {
                Error::with_message(ErrorKind::InvalidInput, "Not a directory")
            }
            _ => error,
        })?;
        Ok((parent, name))
    }

    /// Whether `path` names a directory, without following symbolic links,
    /// as workerd's `mkdir` checks first: anything else there already exists.
    fn is_existing_directory(&self, path: &str) -> Result<bool> {
        match self.resolve(path, false) {
            Ok(Node::Directory(_)) => Ok(true),
            Ok(_) => Err(Error::new(ErrorKind::AlreadyExists)),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Create a unique temporary directory below an existing parent, named
    /// `prefix` and a suffix, as workerd's `mkdir` does for `mkdtemp`. `None`
    /// when `prefix` names a directory, which that `mkdir` leaves alone.
    pub fn make_temp_dir(&mut self, prefix: &str) -> Result<Option<String>> {
        if self.is_existing_directory(prefix)? {
            return Ok(None);
        }
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
                return Ok(Some(path_from_components(&components)));
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
        parent.insert(name, Node::Symlink(normalized_path(target)?.into()))
    }

    /// Read a symbolic link target.
    pub fn read_link(&self, path: &str) -> Result<String> {
        match self.symlink_target(path)? {
            Some(target) => Ok(target),
            None => Err(Error::new(ErrorKind::InvalidInput)),
        }
    }

    /// The target of the symbolic link `path` names, when it is one.
    fn symlink_target(&self, path: &str) -> Result<Option<String>> {
        let node = self
            .resolve(path, false)
            .map_err(|error| missing_at(error, path))?;
        Ok(match node {
            Node::Symlink(target) => Some(target.to_string()),
            _ => None,
        })
    }

    /// The target of the symbolic link `path` names, or else `path`
    /// normalized, as workerd's `realpath` reports it: only a final symbolic
    /// link is read, and only once.
    pub fn realpath(&self, path: &str) -> Result<String> {
        match self.symlink_target(path)? {
            Some(target) => Ok(target),
            None => normalized_path(path),
        }
    }

    /// Open a file and return its descriptor. Failures name `path`.
    pub fn open(&mut self, path: &str, options: OpenOptions) -> Result<u32> {
        self.open_file(path, options)
            .map_err(|error| error.at(path))
    }

    /// Opens what `path` names as workerd's `openFd` does: its parent must be
    /// a directory, a missing file is created with `options.create`, and a
    /// directory opens only for reading.
    fn open_file(&mut self, path: &str, options: OpenOptions) -> Result<u32> {
        if !options.read && !options.write {
            return Err(Error::new(ErrorKind::InvalidInput));
        }
        let mut components = absolute_components(path)?;
        let name = components.pop();
        let parent = self.directory(components)?;
        let node = match self.lookup(path)? {
            Some(_) if options.create && options.exclusive => {
                return Err(Error::new(ErrorKind::AlreadyExists));
            }
            Some(node) => node,
            None if !options.create => return Err(Error::new(ErrorKind::NotFound)),
            None => {
                let name = name.ok_or_else(|| Error::new(ErrorKind::InvalidInput))?;
                if parent.lookup(&name)?.is_some() {
                    // A symbolic link that leads nowhere is not replaced.
                    return Err(Error::new(ErrorKind::Failed));
                }
                let node = Node::File(FileNode::memory());
                parent.insert(name, node.clone())?;
                node
            }
        };
        match &node {
            Node::Directory(_) if options.write => return Err(Error::new(ErrorKind::IsDirectory)),
            Node::File(file) if options.write => {
                if !file.writable() {
                    return Err(Error::new(ErrorKind::NotPermitted));
                }
                if options.truncate && file.size()? > 0 {
                    file.resize(0)?;
                }
            }
            _ => {}
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
            return Err(Error::new(ErrorKind::InvalidDescriptor));
        }
        let offset = position.unwrap_or(open.position);
        let bytes = open.node.read(offset, length)?;
        if position.is_none() {
            open.position = offset.saturating_add(bytes.len() as u64);
        }
        Ok(bytes)
    }

    /// Write to a descriptor. `position` does not move the descriptor cursor.
    /// As in workerd, a descriptor opened without write access writes too.
    pub fn write(&mut self, descriptor: u32, data: &[u8], position: Option<u64>) -> Result<usize> {
        let open = self.descriptor_mut(descriptor)?;
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

    /// Whether writing a whole file to `descriptor` appends to it, failing as
    /// workerd's `writeAll` does for a descriptor it cannot write a file to.
    pub fn whole_write_appends(&self, descriptor: u32) -> Result<bool> {
        let open = self.descriptor(descriptor)?;
        if !open.write {
            return Err(Error::new(ErrorKind::InvalidDescriptor));
        }
        let file = open
            .node
            .file()
            .map_err(|_| Error::new(ErrorKind::InvalidDescriptor))?;
        if !file.writable() {
            return Err(Error::new(ErrorKind::NotPermitted));
        }
        Ok(open.append)
    }

    /// The size of the file `descriptor` names, failing as workerd's
    /// `readAll` does for a descriptor it cannot read a whole file from.
    pub fn whole_read_size(&self, descriptor: u32) -> Result<u64> {
        let open = self.descriptor(descriptor)?;
        if !open.read {
            return Err(Error::new(ErrorKind::InvalidDescriptor));
        }
        open.node
            .file()
            .map_err(|_| Error::new(ErrorKind::InvalidDescriptor))?
            .size()
    }

    /// Moves `descriptor` to the end of its file.
    pub fn seek_end(&mut self, descriptor: u32) -> Result<()> {
        let open = self.descriptor_mut(descriptor)?;
        open.position = open.node.size()?;
        Ok(())
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

    /// Truncate a descriptor. As in workerd, a descriptor opened without
    /// write access truncates too.
    pub fn ftruncate(&mut self, descriptor: u32, size: u64) -> Result<()> {
        self.descriptor(descriptor)?.node.resize(size)
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

    fn mkdir_recursive(&mut self, components: &[String]) -> Result<Option<String>> {
        let Some((name, parents)) = components.split_last() else {
            return Ok(None);
        };
        let mut current = self.root.clone();
        let mut created = None;
        for (index, component) in parents.iter().enumerate() {
            let node = match current.lookup(component)? {
                Some(Node::Symlink(_)) => self.lookup_components(components[..=index].to_vec())?,
                node => node,
            };
            current = match node {
                Some(Node::Directory(directory)) => directory,
                Some(_) => return Err(Error::new(ErrorKind::NotDirectory)),
                None => {
                    let child = Directory::memory();
                    current.insert(component.clone(), Node::Directory(child.clone()))?;
                    created.get_or_insert_with(|| path_from_components(&components[..=index]));
                    child
                }
            };
        }
        current.insert(name.clone(), Node::Directory(Directory::memory()))?;
        Ok(created)
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
            let real_path = path_from_components(&self.resolve_with_path(&child)?.1);
            if ancestors.contains(&real_path) {
                return Err(Error::new(ErrorKind::SymlinkLoop));
            }
            ancestors.push(real_path);
            self.walk_directory(&child, &child_directory, ancestors, entries)?;
            ancestors.pop();
        }
        Ok(())
    }

    /// The directory `components` name, following symbolic links.
    fn directory(&self, components: Vec<String>) -> Result<Directory> {
        let (node, _) = self.resolve_components(components)?;
        node.as_directory()
            .ok_or_else(|| Error::new(ErrorKind::NotDirectory))
    }

    /// The directory holding the last component of `path`, and that
    /// component's name, as `split_name` splits them. A missing directory is
    /// reported under the name.
    fn containing_directory(&self, path: &str) -> Result<(Directory, String)> {
        let (base, name) = split_name(path)?;
        match self.lookup_components(base)? {
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
        match self.resolve_components(components) {
            Ok((node, _)) => Ok(Some(node)),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// The node `path` names, following symbolic links when `follow`. As
    /// with workerd's `tryOpen`, a lookup that does not follow symbolic links
    /// follows none on the way either, and a file on the way leaves nothing
    /// found.
    fn resolve(&self, path: &str, follow: bool) -> Result<Node> {
        if follow {
            return self.resolve_with_path(path).map(|(node, _)| node);
        }
        self.node_without_links(path)?
            .ok_or_else(|| Error::new(ErrorKind::NotFound))
    }

    fn resolve_with_path(&self, path: &str) -> Result<(Node, Vec<String>)> {
        self.resolve_components(absolute_components(path)?)
    }

    /// The node `pending` names, following symbolic links, and the
    /// components of its real path.
    fn resolve_components(&self, mut pending: Vec<String>) -> Result<(Node, Vec<String>)> {
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
            let child = current
                .as_directory()
                .map(|directory| directory.lookup(&name))
                .transpose()?
                .flatten()
                .ok_or_else(|| Error::new(ErrorKind::NotFound))?;
            match child {
                Node::Symlink(_) if symlinks == 40 => {
                    return Err(Error::new(ErrorKind::SymlinkLoop));
                }
                Node::Symlink(target) => {
                    symlinks += 1;
                    let mut next = target_components(&resolved, &target)?;
                    next.extend_from_slice(&pending[index + 1..]);
                    pending = next;
                    current = root.clone();
                    resolved.clear();
                    index = 0;
                }
                child => {
                    current = child;
                    resolved.push(name);
                    index += 1;
                }
            }
        }
    }
}

/// The components of the directory holding the last component of `path`,
/// and that component's name, as workerd splits a path it creates or removes
/// an entry at: a path ending in `/` names an empty name in the directory the
/// path itself names.
fn split_name(path: &str) -> Result<(Vec<String>, String)> {
    let mut components = absolute_components(path)?;
    if path.ends_with('/') {
        return Ok((components, String::new()));
    }
    let name = components
        .pop()
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput))?;
    Ok((components, name))
}

/// A copy of the file or symbolic link `node`, as workerd's `cp` makes one:
/// a file's content is copied, and a symbolic link is shared.
fn copied(node: &Node) -> Result<Node> {
    match node {
        Node::File(file) => Ok(Node::File(file.copied()?)),
        node => Ok(node.clone()),
    }
}

/// Creates the directories `names` name, each in the one before, in
/// `parent`, returning the last.
fn create_directories(parent: &Directory, names: &[String]) -> Result<Node> {
    if !parent.writable() {
        return Err(Error::new(ErrorKind::NotPermitted));
    }
    let mut current = parent.clone();
    for name in names {
        let child = Directory::memory();
        current.insert(name.clone(), Node::Directory(child.clone()))?;
        current = child;
    }
    Ok(Node::Directory(current))
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
