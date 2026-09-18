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
    vfs.write_file("/tmp/value.txt", b"one", false)?;
    vfs.write_file("/tmp/value.txt", b" two", true)?;
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
    let error = match vfs.write_file("/bundle/nope", b"x", false) {
        Ok(()) => return Err("readonly bundle accepted a write".into()),
        Err(error) => error,
    };
    assert_eq!(error.kind(), ErrorKind::ReadOnly);
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
    vfs.close(null)?;

    let zero = vfs.open("/dev/zero", OpenOptions::default())?;
    assert_eq!(vfs.read(zero, 3, None)?, vec![0, 0, 0]);
    vfs.close(zero)?;

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
    vfs.write_file("/tmp/source", b"hello", false)?;
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
    vfs.close(fd)?;
    assert_eq!(vfs.read_file("/tmp/source")?, b"hello!");
    Ok(())
}

#[test]
fn enforces_descriptor_permissions_and_path_limits() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut vfs = vfs(directory.path());
    vfs.write_file("/tmp/value", b"hello", false)?;

    let read_only = vfs.open("/tmp/value", OpenOptions::default())?;
    assert_eq!(
        vfs.ftruncate(read_only, 0).err().map(|error| error.kind()),
        Some(ErrorKind::NotPermitted)
    );
    assert_eq!(vfs.read_file("/tmp/value")?, b"hello");
    vfs.close(read_only)?;

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
    vfs.write_file("/tmp/source.txt", b"hello", false)?;
    vfs.mkdir("/tmp/tree", false)?;
    vfs.write_file("/tmp/tree/nested.txt", b"nested", false)?;

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

    let temp = vfs.make_temp_dir("/tmp/prefix-")?;
    assert!(temp.starts_with("/tmp/prefix-"));
    assert_eq!(vfs.stat(&temp)?.kind, NodeType::Directory);

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
    vfs.close(descriptor)?;
    Ok(())
}

#[test]
fn joins_child_paths_without_doubling_the_root_separator() {
    assert_eq!(join_child("/", "tmp"), "/tmp");
    assert_eq!(join_child("/tmp", "data"), "/tmp/data");
    assert_eq!(join_child("/tmp/data", "value.txt"), "/tmp/data/value.txt");
}
