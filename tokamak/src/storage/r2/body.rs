//! Object and part bodies on disk: written with their digests, and read by
//! range.

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::digest::DynDigest;

use super::failure::Failure;
use super::range::Range;
use super::{hex, random_name};

/// Bytes a body read returns at most.
const CHUNK: u64 = 1024 * 1024;

/// A checksum algorithm R2 verifies.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Algorithm {
    Md5,
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}

impl Algorithm {
    /// The algorithm's name in R2's messages.
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Md5 => "MD5",
            Self::Sha1 => "SHA-1",
            Self::Sha256 => "SHA-256",
            Self::Sha384 => "SHA-384",
            Self::Sha512 => "SHA-512",
        }
    }

    /// The algorithm's field in an object's checksums.
    const fn field(self) -> &'static str {
        match self {
            Self::Md5 => "md5",
            Self::Sha1 => "sha1",
            Self::Sha256 => "sha256",
            Self::Sha384 => "sha384",
            Self::Sha512 => "sha512",
        }
    }

    fn digest(self) -> Box<dyn DynDigest + Send> {
        match self {
            Self::Md5 => Box::new(md5::Md5::default()),
            Self::Sha1 => Box::new(sha1::Sha1::default()),
            Self::Sha256 => Box::new(sha2::Sha256::default()),
            Self::Sha384 => Box::new(sha2::Sha384::default()),
            Self::Sha512 => Box::new(sha2::Sha512::default()),
        }
    }
}

/// A checksum a write provides, in lower-case hex.
#[derive(Debug, Deserialize)]
pub(crate) struct Checksum {
    pub(crate) algorithm: Algorithm,
    pub(crate) value: String,
}

/// A body being written to a new file in a directory.
pub(crate) struct BodyWriter {
    file: File,
    pending: Pending,
    name: String,
    length: u64,
    written: u64,
    md5: Box<dyn DynDigest + Send>,
    checksum: Option<(Checksum, Box<dyn DynDigest + Send>)>,
}

impl BodyWriter {
    /// A writer of a `length`-byte body into a new file in `directory`, which
    /// digests the body with MD5 and the algorithm of `checksum`.
    pub(super) fn create(
        directory: &Path,
        length: u64,
        checksum: Option<Checksum>,
    ) -> Result<Self, Failure> {
        let name = random_name()?;
        let (file, pending) = Pending::create(directory.join(&name))?;
        Ok(Self {
            file,
            pending,
            name,
            length,
            written: 0,
            md5: Algorithm::Md5.digest(),
            checksum: checksum.map(|checksum| {
                let digest = checksum.algorithm.digest();
                (checksum, digest)
            }),
        })
    }

    /// Append `bytes` to the body.
    pub(crate) fn write(&mut self, bytes: &[u8]) -> Result<(), Failure> {
        let written = self.written + bytes.len() as u64;
        if written > self.length {
            return Err(Failure::Storage(format!(
                "The body is longer than its length of {} bytes",
                self.length
            )));
        }
        self.file.write_all(bytes)?;
        self.md5.update(bytes);
        if let Some((_, digest)) = &mut self.checksum {
            digest.update(bytes);
        }
        self.written = written;
        Ok(())
    }

    /// The complete body, synced to disk.
    pub(crate) fn finish(self) -> Result<Body, Failure> {
        if self.written != self.length {
            return Err(Failure::Storage(format!(
                "The body ended after {} of its {} bytes",
                self.written, self.length
            )));
        }
        self.pending.sync(&self.file)?;
        Ok(Body {
            pending: self.pending,
            name: self.name,
            size: self.written,
            md5: self.md5.finalize().into_vec(),
            checksum: self
                .checksum
                .map(|(checksum, digest)| (checksum, hex(&digest.finalize()))),
        })
    }
}

/// A body synced to disk, whose file is removed unless the bucket records it.
pub(crate) struct Body {
    pending: Pending,
    /// The body's file name.
    pub(super) name: String,
    pub(super) size: u64,
    pub(super) md5: Vec<u8>,
    /// The provided checksum, and the body's digest in its algorithm.
    checksum: Option<(Checksum, String)>,
}

impl Body {
    /// The object checksums of the body as JSON: its MD5, and the provided
    /// checksum once it matches the body.
    pub(super) fn checksums(&self) -> Result<String, Failure> {
        let mut checksums = serde_json::Map::new();
        checksums.insert("md5".to_owned(), hex(&self.md5).into());
        if let Some((provided, actual)) = &self.checksum {
            if provided.value != *actual {
                return Err(Failure::BadDigest {
                    algorithm: provided.algorithm,
                    provided: provided.value.clone(),
                    actual: actual.clone(),
                });
            }
            checksums.insert(provided.algorithm.field().to_owned(), actual.clone().into());
        }
        Ok(serde_json::Value::Object(checksums).to_string())
    }

    /// Keep the body's file.
    pub(super) fn keep(self) {
        self.pending.keep();
    }
}

/// Part of a body, read in order.
#[derive(Debug)]
pub(crate) struct BodyReader {
    file: File,
    remaining: u64,
}

impl BodyReader {
    /// A reader of `range` of the body at `path`.
    pub(super) fn open(path: &Path, range: Range) -> io::Result<Self> {
        let mut file = File::open(path)?;
        file.seek(SeekFrom::Start(range.offset))?;
        Ok(Self {
            file,
            remaining: range.length,
        })
    }

    /// The next bytes of the range, or none after its end.
    pub(crate) fn read(&mut self) -> io::Result<Option<Vec<u8>>> {
        let size = self.remaining.min(CHUNK);
        if size == 0 {
            return Ok(None);
        }
        let mut chunk = Vec::with_capacity(usize::try_from(size).map_err(io::Error::other)?);
        (&mut self.file).take(size).read_to_end(&mut chunk)?;
        if chunk.len() as u64 != size {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.remaining -= size;
        Ok(Some(chunk))
    }
}

/// A new file, removed when dropped unless kept.
#[derive(Debug)]
pub(super) struct Pending(Option<PathBuf>);

impl Pending {
    /// Create the file at `path`.
    pub(super) fn create(path: PathBuf) -> io::Result<(File, Self)> {
        let file = File::create_new(&path)?;
        Ok((file, Self(Some(path))))
    }

    /// Sync `file`, opened on this file, to disk, with the directory entry
    /// that names it where directories can be synced.
    pub(super) fn sync(&self, file: &File) -> io::Result<()> {
        file.sync_all()?;
        if cfg!(unix)
            && let Some(directory) = self.0.as_deref().and_then(Path::parent)
        {
            File::open(directory)?.sync_all()?;
        }
        Ok(())
    }

    pub(super) fn keep(mut self) {
        self.0 = None;
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = fs::remove_file(path);
        }
    }
}
