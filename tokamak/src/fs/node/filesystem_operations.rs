#![allow(clippy::wildcard_imports)]

use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::collections::VecDeque;

/// `(input, options?)` operations answering with a value.
pub(super) type ValueOperation =
    for<'js> fn(Ctx<'js>, Value<'js>, Opt<Value<'js>>) -> rquickjs::Result<Value<'js>>;
/// `(path, options?)` operations.
pub(super) type OptionsOperation =
    for<'js> fn(Ctx<'js>, Value<'js>, Opt<Value<'js>>) -> rquickjs::Result<()>;
/// `(first, second)` operations.
pub(super) type PairOperation =
    for<'js> fn(Ctx<'js>, Value<'js>, Value<'js>) -> rquickjs::Result<()>;
/// `(first, second, options?)` operations.
pub(super) type PairOptionsOperation =
    for<'js> fn(Ctx<'js>, Value<'js>, Value<'js>, Opt<Value<'js>>) -> rquickjs::Result<()>;
/// `(path, uid, gid)` operations.
pub(super) type OwnerOperation =
    for<'js> fn(Ctx<'js>, Value<'js>, u32, u32) -> rquickjs::Result<()>;
/// `(path, atime, mtime)` operations.
pub(super) type TimesOperation =
    for<'js> fn(Ctx<'js>, Value<'js>, Value<'js>, Value<'js>) -> rquickjs::Result<()>;

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
    if value.is_undefined() || value.is_null() || value.is_bool() {
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
    let object = value
        .try_into_object()
        .map_err(|_| Exception::throw_type(ctx, "filesystem options must be a string or object"))?;
    let boolean = |name: &str, default: bool| -> rquickjs::Result<bool> {
        Ok(object.get::<_, Option<bool>>(name)?.unwrap_or(default))
    };
    Ok(FsOptions {
        encoding: object.get("encoding")?,
        recursive: boolean("recursive", false)?,
        force: boolean("force", false)?,
        error_on_exist: boolean("errorOnExist", false)?,
        dereference: boolean("dereference", false)?,
        with_file_types: boolean("withFileTypes", false)?,
        bigint: boolean("bigint", false)?,
        throw_if_no_entry: boolean("throwIfNoEntry", true)?,
        cwd: object.get("cwd")?,
        blob_type: object.get("type")?,
    })
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
    let mut path = Coerced::<String>::from_js(ctx, value)?.0;
    if !path.starts_with('/') {
        path = format!("/bundle/{path}");
    }
    if path.chars().count() > MAX_PATH_LENGTH {
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
                if parts.len() > MAX_PATH_SEGMENTS {
                    return Err(Exception::throw_range(ctx, "path has too many segments"));
                }
            }
        }
    }
    let path = format!("/{}", parts.join("/"));
    if !["/", "/bundle", "/tmp", "/dev"].into_iter().any(|root| {
        path.strip_prefix(root)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    }) {
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
    value
        .try_into_object()
        .ok()
        .and_then(|object| typed_array_bytes(&object))
        .ok_or_else(|| {
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
        Some("base64") => STANDARD.encode(bytes).into_js(&ctx),
        Some("hex") => {
            use std::fmt::Write as _;
            let mut value = String::with_capacity(bytes.len() * 2);
            for byte in bytes {
                let _ = write!(&mut value, "{byte:02x}");
            }
            value.into_js(&ctx)
        }
        Some("utf8" | "utf-8") => String::from_utf8_lossy(bytes).into_owned().into_js(&ctx),
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
    let flag = option_property(&ctx, options_value.as_ref(), "flag")?;
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
        PathOrFd::Fd(descriptor) => write_descriptor(&ctx, descriptor, &bytes, append).map(|_| ()),
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
        PathOrFd::Fd(descriptor) => write_descriptor(&ctx, descriptor, &bytes, true).map(|_| ()),
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

/// Fails unless `path` exists.
fn require_path(ctx: &Ctx<'_>, path: &str, follow_symlinks: bool) -> rquickjs::Result<()> {
    vfs_call(ctx, |vfs| {
        let stat = if follow_symlinks {
            vfs.stat(path)
        } else {
            vfs.lstat(path)
        };
        stat.map(|_| ())
    })
}

/// Fails unless `descriptor` is open.
pub(super) fn require_descriptor(ctx: &Ctx<'_>, descriptor: u32) -> rquickjs::Result<()> {
    vfs_call(ctx, |vfs| vfs.fstat(descriptor).map(|_| ()))
}

pub(super) fn chmod_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    mode: Value<'js>,
) -> rquickjs::Result<()> {
    let path = path(&ctx, input)?;
    validate_mode(&ctx, mode)?;
    require_path(&ctx, &path, true)
}

pub(super) fn chown_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    _uid: u32,
    _gid: u32,
) -> rquickjs::Result<()> {
    require_path(&ctx, &path(&ctx, input)?, true)
}

pub(super) fn fchmod_sync<'js>(
    ctx: Ctx<'js>,
    descriptor: u32,
    mode: Value<'js>,
) -> rquickjs::Result<()> {
    validate_mode(&ctx, mode)?;
    require_descriptor(&ctx, descriptor)
}

pub(super) fn fchown_sync(
    ctx: Ctx<'_>,
    descriptor: u32,
    _uid: u32,
    _gid: u32,
) -> rquickjs::Result<()> {
    require_descriptor(&ctx, descriptor)
}

/// Also serves `fdatasync`.
pub(super) fn fsync_sync(ctx: Ctx<'_>, descriptor: u32) -> rquickjs::Result<()> {
    require_descriptor(&ctx, descriptor)
}

pub(super) fn futimes_sync(
    ctx: Ctx<'_>,
    descriptor: u32,
    _atime: Value<'_>,
    _mtime: Value<'_>,
) -> rquickjs::Result<()> {
    require_descriptor(&ctx, descriptor)
}

pub(super) fn lchmod_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    mode: Value<'js>,
) -> rquickjs::Result<()> {
    let path = path(&ctx, input)?;
    validate_mode(&ctx, mode)?;
    require_path(&ctx, &path, false)
}

pub(super) fn lchown_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    _uid: u32,
    _gid: u32,
) -> rquickjs::Result<()> {
    require_path(&ctx, &path(&ctx, input)?, false)
}

pub(super) fn lutimes_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    _atime: Value<'js>,
    _mtime: Value<'js>,
) -> rquickjs::Result<()> {
    require_path(&ctx, &path(&ctx, input)?, false)
}

pub(super) fn utimes_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    _atime: Value<'js>,
    _mtime: Value<'js>,
) -> rquickjs::Result<()> {
    require_path(&ctx, &path(&ctx, input)?, true)
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
) -> rquickjs::Result<Value<'js>> {
    let path = path(&ctx, input)?;
    let options = parse_options(&ctx, options.0)?;
    let items = dir_items(&ctx, &path, options.recursive)?;
    let encoding = options.encoding.as_deref();
    let result = Array::new(ctx.clone())?;
    for (index, item) in items.into_iter().enumerate() {
        let value = if options.with_file_types {
            item_dirent(&ctx, &item, encoding)?.into_value()
        } else {
            text_value(&ctx, &item.name, encoding)?
        };
        result.set(index, value)?;
    }
    Ok(result.into_value())
}

/// The entries below `path`, named relative to it.
fn dir_items(ctx: &Ctx<'_>, path: &str, recursive: bool) -> rquickjs::Result<Vec<DirItem>> {
    if !recursive {
        let entries = vfs_call(ctx, |vfs| vfs.read_dir(path))?;
        return Ok(entries
            .into_iter()
            .map(|entry| DirItem {
                name: entry.name.clone(),
                parent: path.to_owned(),
                entry,
            })
            .collect());
    }
    let entries = vfs_call(ctx, |vfs| vfs.walk(path))?;
    Ok(entries
        .into_iter()
        .map(|(entry_path, entry)| DirItem {
            name: relative_to(&entry_path, path),
            parent: normalized_parent(parent_of(&entry_path)).to_owned(),
            entry,
        })
        .collect())
}

/// `path` relative to `base`, without a leading slash.
fn relative_to(path: &str, base: &str) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .trim_start_matches('/')
        .to_owned()
}

/// Everything before the final slash of `path`; `"/"` without one.
fn parent_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("/", |(parent, _)| parent)
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
        Err(error) if error.kind() == ErrorKind::NotFound && !options.throw_if_no_entry => {
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
    if mode > 2 {
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
    let force = option_property(&ctx, options_value.as_ref(), "force")?
        .map(|value| bool::from_js(&ctx, value))
        .transpose()?
        .unwrap_or(true);
    let options = parse_options(&ctx, options_value)?;
    vfs_call(&ctx, |vfs| {
        vfs.copy(
            &from,
            &to,
            CopyOptions {
                recursive: options.recursive,
                force,
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
) -> rquickjs::Result<Value<'js>> {
    let path = path(&ctx, input)?;
    let options = parse_options(&ctx, options.0)?;
    let entries = dir_items(&ctx, &path, options.recursive)?;
    dir_object(&ctx, path, entries, options.encoding.as_deref()).map(Object::into_value)
}

pub(super) fn readv_sync<'js>(
    ctx: Ctx<'js>,
    descriptor: u32,
    buffers: Array<'js>,
    position: Opt<Value<'js>>,
) -> rquickjs::Result<u32> {
    let position = position_value(&ctx, position.0)?;
    let writable = buffers
        .iter::<Value>()
        .map(|value| writable_bytes(&ctx, value?))
        .collect::<rquickjs::Result<Vec<_>>>()?;
    let lengths: Vec<usize> = writable.iter().map(|bytes| bytes.len).collect();
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
    let written = write_buffers(&ctx, descriptor, &buffers, position.0)?;
    u32::try_from(written).map_err(|_| Exception::throw_range(&ctx, "write is too large"))
}

/// Write `buffers` in order, returning the byte count.
pub(super) fn write_buffers<'js>(
    ctx: &Ctx<'js>,
    descriptor: u32,
    buffers: &Array<'js>,
    position: Option<Value<'js>>,
) -> rquickjs::Result<usize> {
    let position = position_value(ctx, position)?;
    let values = buffers
        .iter::<Value>()
        .map(|value| bytes(ctx, value?, None))
        .collect::<rquickjs::Result<Vec<_>>>()?;
    vfs_call(ctx, |vfs| vfs.writev(descriptor, &values, position))
}

pub(super) fn statfs_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let _path = path(&ctx, input)?;
    let options = parse_options(&ctx, options.0)?;
    statfs_object(&ctx, options.bigint).map(Object::into_value)
}

pub(super) fn open_as_blob<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let path = path(&ctx, input)?;
    let options = parse_options(&ctx, options.0)?;
    let bytes = vfs_call(&ctx, |vfs| vfs.read_file(&path))?;
    blob_object(&ctx, bytes, options.blob_type.as_deref().unwrap_or("")).map(Object::into_value)
}

pub(super) fn glob_sync<'js>(
    ctx: Ctx<'js>,
    pattern: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    glob_values(&ctx, pattern, options.0)?.into_js(&ctx)
}

pub(super) fn glob_promise<'js>(
    ctx: Ctx<'js>,
    pattern: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Object<'js>> {
    let values = glob_values(&ctx, pattern, options.0)?;
    async_value_iterator(&ctx, values)
}

/// The matches of `pattern`, as names or dirents.
fn glob_values<'js>(
    ctx: &Ctx<'js>,
    pattern: Value<'js>,
    options_value: Option<Value<'js>>,
) -> rquickjs::Result<Vec<Value<'js>>> {
    let exclude = option_property(ctx, options_value.as_ref(), "exclude")?;
    let options = parse_options(ctx, options_value)?;
    let patterns = if pattern.is_array() {
        Array::from_value(pattern)?
            .iter::<String>()
            .collect::<rquickjs::Result<Vec<_>>>()?
    } else {
        vec![Coerced::<String>::from_js(ctx, pattern)?.0]
    };
    let cwd = options.cwd.as_deref().unwrap_or("/bundle");
    let cwd = path(ctx, cwd.into_js(ctx)?)?;
    let encoding = options.encoding.as_deref();
    let mut matches: Vec<Value<'js>> = Vec::new();
    for pattern in patterns {
        for (path, entry) in glob_matches(ctx, &cwd, &pattern)? {
            let relative = glob_candidate(&path, &cwd, &pattern);
            if glob_excluded(
                ctx,
                exclude.as_ref(),
                &relative,
                &entry,
                options.with_file_types,
                &path,
            )? {
                continue;
            }
            matches.push(if options.with_file_types {
                let name = path.rsplit('/').next().unwrap_or_default();
                let name = text_value(ctx, name, encoding)?;
                dirent(ctx, &entry, name, normalized_parent(parent_of(&path)))?.into_value()
            } else {
                text_value(ctx, &relative, encoding)?
            });
        }
    }
    Ok(matches)
}

/// `path` as a glob `pattern` sees it: absolute for absolute patterns, else relative to `cwd`.
pub(super) fn glob_candidate(path: &str, cwd: &str, pattern: &str) -> String {
    if pattern.starts_with('/') {
        path.to_owned()
    } else {
        relative_to(path, cwd)
    }
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
            let name = text_value(ctx, relative.rsplit('/').next().unwrap_or_default(), None)?;
            dirent(ctx, entry, name, normalized_parent(parent_of(parent_path)))?.into_value()
        } else {
            relative.to_owned().into_js(ctx)?
        };
        return function.call::<_, bool>((value,));
    }
    let patterns = Array::from_value(exclude.clone())
        .map_err(|_| Exception::throw_type(ctx, "options.exclude must be a function or array"))?;
    for pattern in patterns.iter::<String>() {
        if glob_match(&pattern?, relative) {
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
    let target = Coerced::<String>::from_js(&ctx, target)?.0;
    let path = path(&ctx, input)?;
    vfs_call(&ctx, |vfs| vfs.symlink(&target, &path))
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
    let (offset, length, position) = match offset_or_options.0 {
        Some(value) if value.is_object() => read_options(&ctx, value)?,
        offset => (offset, length.0, position.0),
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
    let args = match offset_or_options.0 {
        Some(options) if options.is_object() => {
            let options = options
                .try_into_object()
                .map_err(|_| Exception::throw_type(&ctx, "write options must be an object"))?;
            let offset = options.get::<_, Option<Value>>("offset")?;
            let length = options.get::<_, Option<Value>>("length")?;
            let position = options.get::<_, Option<Value>>("position")?;
            let fields = if value.is_string() {
                vec![position, options.get("encoding")?]
            } else {
                vec![offset, length, position]
            };
            fields
                .into_iter()
                .map(|field| field.unwrap_or_else(|| Value::new_undefined(ctx.clone())))
                .collect()
        }
        offset => [offset, length.0, position.0]
            .into_iter()
            .flatten()
            .collect(),
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

pub(super) fn unlink_promise<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
) -> rquickjs::Result<Promise<'js>> {
    promise_unit(ctx.clone(), unlink_sync(ctx, input))
}

pub(super) fn copy_file_promise<'js>(
    ctx: Ctx<'js>,
    from: Value<'js>,
    to: Value<'js>,
    mode: Opt<u32>,
) -> rquickjs::Result<Promise<'js>> {
    promise_unit(ctx.clone(), copy_file_sync(ctx, from, to, mode))
}

pub(super) fn access_promise<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    mode: Opt<u32>,
) -> rquickjs::Result<Promise<'js>> {
    promise_unit(ctx.clone(), access_sync(ctx, input, mode))
}

pub(super) fn async_value_iterator<'js>(
    ctx: &Ctx<'js>,
    values: Vec<Value<'js>>,
) -> rquickjs::Result<Object<'js>> {
    let state = Mutex::new(VecDeque::from(values));
    let next = Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
        let value = lock(&state).pop_front();
        let done = value.is_none();
        let value = value.unwrap_or_else(|| Value::new_undefined(ctx.clone()));
        iterator_result(ctx, done, value)
    })?;
    async_iterator(ctx, next)
}

/// An async iterator object over `next`.
pub(super) fn async_iterator<'js>(
    ctx: &Ctx<'js>,
    next: Function<'js>,
) -> rquickjs::Result<Object<'js>> {
    let iterator = Object::new(ctx.clone())?;
    iterator.set("next", next)?;
    iterator.set(
        Symbol::async_iterator(ctx.clone()),
        Function::new(ctx.clone(), return_this)?,
    )?;
    Ok(iterator)
}

fn return_this<'js>(this: This<Object<'js>>) -> Object<'js> {
    this.0
}

/// A promise of the iterator result `{ done, value }`.
pub(super) fn iterator_result<'js>(
    ctx: Ctx<'js>,
    done: bool,
    value: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let result = Object::new(ctx.clone())?;
    result.set("done", done)?;
    result.set("value", value)?;
    promise(ctx, Ok(result.into_value())).map(Promise::into_value)
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

pub(super) fn truncate_promise<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    length: Opt<u64>,
) -> rquickjs::Result<Promise<'js>> {
    promise_unit(ctx.clone(), truncate_sync(ctx, input, length))
}

// Promise forms of the shared operation signatures, one host wrapper each.

pub(super) fn value_promise<'js>(
    ctx: &Ctx<'js>,
    operation: ValueOperation,
) -> rquickjs::Result<Function<'js>> {
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, input: Value<'js>, options: Opt<Value<'js>>| {
            promise(ctx.clone(), operation(ctx, input, options))
        },
    )
}

pub(super) fn options_promise<'js>(
    ctx: &Ctx<'js>,
    operation: OptionsOperation,
) -> rquickjs::Result<Function<'js>> {
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, input: Value<'js>, options: Opt<Value<'js>>| {
            promise_unit(ctx.clone(), operation(ctx, input, options))
        },
    )
}

pub(super) fn pair_promise<'js>(
    ctx: &Ctx<'js>,
    operation: PairOperation,
) -> rquickjs::Result<Function<'js>> {
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, first: Value<'js>, second: Value<'js>| {
            promise_unit(ctx.clone(), operation(ctx, first, second))
        },
    )
}

pub(super) fn pair_options_promise<'js>(
    ctx: &Ctx<'js>,
    operation: PairOptionsOperation,
) -> rquickjs::Result<Function<'js>> {
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, first: Value<'js>, second: Value<'js>, options: Opt<Value<'js>>| {
            promise_unit(ctx.clone(), operation(ctx, first, second, options))
        },
    )
}

pub(super) fn owner_promise<'js>(
    ctx: &Ctx<'js>,
    operation: OwnerOperation,
) -> rquickjs::Result<Function<'js>> {
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, input: Value<'js>, uid: u32, gid: u32| {
            promise_unit(ctx.clone(), operation(ctx, input, uid, gid))
        },
    )
}

pub(super) fn times_promise<'js>(
    ctx: &Ctx<'js>,
    operation: TimesOperation,
) -> rquickjs::Result<Function<'js>> {
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, input: Value<'js>, atime: Value<'js>, mtime: Value<'js>| {
            promise_unit(ctx.clone(), operation(ctx, input, atime, mtime))
        },
    )
}

/// A promise of `result`, resolving to `undefined`.
pub(super) fn promise_unit<'js>(
    ctx: Ctx<'js>,
    result: rquickjs::Result<()>,
) -> rquickjs::Result<Promise<'js>> {
    promise(ctx.clone(), result.map(|()| Value::new_undefined(ctx)))
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

pub(super) fn vfs_call<T>(
    ctx: &Ctx<'_>,
    operation: impl FnOnce(&mut VirtualFileSystem) -> VfsResult<T>,
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
