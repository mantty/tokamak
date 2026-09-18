//! Read-only bundle storage for the virtual filesystem.

use std::fs;
use std::path::Path;

use super::nodes::{Directory, FileNode, Node};
use super::{Bundle, DirectoryEntry, Error, ErrorKind, NodeType, Result};

impl Bundle {
    pub(super) fn node(&self, relative: &Path) -> Result<Option<Node>> {
        let metadata = match fs::symlink_metadata(self.root.join(relative)) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(Error::from_io(relative.display(), &error)),
        };
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            return Err(Error::new(ErrorKind::Unsupported, relative.display()));
        }
        if file_type.is_dir() {
            return Ok(Some(Node::Directory(Directory::bundle(
                self.clone(),
                relative.to_owned(),
            ))));
        }
        if file_type.is_file() {
            return Ok(Some(Node::File(FileNode::Bundle {
                bundle: self.clone(),
                relative: relative.to_owned(),
            })));
        }
        Err(Error::new(ErrorKind::Unsupported, relative.display()))
    }

    pub(super) fn metadata(&self, relative: &Path) -> Result<fs::Metadata> {
        let metadata = fs::symlink_metadata(self.root.join(relative))
            .map_err(|error| Error::from_io(relative.display(), &error))?;
        if !metadata.file_type().is_file() {
            return Err(Error::new(ErrorKind::IsDirectory, relative.display()));
        }
        Ok(metadata)
    }

    pub(super) fn read(&self, relative: &Path) -> Result<Vec<u8>> {
        self.metadata(relative)?;
        fs::read(self.root.join(relative))
            .map_err(|error| Error::from_io(relative.display(), &error))
    }

    pub(super) fn entries(&self, relative: &Path) -> Result<Vec<DirectoryEntry>> {
        let root = self.root.join(relative);
        let metadata = match fs::symlink_metadata(&root) {
            Ok(metadata) => metadata,
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && relative.as_os_str().is_empty() =>
            {
                return Ok(Vec::new());
            }
            Err(error) => return Err(Error::from_io(relative.display(), &error)),
        };
        if !metadata.file_type().is_dir() {
            return Err(Error::new(ErrorKind::NotDirectory, relative.display()));
        }
        let mut entries = Vec::new();
        for entry in
            fs::read_dir(root).map_err(|error| Error::from_io(relative.display(), &error))?
        {
            let entry = entry.map_err(|error| Error::from_io(relative.display(), &error))?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| Error::new(ErrorKind::InvalidPath, relative.display()))?;
            let file_type = entry
                .file_type()
                .map_err(|error| Error::from_io(name.clone(), &error))?;
            let kind = if file_type.is_file() {
                NodeType::File
            } else if file_type.is_dir() {
                NodeType::Directory
            } else {
                return Err(Error::new(ErrorKind::Unsupported, name));
            };
            entries.push(DirectoryEntry {
                name,
                kind,
                device: false,
            });
        }
        Ok(entries)
    }
}
