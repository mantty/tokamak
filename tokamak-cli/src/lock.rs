//! The app's `tokamak.lock`: each platform family's lock files, which pin the versions of
//! plugins' native dependencies that builds resolve. A build sees its family's files in its
//! input's `lock` directory and leaves there the files it resolved.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};

const LOCK: &str = "tokamak.lock";

/// The lines of each lock file, by family and then file name, so that changes diff by line.
type Locks = BTreeMap<String, BTreeMap<String, Vec<String>>>;

/// Writes `family`'s lock files into `input/lock`, unless `update`, when the build resolves the
/// family's dependencies afresh.
pub(crate) fn stage(project: &Path, family: &str, input: &Path, update: bool) -> Result<()> {
    if update {
        return Ok(());
    }
    let Some(files) = read(project)?.remove(family) else {
        return Ok(());
    };
    let directory = input.join("lock");
    fs::create_dir_all(&directory)?;
    for (name, lines) in files {
        fs::write(directory.join(name), lines.join("\n"))?;
    }
    Ok(())
}

/// Records the lock files a build left in `input/lock` as `family`'s.
pub(crate) fn keep(project: &Path, family: &str, input: &Path) -> Result<()> {
    let mut locks = read(project)?;
    let files = written(&input.join("lock"))?;
    if files.is_empty() {
        locks.remove(family);
    } else {
        locks.insert(family.to_owned(), files);
    }
    write(project, &locks)
}

fn read(project: &Path) -> Result<Locks> {
    let path = project.join(LOCK);
    if !path.is_file() {
        return Ok(Locks::new());
    }
    let locks: Locks = serde_json::from_str(&fs::read_to_string(&path)?)
        .with_context(|| format!("parse {}", path.display()))?;
    for (family, files) in &locks {
        if let Some(name) = files.keys().find(|name| !is_file_name(name)) {
            bail!(
                "{} has {family} lock file '{name}', which is not a file name",
                path.display()
            );
        }
    }
    Ok(locks)
}

/// The text files in `directory`, which a build writes.
fn written(directory: &Path) -> Result<BTreeMap<String, Vec<String>>> {
    let mut files = BTreeMap::new();
    if !directory.is_dir() {
        return Ok(files);
    }
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        let name = path.file_name().and_then(|name| name.to_str());
        let (Some(name), true) = (name, path.is_file()) else {
            bail!(
                "the build wrote {}, which is not a lock file",
                path.display()
            );
        };
        let text = fs::read_to_string(&path)
            .with_context(|| format!("read the lock file {}", path.display()))?;
        files.insert(
            name.to_owned(),
            text.split('\n').map(str::to_owned).collect(),
        );
    }
    Ok(files)
}

/// Writes `locks`, or removes the lock when it has none, leaving an unchanged lock untouched.
fn write(project: &Path, locks: &Locks) -> Result<()> {
    let path = project.join(LOCK);
    if locks.is_empty() {
        if path.is_file() {
            fs::remove_file(&path)?;
        }
        return Ok(());
    }
    let text = serde_json::to_string_pretty(locks)? + "\n";
    if fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
        fs::write(&path, text).with_context(|| format!("write {}", path.display()))?;
    }
    Ok(())
}

fn is_file_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\'])
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{keep, stage};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn keeps_each_family_s_lock_files_and_stages_them_for_its_builds() -> TestResult {
        let project = tempfile::tempdir()?;
        let input = tempfile::tempdir()?;
        let lock = input.path().join("lock");
        fs::create_dir(&lock)?;
        fs::write(lock.join("Package.resolved"), "{\n  \"pins\" : []\n}\n")?;
        keep(project.path(), "apple", input.path())?;
        fs::write(lock.join("Package.resolved"), "changed")?;
        fs::write(lock.join("app.lockfile"), "a=1")?;
        fs::remove_file(lock.join("Package.resolved"))?;
        keep(project.path(), "android", input.path())?;

        let text = fs::read_to_string(project.path().join("tokamak.lock"))?;
        let expected = serde_json::json!({
            "android": { "app.lockfile": ["a=1"] },
            "apple": { "Package.resolved": ["{", "  \"pins\" : []", "}", ""] },
        });
        assert_eq!(serde_json::from_str::<serde_json::Value>(&text)?, expected);

        let staged = tempfile::tempdir()?;
        stage(project.path(), "apple", staged.path(), false)?;
        assert_eq!(
            fs::read_to_string(staged.path().join("lock/Package.resolved"))?,
            "{\n  \"pins\" : []\n}\n"
        );
        assert!(!staged.path().join("lock/app.lockfile").exists());
        Ok(())
    }

    /// A project whose Apple family keeps the lock files in `input`.
    fn project_with_apple_lock() -> Result<tempfile::TempDir, Box<dyn std::error::Error>> {
        let project = tempfile::tempdir()?;
        let input = tempfile::tempdir()?;
        fs::create_dir(input.path().join("lock"))?;
        fs::write(input.path().join("lock/Package.resolved"), "pins")?;
        keep(project.path(), "apple", input.path())?;
        Ok(project)
    }

    #[test]
    fn stages_nothing_for_an_update_or_a_family_without_locks() -> TestResult {
        let project = project_with_apple_lock()?;

        let staged = tempfile::tempdir()?;
        stage(project.path(), "apple", staged.path(), true)?;
        stage(project.path(), "android", staged.path(), false)?;
        assert!(!staged.path().join("lock").exists());
        Ok(())
    }

    #[test]
    fn removes_the_lock_when_no_family_has_lock_files() -> TestResult {
        let project = project_with_apple_lock()?;
        assert!(project.path().join("tokamak.lock").is_file());

        let empty = tempfile::tempdir()?;
        keep(project.path(), "apple", empty.path())?;
        assert!(!project.path().join("tokamak.lock").exists());
        Ok(())
    }

    #[test]
    fn rejects_lock_file_names_that_are_paths() -> TestResult {
        let project = tempfile::tempdir()?;
        fs::write(
            project.path().join("tokamak.lock"),
            r#"{"apple": {"../escape": ["x"]}}"#,
        )?;
        let staged = tempfile::tempdir()?;
        let error = stage(project.path(), "apple", staged.path(), false)
            .err()
            .ok_or("staged a path")?;
        assert!(
            error
                .to_string()
                .contains("apple lock file '../escape', which is not a file name"),
            "{error}"
        );
        assert!(!staged.path().join("escape").exists());
        Ok(())
    }
}
