#![allow(clippy::wildcard_imports)]

use super::*;

#[allow(clippy::struct_excessive_bools)]
pub(super) struct FsOptions {
    pub(super) encoding: Option<String>,
    recursive: bool,
    force: bool,
    error_on_exist: bool,
    dereference: bool,
    with_file_types: bool,
    pub(super) bigint: bool,
    throw_if_no_entry: bool,
    cwd: Option<String>,
    blob_type: Option<String>,
}

impl Default for FsOptions {
    fn default() -> Self {
        Self {
            encoding: None,
            recursive: false,
            force: false,
            error_on_exist: false,
            dereference: false,
            with_file_types: false,
            bigint: false,
            throw_if_no_entry: true,
            cwd: None,
            blob_type: None,
        }
    }
}

pub(super) fn parse_options<'js>(
    ctx: &Ctx<'js>,
    value: Option<Value<'js>>,
) -> rquickjs::Result<FsOptions> {
    let Some(value) = value else {
        return Ok(FsOptions::default());
    };
    if value.is_undefined() || value.is_null() {
        return Ok(FsOptions::default());
    }
    if value.is_string() {
        return Ok(FsOptions {
            encoding: Some(
                value
                    .into_string()
                    .ok_or_else(|| Exception::throw_type(ctx, "expected a string"))?
                    .to_string()?,
            ),
            ..FsOptions::default()
        });
    }
    if value.is_bool() {
        return Ok(FsOptions::default());
    }
    let object = value
        .try_into_object()
        .map_err(|_| Exception::throw_type(ctx, "filesystem options must be a string or object"))?;
    Ok(FsOptions {
        encoding: object.get("encoding")?,
        recursive: object.get::<_, Option<bool>>("recursive")?.unwrap_or(false),
        force: object.get::<_, Option<bool>>("force")?.unwrap_or(false),
        error_on_exist: object
            .get::<_, Option<bool>>("errorOnExist")?
            .unwrap_or(false),
        dereference: object
            .get::<_, Option<bool>>("dereference")?
            .unwrap_or(false),
        with_file_types: object
            .get::<_, Option<bool>>("withFileTypes")?
            .unwrap_or(false),
        bigint: object.get::<_, Option<bool>>("bigint")?.unwrap_or(false),
        throw_if_no_entry: object
            .get::<_, Option<bool>>("throwIfNoEntry")?
            .unwrap_or(true),
        cwd: object.get("cwd")?,
        blob_type: object.get("type")?,
    })
}

pub(super) fn option_flag<'js>(
    ctx: &Ctx<'js>,
    value: Option<Value<'js>>,
) -> rquickjs::Result<Option<Value<'js>>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if !value.is_object() {
        return Ok(None);
    }
    let object = value
        .try_into_object()
        .map_err(|_| Exception::throw_type(ctx, "filesystem options must be a string or object"))?;
    object.get("flag")
}

pub(super) fn option_property<'js>(
    ctx: &Ctx<'js>,
    value: Option<&Value<'js>>,
    name: &str,
) -> rquickjs::Result<Option<Value<'js>>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if !value.is_object() {
        return Ok(None);
    }
    let object = value
        .clone()
        .try_into_object()
        .map_err(|_| Exception::throw_type(ctx, "filesystem options must be an object"))?;
    object.get(name)
}

pub(super) fn path<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<String> {
    let value: Coerced<std::string::String> = Coerced::from_js(ctx, value)?;
    let mut path = value.as_ref().clone();
    if !path.starts_with('/') {
        path = format!("/bundle/{path}");
    }
    if path.chars().count() > crate::fs::vfs::MAX_PATH_LENGTH {
        return Err(Exception::throw_range(ctx, "path is too long"));
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(Exception::throw_range(
                        ctx,
                        "path escapes the virtual filesystem",
                    ));
                }
            }
            part => {
                parts.push(part);
                if parts.len() > crate::fs::vfs::MAX_PATH_SEGMENTS {
                    return Err(Exception::throw_range(ctx, "path has too many segments"));
                }
            }
        }
    }
    let path = format!("/{}", parts.join("/"));
    if !["/", "/bundle", "/tmp", "/dev"]
        .iter()
        .any(|root| path == *root || path.starts_with(&format!("{root}/")))
    {
        return Err(Exception::throw_message(
            ctx,
            &format!("ENOENT: no such file or directory, '{path}'"),
        ));
    }
    Ok(path)
}

pub(super) enum PathOrFd {
    Path(String),
    Fd(u32),
}

pub(super) fn path_or_fd<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<PathOrFd> {
    if value.is_number() {
        return descriptor_value(ctx, value).map(PathOrFd::Fd);
    }
    path(ctx, value).map(PathOrFd::Path)
}

pub(super) fn descriptor_value<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<u32> {
    let descriptor: Coerced<i64> = Coerced::from_js(ctx, value)?;
    u32::try_from(*descriptor)
        .map_err(|_| Exception::throw_range(ctx, "file descriptor must be non-negative"))
}

pub(super) fn bytes<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
    encoding: Option<&str>,
) -> rquickjs::Result<Vec<u8>> {
    if value.is_string() {
        let text = value
            .into_string()
            .ok_or_else(|| Exception::throw_type(ctx, "expected a string"))?
            .to_string()?;
        return match encoding.unwrap_or("utf8").to_ascii_lowercase().as_str() {
            "base64" => Ok(decode_base64(&text)),
            "hex" => decode_hex(ctx, &text),
            "buffer" | "utf8" | "utf-8" => Ok(text.into_bytes()),
            _ => Err(Exception::throw_type(ctx, "unsupported encoding")),
        };
    }
    if let Some(buffer) = ArrayBuffer::from_value(value.clone()) {
        return buffer
            .as_bytes()
            .map(ToOwned::to_owned)
            .ok_or_else(|| Exception::throw_type(ctx, "array buffer is detached"));
    }
    let object = value.clone().try_into_object().map_err(|_| {
        Exception::throw_type(ctx, "data must be a string, ArrayBuffer, or typed array")
    })?;
    typed_array_bytes(&object).ok_or_else(|| {
        Exception::throw_type(ctx, "data must be a string, ArrayBuffer, or typed array")
    })
}

pub(super) fn write_buffer_value<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    bytes: &[u8],
) -> rquickjs::Result<Value<'js>> {
    if value.is_string() {
        Ok(TypedArray::<u8>::new_copy(ctx.clone(), bytes)?.into_value())
    } else {
        Ok(value.clone())
    }
}

pub(super) fn typed_array_bytes(object: &Object<'_>) -> Option<Vec<u8>> {
    macro_rules! typed_array {
        ($type:ty) => {
            if object.is_typed_array::<$type>() {
                return TypedArray::<$type>::from_object(object.clone())
                    .ok()?
                    .as_bytes()
                    .map(ToOwned::to_owned);
            }
        };
    }
    typed_array!(u8);
    typed_array!(i8);
    typed_array!(u16);
    typed_array!(i16);
    typed_array!(u32);
    typed_array!(i32);
    typed_array!(u64);
    typed_array!(i64);
    typed_array!(f32);
    typed_array!(f64);
    None
}

pub(super) fn is_buffer_value(value: &Value<'_>) -> bool {
    if ArrayBuffer::from_value(value.clone()).is_some() {
        return true;
    }
    value
        .clone()
        .try_into_object()
        .ok()
        .is_some_and(|object| typed_array_bytes(&object).is_some())
}

pub(super) fn output<'js>(
    ctx: Ctx<'js>,
    bytes: &[u8],
    encoding: Option<&str>,
) -> rquickjs::Result<Value<'js>> {
    match encoding.map(str::to_ascii_lowercase).as_deref() {
        None | Some("buffer") => Ok(TypedArray::<u8>::new_copy(ctx, bytes)?.into_value()),
        Some("base64") => Ok(base64_encode(bytes).into_js(&ctx)?),
        Some("hex") => {
            use std::fmt::Write as _;
            let mut value = String::with_capacity(bytes.len() * 2);
            for byte in bytes {
                let _ = write!(&mut value, "{byte:02x}");
            }
            Ok(value.into_js(&ctx)?)
        }
        Some("utf8" | "utf-8") => Ok(String::from_utf8_lossy(bytes).into_owned().into_js(&ctx)?),
        Some(_) => Err(Exception::throw_type(&ctx, "unsupported encoding")),
    }
}

pub(super) fn read_descriptor(ctx: &Ctx<'_>, descriptor: u32) -> rquickjs::Result<Vec<u8>> {
    let size = vfs_call(ctx, |vfs| vfs.fstat(descriptor))?.size;
    let size = usize::try_from(size)
        .map_err(|_| Exception::throw_range(ctx, "file is too large to read"))?;
    vfs_call(ctx, |vfs| vfs.read(descriptor, size, Some(0)))
}

pub(super) fn write_descriptor(
    ctx: &Ctx<'_>,
    descriptor: u32,
    data: &[u8],
    append: bool,
) -> rquickjs::Result<usize> {
    if append {
        let position = vfs_call(ctx, |vfs| vfs.fstat(descriptor).map(|stat| stat.size))?;
        vfs_call(ctx, |vfs| vfs.write(descriptor, data, Some(position)))
    } else {
        vfs_call(ctx, |vfs| vfs.ftruncate(descriptor, 0))?;
        vfs_call(ctx, |vfs| vfs.write(descriptor, data, Some(0)))
    }
}

pub(super) fn read_file_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let options = parse_options(&ctx, options.0)?;
    let bytes = match path_or_fd(&ctx, input)? {
        PathOrFd::Path(path) => vfs_call(&ctx, |vfs| vfs.read_file(&path))?,
        PathOrFd::Fd(descriptor) => read_descriptor(&ctx, descriptor)?,
    };
    output(ctx, &bytes, options.encoding.as_deref())
}

pub(super) fn write_file_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    data: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<()> {
    let options_value = options.0;
    let options = parse_options(&ctx, options_value.clone())?;
    let flag = option_flag(&ctx, options_value)?;
    let append = flag_string(flag.as_ref())?.is_some_and(|flag| flag.contains('a'));
    let bytes = bytes(&ctx, data, options.encoding.as_deref())?;
    match path_or_fd(&ctx, input)? {
        PathOrFd::Path(path) => {
            let open_options = if let Some(flag) = flag {
                open_options(&ctx, Some(flag))?
            } else {
                OpenOptions {
                    read: false,
                    write: true,
                    create: true,
                    truncate: true,
                    ..OpenOptions::default()
                }
            };
            vfs_call(&ctx, |vfs| {
                let descriptor = vfs.open(&path, open_options)?;
                let result = vfs.write(descriptor, &bytes, None).map(|_| ());
                result.and(vfs.close(descriptor))
            })
        }
        PathOrFd::Fd(descriptor) => {
            write_descriptor(&ctx, descriptor, &bytes, append)?;
            Ok(())
        }
    }
}

pub(super) fn append_file_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    data: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<()> {
    let options = parse_options(&ctx, options.0)?;
    let bytes = bytes(&ctx, data, options.encoding.as_deref())?;
    match path_or_fd(&ctx, input)? {
        PathOrFd::Path(path) => vfs_call(&ctx, |vfs| vfs.write_file(&path, &bytes, true)),
        PathOrFd::Fd(descriptor) => {
            write_descriptor(&ctx, descriptor, &bytes, true)?;
            Ok(())
        }
    }
}

pub(super) fn access_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    mode: Opt<u32>,
) -> rquickjs::Result<()> {
    let path = path(&ctx, input)?;
    vfs_call(&ctx, |vfs| vfs.access(&path, mode.0.unwrap_or(0)))
}

pub(super) fn chmod_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    mode: Value<'js>,
) -> rquickjs::Result<()> {
    let path = path(&ctx, input)?;
    validate_mode(&ctx, mode)?;
    vfs_call(&ctx, |vfs| vfs.stat(&path).map(|_| ()))
}

pub(super) fn chown_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    _uid: u32,
    _gid: u32,
) -> rquickjs::Result<()> {
    let path = path(&ctx, input)?;
    vfs_call(&ctx, |vfs| vfs.stat(&path).map(|_| ()))
}

pub(super) fn fchmod_sync<'js>(
    ctx: Ctx<'js>,
    descriptor: u32,
    mode: Value<'js>,
) -> rquickjs::Result<()> {
    validate_mode(&ctx, mode)?;
    vfs_call(&ctx, |vfs| vfs.fstat(descriptor).map(|_| ()))
}

pub(super) fn fchown_sync(
    ctx: Ctx<'_>,
    descriptor: u32,
    _uid: u32,
    _gid: u32,
) -> rquickjs::Result<()> {
    vfs_call(&ctx, |vfs| vfs.fstat(descriptor).map(|_| ()))
}

pub(super) fn fdatasync_sync(ctx: Ctx<'_>, descriptor: u32) -> rquickjs::Result<()> {
    vfs_call(&ctx, |vfs| vfs.fstat(descriptor).map(|_| ()))
}

pub(super) fn fsync_sync(ctx: Ctx<'_>, descriptor: u32) -> rquickjs::Result<()> {
    vfs_call(&ctx, |vfs| vfs.fstat(descriptor).map(|_| ()))
}

pub(super) fn futimes_sync(
    ctx: Ctx<'_>,
    descriptor: u32,
    _atime: Value<'_>,
    _mtime: Value<'_>,
) -> rquickjs::Result<()> {
    vfs_call(&ctx, |vfs| vfs.fstat(descriptor).map(|_| ()))
}

pub(super) fn lchmod_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    mode: Value<'js>,
) -> rquickjs::Result<()> {
    let path = path(&ctx, input)?;
    validate_mode(&ctx, mode)?;
    vfs_call(&ctx, |vfs| vfs.lstat(&path).map(|_| ()))
}

pub(super) fn lchown_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    _uid: u32,
    _gid: u32,
) -> rquickjs::Result<()> {
    let path = path(&ctx, input)?;
    vfs_call(&ctx, |vfs| vfs.lstat(&path).map(|_| ()))
}

pub(super) fn lutimes_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    _atime: Value<'js>,
    _mtime: Value<'js>,
) -> rquickjs::Result<()> {
    let path = path(&ctx, input)?;
    vfs_call(&ctx, |vfs| vfs.lstat(&path).map(|_| ()))
}

pub(super) fn utimes_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    _atime: Value<'js>,
    _mtime: Value<'js>,
) -> rquickjs::Result<()> {
    let path = path(&ctx, input)?;
    vfs_call(&ctx, |vfs| vfs.stat(&path).map(|_| ()))
}

pub(super) fn mkdir_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<()> {
    let path = path(&ctx, input)?;
    let options_value = options.0;
    let options = if let Some(value) = options_value.as_ref()
        && value.is_number()
    {
        validate_mode(&ctx, value.clone())?;
        FsOptions::default()
    } else {
        if let Some(mode) = option_property(&ctx, options_value.as_ref(), "mode")?
            && !mode.is_undefined()
        {
            validate_mode(&ctx, mode)?;
        }
        parse_options(&ctx, options_value)?
    };
    vfs_call(&ctx, |vfs| vfs.mkdir(&path, options.recursive))
}

pub(super) fn readdir_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Array<'js>> {
    let path = path(&ctx, input)?;
    let options = parse_options(&ctx, options.0)?;
    let entries: Vec<(String, DirectoryEntry)> = if options.recursive {
        vfs_call(&ctx, |vfs| vfs.walk(&path))?
    } else {
        vfs_call(&ctx, |vfs| vfs.read_dir(&path))?
            .into_iter()
            .map(|entry| {
                let entry_path = if path == "/" {
                    format!("/{name}", name = entry.name)
                } else {
                    format!("{path}/{name}", name = entry.name)
                };
                (entry_path, entry)
            })
            .collect()
    };
    let result = Array::new(ctx.clone())?;
    for (index, (entry_path, entry)) in entries.into_iter().enumerate() {
        let name = if options.recursive {
            entry_path
                .strip_prefix(&path)
                .unwrap_or(&entry_path)
                .trim_start_matches('/')
                .to_owned()
        } else {
            entry.name.clone()
        };
        if options.with_file_types {
            let parent_path = entry_path
                .rsplit_once('/')
                .map_or(path.as_str(), |(parent, _)| normalized_parent(parent));
            result.set(
                index,
                dirent(
                    &ctx,
                    &entry,
                    name_value(&ctx, &name, options.encoding.as_deref())?,
                    parent_path,
                )?,
            )?;
        } else {
            result.set(index, name_value(&ctx, &name, options.encoding.as_deref())?)?;
        }
    }
    Ok(result)
}

pub(super) fn stat_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let options = parse_options(&ctx, options.0)?;
    match path_or_fd(&ctx, input)? {
        PathOrFd::Path(path) => stat_value(&ctx, &path, true, &options),
        PathOrFd::Fd(descriptor) => {
            let stat = vfs_call(&ctx, |vfs| vfs.fstat(descriptor))?;
            stat_object(&ctx, &stat, options.bigint).map(Object::into_value)
        }
    }
}

pub(super) fn lstat_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let path = path(&ctx, input)?;
    let options = parse_options(&ctx, options.0)?;
    stat_value(&ctx, &path, false, &options)
}

pub(super) fn stat_value<'js>(
    ctx: &Ctx<'js>,
    path: &str,
    follow_symlinks: bool,
    options: &FsOptions,
) -> rquickjs::Result<Value<'js>> {
    let vfs = vfs_handle(ctx)?;
    let result = if follow_symlinks {
        lock(&vfs).stat(path)
    } else {
        lock(&vfs).lstat(path)
    };
    match result {
        Ok(stat) => stat_object(ctx, &stat, options.bigint).map(Object::into_value),
        Err(error)
            if error.kind() == crate::fs::vfs::ErrorKind::NotFound
                && !options.throw_if_no_entry =>
        {
            Ok(Value::new_undefined(ctx.clone()))
        }
        Err(error) => Err(vfs_exception(ctx, &error)?.throw()),
    }
}

pub(super) fn exists_sync<'js>(ctx: Ctx<'js>, input: Value<'js>) -> rquickjs::Result<bool> {
    let path = path(&ctx, input)?;
    let vfs = vfs_handle(&ctx)?;
    Ok(lock(&vfs).stat(&path).is_ok())
}

pub(super) fn unlink_sync<'js>(ctx: Ctx<'js>, input: Value<'js>) -> rquickjs::Result<()> {
    let path = path(&ctx, input)?;
    vfs_call(&ctx, |vfs| vfs.remove_file(&path))
}

pub(super) fn rm_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<()> {
    let path = path(&ctx, input)?;
    let options = parse_options(&ctx, options.0)?;
    vfs_call(&ctx, |vfs| {
        vfs.remove(&path, options.recursive, options.force)
    })
}

pub(super) fn rmdir_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<()> {
    let path = path(&ctx, input)?;
    let options = parse_options(&ctx, options.0)?;
    vfs_call(&ctx, |vfs| vfs.remove_directory(&path, options.recursive))
}

pub(super) fn rename_sync<'js>(
    ctx: Ctx<'js>,
    from: Value<'js>,
    to: Value<'js>,
) -> rquickjs::Result<()> {
    let from = path(&ctx, from)?;
    let to = path(&ctx, to)?;
    vfs_call(&ctx, |vfs| vfs.rename(&from, &to))
}

pub(super) fn copy_file_sync<'js>(
    ctx: Ctx<'js>,
    from: Value<'js>,
    to: Value<'js>,
    mode: Opt<u32>,
) -> rquickjs::Result<()> {
    let from = path(&ctx, from)?;
    let to = path(&ctx, to)?;
    let mode = mode.0.unwrap_or_default();
    if !matches!(mode, 0..=2) {
        return Err(Exception::throw_range(&ctx, "unsupported copy mode"));
    }
    vfs_call(&ctx, |vfs| {
        vfs.copy_file_with_options(&from, &to, mode & 1 != 0)
    })
}

pub(super) fn cp_sync<'js>(
    ctx: Ctx<'js>,
    from: Value<'js>,
    to: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<()> {
    let from = path(&ctx, from)?;
    let to = path(&ctx, to)?;
    let options_value = options.0;
    if let Some(mode) = option_property(&ctx, options_value.as_ref(), "mode")? {
        let mode = Coerced::<i64>::from_js(&ctx, mode)?.0;
        let mode = u32::try_from(mode)
            .map_err(|_| Exception::throw_range(&ctx, "options.mode is out of range"))?;
        if mode & 4 != 0 {
            return Err(Exception::throw_message(
                &ctx,
                "COPYFILE_FICLONE_FORCE is not supported",
            ));
        }
    }
    if let Some(filter) = option_property(&ctx, options_value.as_ref(), "filter")?
        && !filter.is_undefined()
    {
        if !filter.is_function() {
            return Err(Exception::throw_type(
                &ctx,
                "options.filter must be a function",
            ));
        }
        return Err(Exception::throw_message(
            &ctx,
            "options.filter is not supported",
        ));
    }
    for name in ["preserveTimestamps", "verbatimSymlinks"] {
        if let Some(value) = option_property(&ctx, options_value.as_ref(), name)? {
            let _ = bool::from_js(&ctx, value)?;
        }
    }
    let mut options = parse_options(&ctx, options_value.clone())?;
    options.force = option_property(&ctx, options_value.as_ref(), "force")?
        .map(|value| bool::from_js(&ctx, value))
        .transpose()?
        .unwrap_or(true);
    vfs_call(&ctx, |vfs| {
        vfs.copy(
            &from,
            &to,
            CopyOptions {
                recursive: options.recursive,
                force: options.force,
                error_on_exist: options.error_on_exist,
                dereference: options.dereference,
            },
        )
    })
}

pub(super) fn link_sync<'js>(
    ctx: Ctx<'js>,
    existing: Value<'js>,
    new: Value<'js>,
) -> rquickjs::Result<()> {
    let existing = path(&ctx, existing)?;
    let new = path(&ctx, new)?;
    vfs_call(&ctx, |vfs| vfs.link(&existing, &new))
}

pub(super) fn mkdtemp_sync<'js>(
    ctx: Ctx<'js>,
    prefix: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let prefix = path(&ctx, prefix)?;
    let directory = vfs_call(&ctx, |vfs| vfs.make_temp_dir(&prefix))?;
    let options = parse_options(&ctx, options.0)?;
    text_value(&ctx, &directory, options.encoding.as_deref())
}

pub(super) fn opendir_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Object<'js>> {
    let path = path(&ctx, input)?;
    let options = parse_options(&ctx, options.0)?;
    let entries = if options.recursive {
        vfs_call(&ctx, |vfs| vfs.walk(&path))?
            .into_iter()
            .map(|(entry_path, entry)| DirItem {
                name: entry_path
                    .strip_prefix(&path)
                    .unwrap_or(&entry_path)
                    .trim_start_matches('/')
                    .to_owned(),
                parent: entry_path.rsplit_once('/').map_or_else(
                    || "/".to_owned(),
                    |(parent, _)| normalized_parent(parent).to_owned(),
                ),
                entry,
            })
            .collect()
    } else {
        vfs_call(&ctx, |vfs| vfs.read_dir(&path))?
            .into_iter()
            .map(|entry| DirItem {
                name: entry.name.clone(),
                parent: path.clone(),
                entry,
            })
            .collect()
    };
    dir_object(&ctx, path, entries, options.encoding.as_deref())
}

pub(super) fn readv_sync<'js>(
    ctx: Ctx<'js>,
    descriptor: u32,
    buffers: Array<'js>,
    position: Opt<Value<'js>>,
) -> rquickjs::Result<u32> {
    let position = position_value(&ctx, position.0)?;
    let mut lengths = Vec::with_capacity(buffers.len());
    let mut writable = Vec::with_capacity(buffers.len());
    for index in 0..buffers.len() {
        let value: Value = buffers.get(index)?;
        let bytes = writable_bytes(&ctx, value)?;
        lengths.push(bytes.len);
        writable.push(bytes);
    }
    let chunks = vfs_call(&ctx, |vfs| vfs.readv(descriptor, &lengths, position))?;
    let mut total = 0_usize;
    for (buffer, bytes) in writable.iter().zip(chunks) {
        buffer.write(0, &bytes);
        total += bytes.len();
    }
    u32::try_from(total).map_err(|_| Exception::throw_range(&ctx, "read is too large"))
}

pub(super) fn writev_sync<'js>(
    ctx: Ctx<'js>,
    descriptor: u32,
    buffers: Array<'js>,
    position: Opt<Value<'js>>,
) -> rquickjs::Result<u32> {
    let position = position_value(&ctx, position.0)?;
    let mut values = Vec::with_capacity(buffers.len());
    for index in 0..buffers.len() {
        let value: Value = buffers.get(index)?;
        values.push(bytes(&ctx, value, None)?);
    }
    let written = vfs_call(&ctx, |vfs| vfs.writev(descriptor, &values, position))?;
    u32::try_from(written).map_err(|_| Exception::throw_range(&ctx, "write is too large"))
}

pub(super) fn statfs_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Object<'js>> {
    let _path = path(&ctx, input)?;
    let options = parse_options(&ctx, options.0)?;
    statfs_object(&ctx, options.bigint)
}

pub(super) fn open_as_blob<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Object<'js>> {
    let path = path(&ctx, input)?;
    let options = parse_options(&ctx, options.0)?;
    let bytes = vfs_call(&ctx, |vfs| vfs.read_file(&path))?;
    blob_object(&ctx, bytes, options.blob_type.as_deref().unwrap_or(""))
}

pub(super) fn glob_sync<'js>(
    ctx: Ctx<'js>,
    pattern: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Array<'js>> {
    let options_value = options.0;
    let exclude = option_property(&ctx, options_value.as_ref(), "exclude")?;
    let options = parse_options(&ctx, options_value)?;
    let patterns = if pattern.is_array() {
        let patterns = Array::from_value(pattern)?;
        let mut values = Vec::with_capacity(patterns.len());
        for index in 0..patterns.len() {
            values.push(patterns.get::<String>(index)?);
        }
        values
    } else {
        vec![Coerced::<String>::from_js(&ctx, pattern)?.0]
    };
    let cwd = options.cwd.clone().unwrap_or_else(|| "/bundle".to_owned());
    let cwd = path(&ctx, cwd.into_js(&ctx)?)?;
    let mut matches: Vec<Value<'js>> = Vec::new();
    for pattern in patterns {
        for (path, entry) in glob_matches(&ctx, &cwd, &pattern, &options)? {
            let relative = if pattern.starts_with('/') {
                path.clone()
            } else {
                path.strip_prefix(&cwd)
                    .unwrap_or(&path)
                    .trim_start_matches('/')
                    .to_owned()
            };
            if glob_excluded(
                &ctx,
                exclude.as_ref(),
                &relative,
                &entry,
                options.with_file_types,
                &path,
            )? {
                continue;
            }
            if options.with_file_types {
                let name = path.rsplit('/').next().unwrap_or_default();
                let parent = path.rsplit_once('/').map_or("/", |(parent, _)| parent);
                matches.push(
                    dirent(
                        &ctx,
                        &entry,
                        name_value(&ctx, name, options.encoding.as_deref())?,
                        normalized_parent(parent),
                    )?
                    .into_value(),
                );
            } else {
                matches.push(name_value(
                    &ctx,
                    if pattern.starts_with('/') {
                        &path
                    } else {
                        path.strip_prefix(&cwd)
                            .unwrap_or(&path)
                            .trim_start_matches('/')
                    },
                    options.encoding.as_deref(),
                )?);
            }
        }
    }
    let result = Array::new(ctx.clone())?;
    for (index, value) in matches.into_iter().enumerate() {
        result.set(index, value)?;
    }
    Ok(result)
}

pub(super) fn glob_excluded<'js>(
    ctx: &Ctx<'js>,
    exclude: Option<&Value<'js>>,
    relative: &str,
    entry: &DirectoryEntry,
    with_file_types: bool,
    parent_path: &str,
) -> rquickjs::Result<bool> {
    let Some(exclude) = exclude else {
        return Ok(false);
    };
    if let Ok(function) = Function::from_value(exclude.clone()) {
        let value = if with_file_types {
            dirent(
                ctx,
                entry,
                name_value(ctx, relative.rsplit('/').next().unwrap_or_default(), None)?,
                parent_path
                    .rsplit_once('/')
                    .map_or("/", |(parent, _)| normalized_parent(parent)),
            )?
            .into_value()
        } else {
            relative.to_owned().into_js(ctx)?
        };
        return function.call::<_, bool>((value,));
    }
    let patterns = Array::from_value(exclude.clone())
        .map_err(|_| Exception::throw_type(ctx, "options.exclude must be a function or array"))?;
    for index in 0..patterns.len() {
        let pattern: String = patterns.get(index)?;
        if glob_match(&pattern, relative) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn symlink_sync<'js>(
    ctx: Ctx<'js>,
    target: Value<'js>,
    input: Value<'js>,
) -> rquickjs::Result<()> {
    let target: Coerced<std::string::String> = Coerced::from_js(&ctx, target)?;
    let path = path(&ctx, input)?;
    vfs_call(&ctx, |vfs| vfs.symlink(target.as_ref(), &path))
}

pub(super) fn read_link_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let path = path(&ctx, input)?;
    let target = vfs_call(&ctx, |vfs| vfs.read_link(&path))?;
    let options = parse_options(&ctx, options.0)?;
    text_value(&ctx, &target, options.encoding.as_deref())
}

pub(super) fn realpath_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let path = path(&ctx, input)?;
    let target = vfs_call(&ctx, |vfs| vfs.realpath(&path))?;
    let options = parse_options(&ctx, options.0)?;
    text_value(&ctx, &target, options.encoding.as_deref())
}

pub(super) fn open_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    flags: Opt<Value<'js>>,
    mode: Opt<Value<'js>>,
) -> rquickjs::Result<u32> {
    let path = path(&ctx, input)?;
    if let Some(mode) = mode.0 {
        validate_mode(&ctx, mode)?;
    }
    let options = open_options(&ctx, flags.0)?;
    vfs_call(&ctx, |vfs| vfs.open(&path, options))
}

pub(super) fn close_sync(ctx: Ctx<'_>, descriptor: u32) -> rquickjs::Result<()> {
    vfs_call(&ctx, |vfs| vfs.close(descriptor))
}

pub(super) fn fstat_sync<'js>(
    ctx: Ctx<'js>,
    descriptor: u32,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Object<'js>> {
    let options = parse_options(&ctx, options.0)?;
    let stat = vfs_call(&ctx, |vfs| vfs.fstat(descriptor))?;
    stat_object(&ctx, &stat, options.bigint)
}

pub(super) fn read_sync<'js>(
    ctx: Ctx<'js>,
    descriptor: u32,
    buffer: Value<'js>,
    offset: Opt<u32>,
    length: Opt<u32>,
    position: Opt<Value<'js>>,
) -> rquickjs::Result<u32> {
    let buffer = writable_bytes(&ctx, buffer)?;
    let offset = offset.0.unwrap_or(0) as usize;
    if offset > buffer.len {
        return Err(Exception::throw_range(&ctx, "offset is outside the buffer"));
    }
    let length = length
        .0
        .map_or(buffer.len - offset, |length| length as usize);
    if length > buffer.len - offset {
        return Err(Exception::throw_range(&ctx, "length is outside the buffer"));
    }
    let position = position_value(&ctx, position.0)?;
    let bytes = vfs_call(&ctx, |vfs| vfs.read(descriptor, length, position))?;
    buffer.write(offset, &bytes);
    u32::try_from(bytes.len()).map_err(|_| Exception::throw_range(&ctx, "read is too large"))
}

pub(super) fn read_sync_export<'js>(
    ctx: Ctx<'js>,
    descriptor: u32,
    buffer: Value<'js>,
    offset_or_options: Opt<Value<'js>>,
    length: Opt<Value<'js>>,
    position: Opt<Value<'js>>,
) -> rquickjs::Result<u32> {
    let (offset, length, position) = if let Some(value) = offset_or_options.0 {
        if value.is_object() {
            let options = value
                .clone()
                .try_into_object()
                .map_err(|_| Exception::throw_type(&ctx, "read options must be an object"))?;
            (
                options.get("offset")?,
                options.get("length")?,
                options.get("position")?,
            )
        } else {
            (Some(value), length.0, position.0)
        }
    } else {
        (None, length.0, position.0)
    };
    read_sync(
        ctx.clone(),
        descriptor,
        buffer,
        optional_u32(&ctx, offset)?,
        optional_u32(&ctx, length)?,
        Opt(position),
    )
}

pub(super) fn write_sync<'js>(
    ctx: Ctx<'js>,
    descriptor: u32,
    value: Value<'js>,
    args: Rest<Value<'js>>,
) -> rquickjs::Result<u32> {
    let (bytes, position) = write_arguments(&ctx, value, args.0)?;
    let written = vfs_call(&ctx, |vfs| vfs.write(descriptor, &bytes, position))?;
    u32::try_from(written).map_err(|_| Exception::throw_range(&ctx, "write is too large"))
}

pub(super) fn write_sync_export<'js>(
    ctx: Ctx<'js>,
    descriptor: u32,
    value: Value<'js>,
    offset_or_options: Opt<Value<'js>>,
    length: Opt<Value<'js>>,
    position: Opt<Value<'js>>,
) -> rquickjs::Result<u32> {
    let data_is_string = value.is_string();
    let args = if let Some(value) = offset_or_options.0 {
        if value.is_object() {
            let options = value
                .clone()
                .try_into_object()
                .map_err(|_| Exception::throw_type(&ctx, "write options must be an object"))?;
            let offset = options.get::<_, Option<Value>>("offset")?;
            let length = options.get::<_, Option<Value>>("length")?;
            let position = options.get::<_, Option<Value>>("position")?;
            if data_is_string {
                vec![
                    position.unwrap_or_else(|| Value::new_undefined(ctx.clone())),
                    options
                        .get::<_, Option<Value>>("encoding")?
                        .unwrap_or_else(|| Value::new_undefined(ctx.clone())),
                ]
            } else {
                vec![
                    offset.unwrap_or_else(|| Value::new_undefined(ctx.clone())),
                    length.unwrap_or_else(|| Value::new_undefined(ctx.clone())),
                    position.unwrap_or_else(|| Value::new_undefined(ctx.clone())),
                ]
            }
        } else {
            [Some(value), length.0, position.0]
                .into_iter()
                .flatten()
                .collect()
        }
    } else {
        [None, length.0, position.0].into_iter().flatten().collect()
    };
    write_sync(ctx, descriptor, value, Rest(args))
}

pub(super) fn truncate_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    length: Opt<u64>,
) -> rquickjs::Result<()> {
    let path = path(&ctx, input)?;
    vfs_call(&ctx, |vfs| vfs.truncate(&path, length.0.unwrap_or(0)))
}

pub(super) fn ftruncate_sync(
    ctx: Ctx<'_>,
    descriptor: u32,
    length: Opt<u64>,
) -> rquickjs::Result<()> {
    vfs_call(&ctx, |vfs| vfs.ftruncate(descriptor, length.0.unwrap_or(0)))
}

pub(super) fn read_file_promise<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Promise<'js>> {
    promise(ctx.clone(), read_file_sync(ctx, input, options))
}

pub(super) fn write_file_promise<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    data: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Promise<'js>> {
    promise(
        ctx.clone(),
        write_file_sync(ctx.clone(), input, data, options)
            .map(|()| Value::new_undefined(ctx.clone())),
    )
}

pub(super) fn append_file_promise<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    data: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Promise<'js>> {
    promise(
        ctx.clone(),
        append_file_sync(ctx.clone(), input, data, options)
            .map(|()| Value::new_undefined(ctx.clone())),
    )
}

pub(super) fn mkdir_promise<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Promise<'js>> {
    promise(
        ctx.clone(),
        mkdir_sync(ctx.clone(), input, options).map(|()| Value::new_undefined(ctx.clone())),
    )
}

pub(super) fn readdir_promise<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Promise<'js>> {
    promise(
        ctx.clone(),
        readdir_sync(ctx.clone(), input, options).map(Array::into_value),
    )
}

pub(super) fn stat_promise<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Promise<'js>> {
    promise(ctx.clone(), stat_sync(ctx.clone(), input, options))
}

pub(super) fn lstat_promise<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Promise<'js>> {
    promise(ctx.clone(), lstat_sync(ctx.clone(), input, options))
}

pub(super) fn unlink_promise<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
) -> rquickjs::Result<Promise<'js>> {
    promise(
        ctx.clone(),
        unlink_sync(ctx.clone(), input).map(|()| Value::new_undefined(ctx.clone())),
    )
}

pub(super) fn rm_promise<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Promise<'js>> {
    promise(
        ctx.clone(),
        rm_sync(ctx.clone(), input, options).map(|()| Value::new_undefined(ctx.clone())),
    )
}

pub(super) fn rmdir_promise<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Promise<'js>> {
    promise(
        ctx.clone(),
        rmdir_sync(ctx.clone(), input, options).map(|()| Value::new_undefined(ctx)),
    )
}

pub(super) fn rename_promise<'js>(
    ctx: Ctx<'js>,
    from: Value<'js>,
    to: Value<'js>,
) -> rquickjs::Result<Promise<'js>> {
    promise(
        ctx.clone(),
        rename_sync(ctx.clone(), from, to).map(|()| Value::new_undefined(ctx.clone())),
    )
}

pub(super) fn copy_file_promise<'js>(
    ctx: Ctx<'js>,
    from: Value<'js>,
    to: Value<'js>,
    mode: Opt<u32>,
) -> rquickjs::Result<Promise<'js>> {
    promise(
        ctx.clone(),
        copy_file_sync(ctx.clone(), from, to, mode).map(|()| Value::new_undefined(ctx.clone())),
    )
}

pub(super) fn access_promise<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    mode: Opt<u32>,
) -> rquickjs::Result<Promise<'js>> {
    promise(
        ctx.clone(),
        access_sync(ctx.clone(), input, mode).map(|()| Value::new_undefined(ctx)),
    )
}

pub(super) fn chmod_promise<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    mode: Value<'js>,
) -> rquickjs::Result<Promise<'js>> {
    promise(
        ctx.clone(),
        chmod_sync(ctx.clone(), input, mode).map(|()| Value::new_undefined(ctx)),
    )
}

pub(super) fn chown_promise<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    uid: u32,
    gid: u32,
) -> rquickjs::Result<Promise<'js>> {
    promise(
        ctx.clone(),
        chown_sync(ctx.clone(), input, uid, gid).map(|()| Value::new_undefined(ctx)),
    )
}

pub(super) fn cp_promise<'js>(
    ctx: Ctx<'js>,
    from: Value<'js>,
    to: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Promise<'js>> {
    promise(
        ctx.clone(),
        cp_sync(ctx.clone(), from, to, options).map(|()| Value::new_undefined(ctx)),
    )
}

pub(super) fn glob_promise<'js>(
    ctx: Ctx<'js>,
    pattern: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Object<'js>> {
    let values = glob_sync(ctx.clone(), pattern, options)?;
    let values = (0..values.len())
        .map(|index| values.get(index))
        .collect::<rquickjs::Result<Vec<Value>>>()?;
    async_value_iterator(&ctx, values)
}

#[allow(clippy::arc_with_non_send_sync)]
pub(super) fn async_value_iterator<'js>(
    ctx: &Ctx<'js>,
    values: Vec<Value<'js>>,
) -> rquickjs::Result<Object<'js>> {
    let state = Arc::new(Mutex::new(values));
    let iterator = Object::new(ctx.clone())?;
    let next_state = Arc::clone(&state);
    iterator.set(
        "next",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
            let mut values = lock(&next_state);
            let value = values
                .is_empty()
                .then(|| Value::new_undefined(ctx.clone()))
                .or_else(|| Some(values.remove(0)));
            let done = value.as_ref().is_some_and(Value::is_undefined);
            let result = Object::new(ctx.clone())?;
            result.set("done", done)?;
            result.set(
                "value",
                value.unwrap_or_else(|| Value::new_undefined(ctx.clone())),
            )?;
            promise(ctx.clone(), Ok(result.into_value())).map(Promise::into_value)
        })?,
    )?;
    iterator.set(
        Symbol::async_iterator(ctx.clone()),
        Function::new(ctx.clone(), |this: This<Object<'js>>| {
            Ok::<Object<'js>, rquickjs::Error>(this.0)
        })?,
    )?;
    Ok(iterator)
}

pub(super) fn lchmod_promise<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    mode: Value<'js>,
) -> rquickjs::Result<Promise<'js>> {
    promise(
        ctx.clone(),
        lchmod_sync(ctx.clone(), input, mode).map(|()| Value::new_undefined(ctx)),
    )
}

pub(super) fn lchown_promise<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    uid: u32,
    gid: u32,
) -> rquickjs::Result<Promise<'js>> {
    promise(
        ctx.clone(),
        lchown_sync(ctx.clone(), input, uid, gid).map(|()| Value::new_undefined(ctx)),
    )
}

pub(super) fn link_promise<'js>(
    ctx: Ctx<'js>,
    existing: Value<'js>,
    new: Value<'js>,
) -> rquickjs::Result<Promise<'js>> {
    promise(
        ctx.clone(),
        link_sync(ctx.clone(), existing, new).map(|()| Value::new_undefined(ctx)),
    )
}

pub(super) fn lutimes_promise<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    atime: Value<'js>,
    mtime: Value<'js>,
) -> rquickjs::Result<Promise<'js>> {
    promise(
        ctx.clone(),
        lutimes_sync(ctx.clone(), input, atime, mtime).map(|()| Value::new_undefined(ctx)),
    )
}

pub(super) fn mkdtemp_promise<'js>(
    ctx: Ctx<'js>,
    prefix: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Promise<'js>> {
    promise(ctx.clone(), mkdtemp_sync(ctx.clone(), prefix, options))
}

pub(super) fn open_promise<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    flags: Opt<Value<'js>>,
    mode: Opt<Value<'js>>,
) -> rquickjs::Result<Promise<'js>> {
    let descriptor = open_sync(ctx.clone(), input, flags, mode)?;
    let state = FileHandleState {
        descriptor: Arc::new(Mutex::new(Some(descriptor))),
        owns_descriptor: true,
    };
    promise(
        ctx.clone(),
        file_handle_object(&ctx, state).map(Object::into_value),
    )
}

pub(super) fn opendir_promise<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Promise<'js>> {
    promise(
        ctx.clone(),
        opendir_sync(ctx.clone(), input, options).map(Object::into_value),
    )
}

pub(super) fn statfs_promise<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Promise<'js>> {
    promise(
        ctx.clone(),
        statfs_sync(ctx.clone(), input, options).map(Object::into_value),
    )
}

pub(super) fn utimes_promise<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    atime: Value<'js>,
    mtime: Value<'js>,
) -> rquickjs::Result<Promise<'js>> {
    promise(
        ctx.clone(),
        utimes_sync(ctx.clone(), input, atime, mtime).map(|()| Value::new_undefined(ctx)),
    )
}

pub(super) fn symlink_promise<'js>(
    ctx: Ctx<'js>,
    target: Value<'js>,
    input: Value<'js>,
) -> rquickjs::Result<Promise<'js>> {
    promise(
        ctx.clone(),
        symlink_sync(ctx.clone(), target, input).map(|()| Value::new_undefined(ctx.clone())),
    )
}

pub(super) fn read_link_promise<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Promise<'js>> {
    promise(ctx.clone(), read_link_sync(ctx, input, options))
}

pub(super) fn realpath_promise<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Promise<'js>> {
    promise(ctx.clone(), realpath_sync(ctx, input, options))
}

pub(super) fn truncate_promise<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    length: Opt<u64>,
) -> rquickjs::Result<Promise<'js>> {
    promise(
        ctx.clone(),
        truncate_sync(ctx.clone(), input, length).map(|()| Value::new_undefined(ctx.clone())),
    )
}

pub(super) fn promise<'js>(
    ctx: Ctx<'js>,
    result: rquickjs::Result<Value<'js>>,
) -> rquickjs::Result<Promise<'js>> {
    let (promise, resolve, reject) = Promise::new(&ctx)?;
    match result {
        Ok(value) => resolve.call::<_, ()>((value,))?,
        Err(_error) if ctx.has_exception() => reject.call::<_, ()>((ctx.catch(),))?,
        Err(error) => return Err(error),
    }
    Ok(promise)
}

pub(super) fn vfs_handle(ctx: &Ctx<'_>) -> rquickjs::Result<VfsHandle> {
    ctx.userdata::<VfsUserData>()
        .map(|handle| Arc::clone(&handle.0))
        .ok_or_else(|| Exception::throw_internal(ctx, "tokamak VFS is not installed"))
}

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub(super) fn vfs_call<T>(
    ctx: &Ctx<'_>,
    operation: impl FnOnce(&mut VirtualFileSystem) -> crate::fs::vfs::Result<T>,
) -> rquickjs::Result<T> {
    let vfs = vfs_handle(ctx)?;
    match operation(&mut lock(&vfs)) {
        Ok(value) => Ok(value),
        Err(error) => Err(vfs_exception(ctx, &error)?.throw()),
    }
}

pub(super) fn vfs_exception<'js>(
    ctx: &Ctx<'js>,
    error: &VfsError,
) -> rquickjs::Result<Exception<'js>> {
    let message = error.to_string();
    let exception = Exception::from_message(ctx.clone(), &message)?;
    exception.as_object().set("code", error.code())?;
    if !error.path().is_empty() {
        exception.as_object().set("path", error.path())?;
    }
    Ok(exception)
}

pub(super) fn decode_hex(ctx: &Ctx<'_>, value: &str) -> rquickjs::Result<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return Err(Exception::throw_type(ctx, "invalid hex string"));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| Some((hex_digit(pair[0])? << 4) | hex_digit(pair[1])?))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| Exception::throw_type(ctx, "invalid hex string"))
}

pub(super) fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

pub(super) fn decode_base64(value: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut buffer = 0_u32;
    let mut bits = 0_u8;
    for character in value.bytes() {
        let Some(digit) = base64_digit(character) else {
            continue;
        };
        buffer = (buffer << 6) | u32::from(digit);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push(u8::try_from((buffer >> bits) & 0xff).unwrap_or_default());
        }
    }
    bytes
}

pub(super) fn base64_digit(value: u8) -> Option<u8> {
    match value {
        b'A'..=b'Z' => Some(value - b'A'),
        b'a'..=b'z' => Some(value - b'a' + 26),
        b'0'..=b'9' => Some(value - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

pub(super) fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        result.push(ALPHABET[(first >> 2) as usize] as char);
        result.push(ALPHABET[((first & 3) << 4 | second >> 4) as usize] as char);
        result.push(if chunk.len() > 1 {
            ALPHABET[((second & 15) << 2 | third >> 6) as usize] as char
        } else {
            '='
        });
        result.push(if chunk.len() > 2 {
            ALPHABET[(third & 63) as usize] as char
        } else {
            '='
        });
    }
    result
}
