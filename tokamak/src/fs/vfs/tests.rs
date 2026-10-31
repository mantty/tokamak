use super::{
    Bundle, CopyOptions, ErrorKind, MAX_PATH_LENGTH, MAX_PATH_SEGMENTS, NodeType, OpenOptions,
    VirtualFileSystem, join_child,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn vfs(bundle: &std::path::Path) -> VirtualFileSystem {
    VirtualFileSystem::new(Bundle::new(bundle))
}

#[test]
fn keeps_tmp_mutable_and_bundle_readonly() -> TestResult {
    let directory = tempfile::tempdir()?;
    std::fs::create_dir_all(directory.path().join("config"))?;
    std::fs::write(directory.path().join("config/app.json"), b"hello")?;
    let mut vfs = vfs(directory.path());

    assert_eq!(vfs.read_file("/bundle/config/app.json")?, b"hello");
    assert!(!vfs.stat("/bundle/config/app.json")?.writable);
    vfs.write_file("/tmp/value.txt", b"one", false, false)?;
    vfs.write_file("/tmp/value.txt", b" two", true, false)?;
    assert_eq!(vfs.read_file("/tmp/value.txt")?, b"one two");
    assert_eq!(vfs.read_dir("/bundle/config")?[0].name, "app.json");
    let second = VirtualFileSystem::new(Bundle::new(directory.path()));
    assert_eq!(
        second
            .stat("/tmp/value.txt")
            .err()
            .map(|error| error.kind()),
        Some(ErrorKind::NotFound)
    );
    let Err(error) = vfs.write_file("/bundle/nope", b"x", false, false) else {
        return Err("readonly bundle accepted a write".into());
    };
    assert_eq!(error.kind(), ErrorKind::NotPermitted);
    Ok(())
}

#[test]
fn spells_bundle_names_as_stored_names() -> TestResult {
    let directory = tempfile::tempdir()?;
    std::fs::write(directory.path().join("a b\u{e9}.txt"), b"named")?;
    let vfs = vfs(directory.path());

    assert_eq!(vfs.read_dir("/bundle")?[0].name, "a%20b%C3%A9.txt");
    assert_eq!(vfs.read_file("/bundle/a%20b%C3%A9.txt")?, b"named");
    Ok(())
}

#[test]
fn implements_devices() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut vfs = vfs(directory.path());
    let null = vfs.open(
        "/dev/null",
        OpenOptions {
            read: true,
            write: true,
            ..OpenOptions::default()
        },
    )?;
    assert!(vfs.read(null, 8, None)?.is_empty());
    assert_eq!(vfs.write(null, b"ignored", None)?, 7);
    vfs.close(null);

    let zero = vfs.open("/dev/zero", OpenOptions::default())?;
    assert_eq!(vfs.read(zero, 3, None)?, vec![0, 0, 0]);
    vfs.close(zero);

    let full = vfs.open(
        "/dev/full",
        OpenOptions {
            read: true,
            write: true,
            ..OpenOptions::default()
        },
    )?;
    assert_eq!(vfs.read(full, 2, None)?, vec![0, 0]);
    let Err(error) = vfs.write(full, b"x", None) else {
        return Err("/dev/full accepted a write".into());
    };
    assert_eq!(error.kind(), ErrorKind::NoSpace);
    Ok(())
}

#[test]
fn handles_symlinks_and_descriptors() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut vfs = vfs(directory.path());
    vfs.write_file("/tmp/source", b"hello", false, false)?;
    assert_eq!(
        vfs.mkdir("/tmp/source", true)
            .err()
            .map(|error| error.kind()),
        Some(ErrorKind::AlreadyExists)
    );
    vfs.symlink("/tmp/source", "/tmp/link")?;
    assert_eq!(vfs.read_file("/tmp/link")?, b"hello");
    assert_eq!(vfs.lstat("/tmp/link")?.kind, NodeType::Symlink);
    assert_eq!(vfs.realpath("/tmp/link")?, "/tmp/source");

    let fd = vfs.open(
        "/tmp/source",
        OpenOptions {
            read: true,
            write: true,
            ..OpenOptions::default()
        },
    )?;
    assert_eq!(vfs.read(fd, 5, None)?, b"hello");
    vfs.write(fd, b"!", Some(5))?;
    assert_eq!(vfs.fstat(fd)?.size, 6);
    vfs.close(fd);
    assert_eq!(vfs.read_file("/tmp/source")?, b"hello!");
    Ok(())
}

#[test]
fn checks_descriptor_access_as_workerd_does() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut vfs = vfs(directory.path());
    vfs.write_file("/tmp/value", b"hello", false, false)?;

    let read_only = vfs.open("/tmp/value", OpenOptions::default())?;
    vfs.ftruncate(read_only, 1)?;
    assert_eq!(vfs.write(read_only, b"i", None)?, 1);
    assert_eq!(vfs.read_file("/tmp/value")?, b"i");
    vfs.close(read_only);

    let write_only = vfs.open(
        "/tmp/value",
        OpenOptions {
            read: false,
            write: true,
            ..OpenOptions::default()
        },
    )?;
    assert_eq!(
        vfs.read(write_only, 1, None)
            .err()
            .map(|error| error.kind()),
        Some(ErrorKind::InvalidDescriptor)
    );
    assert_eq!(
        vfs.whole_read_size(write_only)
            .err()
            .map(|error| error.kind()),
        Some(ErrorKind::InvalidDescriptor)
    );
    vfs.close(write_only);

    let directory = vfs.open("/tmp", OpenOptions::default())?;
    assert_eq!(vfs.fstat(directory)?.kind, NodeType::Directory);
    assert_eq!(
        vfs.read(directory, 1, None).err().map(|error| error.kind()),
        Some(ErrorKind::IsDirectory)
    );
    assert_eq!(
        vfs.whole_read_size(directory)
            .err()
            .map(|error| error.kind()),
        Some(ErrorKind::InvalidDescriptor)
    );
    vfs.close(directory);
    let writing_directory = OpenOptions {
        write: true,
        ..OpenOptions::default()
    };
    assert_eq!(
        vfs.open("/tmp", writing_directory)
            .err()
            .map(|error| error.kind()),
        Some(ErrorKind::IsDirectory)
    );
    Ok(())
}

#[test]
fn enforces_path_limits() -> TestResult {
    let directory = tempfile::tempdir()?;
    let vfs = vfs(directory.path());

    let too_long = format!("/{}", "x".repeat(MAX_PATH_LENGTH));
    assert_eq!(
        vfs.stat(&too_long).err().map(|error| error.kind()),
        Some(ErrorKind::InvalidPath)
    );

    let too_deep = format!(
        "/{}",
        (0..=MAX_PATH_SEGMENTS)
            .map(|_| "x")
            .collect::<Vec<_>>()
            .join("/")
    );
    assert_eq!(
        vfs.stat(&too_deep).err().map(|error| error.kind()),
        Some(ErrorKind::InvalidPath)
    );
    Ok(())
}

#[test]
fn supports_access_copy_links_temp_and_vectors() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut vfs = vfs(directory.path());
    vfs.write_file("/tmp/source.txt", b"hello", false, false)?;
    vfs.mkdir("/tmp/tree", false)?;
    vfs.write_file("/tmp/tree/nested.txt", b"nested", false, false)?;

    vfs.access("/tmp/source.txt", 0)?;
    assert_eq!(
        vfs.access("/bundle/missing", 0)
            .err()
            .map(|error| error.kind()),
        Some(ErrorKind::NotFound)
    );
    assert_eq!(
        vfs.access("/tmp/source.txt", 1)
            .err()
            .map(|error| error.kind()),
        Some(ErrorKind::NotFound)
    );

    vfs.link("/tmp/source.txt", "/tmp/hard-link.txt")?;
    vfs.copy(
        "/tmp/tree",
        "/tmp/tree-copy",
        CopyOptions {
            recursive: true,
            force: true,
            ..CopyOptions::default()
        },
    )?;
    assert_eq!(vfs.read_file("/tmp/hard-link.txt")?, b"hello");
    assert_eq!(vfs.read_file("/tmp/tree-copy/nested.txt")?, b"nested");

    let temp = vfs.make_temp_dir("/tmp/prefix-")?.unwrap_or_default();
    assert!(temp.starts_with("/tmp/prefix-"), "{temp}");
    assert_eq!(vfs.stat(&temp)?.kind, NodeType::Directory);
    assert_eq!(vfs.make_temp_dir("/tmp")?, None);

    let descriptor = vfs.open(
        "/tmp/vector.bin",
        OpenOptions {
            read: true,
            write: true,
            create: true,
            ..OpenOptions::default()
        },
    )?;
    assert_eq!(
        vfs.writev(descriptor, &[b"ab".to_vec(), b"cd".to_vec()], Some(0))?,
        4
    );
    assert_eq!(
        vfs.readv(descriptor, &[2, 2], Some(0))?,
        vec![b"ab".to_vec(), b"cd".to_vec()]
    );
    vfs.close(descriptor);
    Ok(())
}

#[test]
fn joins_child_paths_without_doubling_the_root_separator() {
    assert_eq!(join_child("/", "tmp"), "/tmp");
    assert_eq!(join_child("/tmp", "data"), "/tmp/data");
    assert_eq!(join_child("/tmp/data", "value.txt"), "/tmp/data/value.txt");
}

#[test]
fn resolves_paths_as_workerd_does() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut vfs = vfs(directory.path());
    vfs.write_file("/tmp/file", b"x", false, false)?;
    vfs.mkdir("/tmp/full/sub", true)?;
    vfs.symlink("/tmp/full", "/tmp/link")?;
    vfs.symlink("/tmp/missing", "/tmp/dangling")?;

    assert_eq!(
        vfs.stat("/tmp/file/x").err().map(|error| error.kind()),
        Some(ErrorKind::NotFound)
    );
    assert_eq!(vfs.stat("/tmp/link/sub")?.kind, NodeType::Directory);
    assert_eq!(
        vfs.lstat("/tmp/link/sub").err().map(|error| error.kind()),
        Some(ErrorKind::NotFound)
    );

    Ok(())
}

#[test]
fn creates_parents_as_workerd_does() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut vfs = vfs(directory.path());
    vfs.write_file("/tmp/file", b"x", false, false)?;
    vfs.mkdir("/tmp/full/sub", true)?;
    vfs.symlink("/tmp/full", "/tmp/link")?;
    vfs.symlink("/tmp/missing", "/tmp/dangling")?;

    assert_eq!(vfs.mkdir("/tmp/a/b/c", true)?.as_deref(), Some("/tmp/a"));
    assert_eq!(vfs.mkdir("/tmp/a/d", true)?, None);
    assert_eq!(
        vfs.mkdir("/tmp/link/new/leaf", true)?.as_deref(),
        Some("/tmp/link/new")
    );
    assert_eq!(vfs.stat("/tmp/full/new/leaf")?.kind, NodeType::Directory);
    assert_eq!(
        vfs.mkdir("/tmp/dangling/leaf", true)
            .err()
            .map(|error| error.kind()),
        Some(ErrorKind::AlreadyExists)
    );

    let creating = OpenOptions {
        write: true,
        create: true,
        ..OpenOptions::default()
    };
    assert_eq!(
        vfs.open("/tmp/dangling", creating)
            .err()
            .map(|error| error.kind()),
        Some(ErrorKind::Failed)
    );
    assert_eq!(
        vfs.open("/tmp/file/new", creating)
            .err()
            .map(|error| error.kind()),
        Some(ErrorKind::NotDirectory)
    );
    Ok(())
}
