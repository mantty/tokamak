//! File copying shared by `tok` and the maintainer tools.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use walkdir::WalkDir;

/// Copy the file `from` to `to`, creating `to`'s parent directories.
///
/// # Errors
///
/// Returns an error when a directory cannot be created or the copy fails.
pub fn copy_file(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<()> {
    let from = from.as_ref();
    let to = to.as_ref();
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(from, to).with_context(|| format!("copy {} to {}", from.display(), to.display()))?;
    Ok(())
}

/// Copy the contents of the directory `from` into `to`, copying the targets
/// of links as regular files.
///
/// # Errors
///
/// Returns an error when `from` holds anything but files and directories, or
/// a copy fails.
pub fn copy_dir_contents(from: &Path, to: &Path) -> Result<()> {
    copy_dir_contents_except(from, to, |_| false)
}

/// Copy the contents of the directory `from` into `to`, as `copy_dir_contents`
/// does, leaving out each entry, with its contents, whose path in `from`
/// `skip` matches.
///
/// # Errors
///
/// Returns an error when `from` holds anything but files and directories, or
/// a copy fails.
pub fn copy_dir_contents_except(
    from: &Path,
    to: &Path,
    skip: impl Fn(&Path) -> bool,
) -> Result<()> {
    let entries = WalkDir::new(from)
        .follow_links(true)
        .min_depth(1)
        .into_iter()
        .filter_entry(|entry| !entry.path().strip_prefix(from).is_ok_and(&skip));
    for entry in entries {
        let entry = entry.map_err(std::io::Error::other)?;
        let destination = to.join(entry.path().strip_prefix(from)?);
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            fs::create_dir_all(destination)?;
        } else if metadata.is_file() {
            copy_file(entry.path(), destination)?;
        } else {
            bail!("unsupported file in {}", entry.path().display());
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    #[test]
    fn copies_link_targets_as_regular_files() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        let temporary = tempfile::tempdir()?;
        let source = temporary.path().join("source");
        let destination = temporary.path().join("destination");
        std::fs::create_dir_all(&source)?;
        std::fs::write(source.join("target"), "content")?;
        symlink("target", source.join("link"))?;

        super::copy_dir_contents(&source, &destination)?;

        assert_eq!(
            std::fs::read_to_string(destination.join("link"))?,
            "content"
        );
        assert!(
            !std::fs::symlink_metadata(destination.join("link"))?
                .file_type()
                .is_symlink()
        );
        Ok(())
    }

    #[test]
    fn leaves_out_skipped_entries_with_their_contents() -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempfile::tempdir()?;
        let source = temporary.path().join("source");
        let destination = temporary.path().join("destination");
        std::fs::create_dir_all(source.join("kept/build"))?;
        std::fs::create_dir_all(source.join("build/outputs"))?;
        std::fs::write(source.join("kept/build/file"), "kept")?;
        std::fs::write(source.join("build/outputs/file"), "left out")?;

        super::copy_dir_contents_except(&source, &destination, |path| path.starts_with("build"))?;

        assert!(destination.join("kept/build/file").is_file());
        assert!(!destination.join("build").exists());
        Ok(())
    }

    #[test]
    fn copies_nothing_from_a_file_and_fails_without_a_source()
    -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempfile::tempdir()?;
        let file = temporary.path().join("file");
        let destination = temporary.path().join("destination");
        std::fs::write(&file, "content")?;

        assert!(super::copy_dir_contents(&temporary.path().join("missing"), &destination).is_err());
        super::copy_dir_contents(&file, &destination)?;
        assert!(!destination.exists());
        Ok(())
    }
}
