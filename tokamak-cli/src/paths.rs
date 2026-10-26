//! Path helpers.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;

/// Remove whatever is at `path`.
pub(crate) fn reset_path(path: &Path) -> Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)?;
    } else if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

/// `path` with forward slashes.
pub(crate) fn slash_path(path: &Path) -> Result<String> {
    let path = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("path is not valid UTF-8: {}", path.display()))?;
    Ok(path.replace(std::path::MAIN_SEPARATOR, "/"))
}

/// `path` as Bash, PowerShell and Node accept it: without the verbatim prefix
/// of a canonical Windows drive path.
pub(crate) fn command_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let value = path.to_string_lossy();
        if let Some(value) = value.strip_prefix(r"\\?\")
            && value.as_bytes().get(1) == Some(&b':')
        {
            return value.into();
        }
    }
    path.into()
}

/// `program` as a Windows command line finds it: the first file in `PATH`
/// named `program` with one of `PATHEXT`'s extensions, such as the `vite.cmd`
/// an npm package installs. Elsewhere, or when no such file exists, `program`.
pub(crate) fn command_program(program: &OsStr) -> PathBuf {
    #[cfg(windows)]
    if let Some(found) = find_program(
        program,
        &std::env::var_os("PATH").unwrap_or_default(),
        &std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned()),
    ) {
        return found;
    }
    program.into()
}

/// The first file in the directories of `path` named `program` with one of the
/// `;`-separated `extensions`, when `program` is a name without one.
#[cfg(any(windows, test))]
fn find_program(program: &OsStr, path: &OsStr, extensions: &str) -> Option<PathBuf> {
    let name = Path::new(program);
    if name.components().count() != 1 || name.extension().is_some() {
        return None;
    }
    std::env::split_paths(path).find_map(|directory| {
        extensions
            .split(';')
            .filter(|extension| !extension.is_empty())
            .map(|extension| {
                let mut file = program.to_owned();
                file.push(extension);
                directory.join(file)
            })
            .find(|candidate| candidate.is_file())
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;

    use super::find_program;

    // Lowercase, as the files are: Windows matches extensions in any case, Linux exactly.
    const EXTENSIONS: &str = ".com;.exe;.bat;.cmd";

    #[test]
    fn finds_a_name_in_the_first_directory_with_the_first_extension_that_matches()
    -> anyhow::Result<()> {
        let (empty, bin) = (tempfile::tempdir()?, tempfile::tempdir()?);
        fs::write(bin.path().join("vite.cmd"), "")?;
        fs::write(bin.path().join("node.exe"), "")?;
        fs::write(bin.path().join("node.cmd"), "")?;
        let path = std::env::join_paths([empty.path(), bin.path()])?;

        for (program, expected) in [
            ("vite", Some(bin.path().join("vite.cmd"))),
            ("node", Some(bin.path().join("node.exe"))),
            ("astro", None),
            ("vite.cmd", None),
            ("bin/vite", None),
        ] {
            assert_eq!(
                find_program(OsStr::new(program), &path, EXTENSIONS),
                expected,
                "{program}"
            );
        }
        Ok(())
    }
}
