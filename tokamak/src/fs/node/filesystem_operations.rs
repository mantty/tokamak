#![allow(clippy::wildcard_imports)]

use super::*;

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
        blob_type: object.get("type")?,
    })
}

/// How a read checks its options before reading.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EncodingCheck {
    /// A string or `null` names the encoding, which may be `null`.
    NullAllowed,
    /// A string or `null` names the encoding, which may not be `null`.
    NullRejected,
    /// The options are an object, whose encoding may not be `null`.
    ObjectOnly,
}

/// Checks a read's `options` as workerd does before reading: they are an
/// object, or a string or `null` naming the encoding, and the encoding is
/// `"buffer"` or one `Buffer.isEncoding` accepts.
pub(super) fn check_encoding<'js>(
    ctx: &Ctx<'js>,
    options: Option<&Value<'js>>,
    check: EncodingCheck,
) -> rquickjs::Result<()> {
    let Some(options) = options.filter(|options| !options.is_undefined()) else {
        return Ok(());
    };
    let named = check != EncodingCheck::ObjectOnly && (options.is_string() || options.is_null());
    let encoding: Value = if named {
        options.clone()
    } else if options.is_object() && !options.is_array() && !options.is_function() {
        Object::from_value(options.clone())?.get("encoding")?
    } else {
        return Err(throw_type_error(
            ctx,
            "ERR_INVALID_ARG_TYPE",
            "The \"options\" argument must be of type object",
        ));
    };
    let allowed = encoding.is_undefined()
        || (encoding.is_null() && check == EncodingCheck::NullAllowed)
        || is_encoding(ctx, &encoding)?;
    if allowed {
        return Ok(());
    }
    Err(throw_type_error(
        ctx,
        "ERR_INVALID_ARG_VALUE",
        &format!(
            "The property 'options.encoding' is invalid. Received {}",
            inspected(ctx, &encoding)?
        ),
    ))
}

/// Whether `encoding` is `"buffer"` or an encoding `Buffer.isEncoding` accepts.
fn is_encoding<'js>(ctx: &Ctx<'js>, encoding: &Value<'js>) -> rquickjs::Result<bool> {
    if let Some(text) = encoding.as_string()
        && text.to_string()? == "buffer"
    {
        return Ok(true);
    }
    let class = buffer_class(ctx)?;
    let is_encoding: Function = class.get("isEncoding")?;
    is_encoding.call((This(class), encoding.clone()))
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

/// The virtual path `value` names: a string or `Buffer` resolved as a URL
/// against the working directory, or a `file:` URL, spelled as names are
/// stored.
pub(super) fn path<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<String> {
    let url = path_url(ctx, value)?;
    if url.scheme() != "file" {
        return Err(Exception::throw_type(ctx, "File path must be a file: URL"));
    }
    if url.host_str().is_some_and(|host| !host.is_empty()) {
        return Err(Exception::throw_message(
            ctx,
            "File path must not have a host",
        ));
    }
    let parts: Vec<String> = stored_path(url)
        .split('/')
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect();
    let path = format!("/{}", parts.join("/"));
    if path.chars().count() > MAX_PATH_LENGTH {
        return Err(Exception::throw_range(ctx, "path is too long"));
    }
    if parts.len() > MAX_PATH_SEGMENTS {
        return Err(Exception::throw_range(ctx, "path has too many segments"));
    }
    Ok(path)
}

/// The `file:` URL a path argument names, resolved as workerd resolves it.
fn path_url<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<Url> {
    let invalid = |_| Exception::throw_type(ctx, "Invalid URL");
    if let Some(url) = url_object(ctx, &value)? {
        return Url::parse(&url.get::<_, String>("href")?).map_err(invalid);
    }
    let text = path_text(ctx, &value)?;
    if text.contains('\0') {
        return Err(throw_type_error(
            ctx,
            "ERR_INVALID_ARG_VALUE",
            "The argument 'path' must not contain null bytes",
        ));
    }
    let text = text
        .replace('#', "%23")
        .replace('?', "%3F")
        .replace("//", "/");
    let base = if text.starts_with('/') {
        "file://".to_owned()
    } else {
        let vfs = vfs_handle(ctx)?;
        format!("file://{}/", lock(&vfs).working_directory())
    };
    Url::parse(&base)
        .and_then(|base| base.join(&text))
        .map_err(invalid)
}

/// The text of a string or `Buffer` path.
fn path_text<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> rquickjs::Result<String> {
    if let Some(text) = value.as_string() {
        return text.to_string();
    }
    if let Some(object) = value.as_object()
        && object.is_instance_of(buffer_class(ctx)?)
    {
        let bytes = typed_array_bytes(object).unwrap_or_default();
        return Ok(String::from_utf8_lossy(&bytes).into_owned());
    }
    Err(throw_type_error(
        ctx,
        "ERR_INVALID_ARG_TYPE",
        "The \"path\" argument must be of type string or an instance of Buffer or URL",
    ))
}

/// `value` when it is a `URL`.
pub(super) fn url_object<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
) -> rquickjs::Result<Option<Object<'js>>> {
    let url: Option<Value> = ctx.globals().get("URL")?;
    Ok(match (value.as_object(), url) {
        (Some(object), Some(url)) if object.is_instance_of(&url) => Some(object.clone()),
        _ => None,
    })
}

/// `url`'s path with each escape of a byte names hold unescaped decoded once,
/// then parsed again as a path.
fn stored_path(mut url: Url) -> String {
    let path = url.path().as_bytes();
    let mut decoded = String::with_capacity(path.len());
    let mut index = 0;
    while let Some(&byte) = path.get(index) {
        let escaped = escape_at(path, index);
        match escaped {
            Some(escaped) if escaped_in_names(escaped) => {
                push_escape(&mut decoded, escaped);
            }
            Some(escaped) => decoded.push(char::from(escaped)),
            None => decoded.push(char::from(byte)),
        }
        index += if escaped.is_some() { 3 } else { 1 };
    }
    url.set_path(&decoded);
    url.path().replace('^', "%5E")
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
            object.as_typed_array::<$type>().map(TypedArray::as_bytes)
        };
    }
    typed_array!(u8)
        .or_else(|| typed_array!(i8))
        .or_else(|| typed_array!(u16))
        .or_else(|| typed_array!(i16))
        .or_else(|| typed_array!(u32))
        .or_else(|| typed_array!(i32))
        .or_else(|| typed_array!(u64))
        .or_else(|| typed_array!(i64))
        .or_else(|| typed_array!(f32))
        .or_else(|| typed_array!(f64))
        .flatten()
        .map(ToOwned::to_owned)
}

/// `bytes` as a `Buffer`, or as text in `encoding`.
pub(super) fn output<'js>(
    ctx: Ctx<'js>,
    bytes: &[u8],
    encoding: Option<&str>,
) -> rquickjs::Result<Value<'js>> {
    let class = buffer_class(&ctx)?;
    let from: Function = class.get("from")?;
    let buffer: Object = from.call((This(class), ArrayBuffer::new_copy(ctx.clone(), bytes)?))?;
    match encoding {
        None => Ok(buffer.into_value()),
        Some(encoding) => {
            let to_string: Function = buffer.get("toString")?;
            to_string.call((This(buffer), encoding))
        }
    }
}

/// The `Buffer` class of `node:buffer`, loaded on first use.
pub(super) fn buffer_class<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
    const CACHE: &str = "__tokamak_node_fs_buffer";
    if let Some(class) = ctx.globals().get::<_, Option<Object>>(CACHE)? {
        return Ok(class);
    }
    let namespace = crate::runtime_modules::builtin_namespace(ctx.clone(), "node:buffer")?
        .ok_or_else(|| Exception::throw_internal(ctx, "node:buffer is not a builtin module"))?;
    let class: Object = namespace.get("Buffer")?;
    ctx.globals().set(CACHE, class.clone())?;
    Ok(class)
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
    check_encoding(&ctx, options.0.as_ref(), EncodingCheck::NullAllowed)?;
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
    check_encoding(&ctx, options.0.as_ref(), EncodingCheck::NullRejected)?;
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

/// The entries in `path`, or below it when `recursive`, each named by its full
/// path without the leading slash.
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
            name: entry_path.trim_start_matches('/').to_owned(),
            parent: normalized_parent(parent_of(&entry_path)).to_owned(),
            entry,
        })
        .collect())
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
        PathOrFd::Path(path) => stat_value(&ctx, &path, VirtualFileSystem::stat, &options),
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
    stat_value(&ctx, &path, VirtualFileSystem::lstat, &options)
}

pub(super) fn stat_value<'js>(
    ctx: &Ctx<'js>,
    path: &str,
    stat: fn(&VirtualFileSystem, &str) -> VfsResult<Stat>,
    options: &FsOptions,
) -> rquickjs::Result<Value<'js>> {
    let vfs = vfs_handle(ctx)?;
    let result = stat(&lock(&vfs), path);
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
    mode: Opt<Value<'js>>,
) -> rquickjs::Result<()> {
    let from = path(&ctx, from)?;
    let to = path(&ctx, to)?;
    check_copy_mode(&ctx, &to, mode.0)?;
    vfs_call(&ctx, |vfs| vfs.copy_file(&from, &to))
}

/// Checks a copy `mode` as workerd does before copying: it is one of 0 and
/// the `COPYFILE_*` flags, it is not `COPYFILE_FICLONE_FORCE`, and under
/// `COPYFILE_EXCL` nothing exists at `to`.
pub(super) fn check_copy_mode<'js>(
    ctx: &Ctx<'js>,
    to: &str,
    mode: Option<Value<'js>>,
) -> rquickjs::Result<()> {
    let Some(mode) = mode.filter(|mode| !mode.is_undefined()) else {
        return Ok(());
    };
    let number = mode.as_number();
    match [0, 1, 4, 2]
        .into_iter()
        .find(|allowed| number == Some(f64::from(*allowed)))
    {
        None => Err(throw_type_error(
            ctx,
            "ERR_INVALID_ARG_VALUE",
            &format!(
                "The argument 'mode' must be one of: 0, 1, 4, 2. Received {}",
                inspected(ctx, &mode)?
            ),
        )),
        Some(4) => Err(throw_error(
            ctx,
            "ERR_UNSUPPORTED_OPERATION",
            "The requested operation is unsupported",
        )),
        Some(1) => vfs_call(ctx, |vfs| match vfs.stat(to) {
            Ok(_) => Err(VfsError::new(ErrorKind::AlreadyExists, to)),
            Err(_) => Ok(()),
        }),
        Some(_) => Ok(()),
    }
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
        option_property(&ctx, options_value.as_ref(), name)?
            .map(|value| bool::from_js(&ctx, value))
            .transpose()?;
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
    check_encoding(&ctx, options.0.as_ref(), EncodingCheck::NullRejected)?;
    let prefix = path(&ctx, prefix)?;
    vfs_call(&ctx, |vfs| vfs.make_temp_dir(&prefix))?.into_js(&ctx)
}

pub(super) fn opendir_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    check_encoding(&ctx, options.0.as_ref(), EncodingCheck::ObjectOnly)?;
    let normalized = path(&ctx, input.clone())?;
    let options = parse_options(&ctx, options.0)?;
    let entries = dir_items(&ctx, &normalized, options.recursive)?;
    dir_object(&ctx, input, entries, options.encoding.as_deref()).map(Object::into_value)
}

pub(super) fn readv_sync<'js>(
    ctx: Ctx<'js>,
    descriptor: Value<'js>,
    buffers: Value<'js>,
    position: Opt<Value<'js>>,
) -> rquickjs::Result<u32> {
    let descriptor = descriptor_value(&ctx, descriptor)?;
    let read = vector_read(&ctx, &buffers, position.0)?;
    read_views(&ctx, descriptor, &read)
}

pub(super) fn writev_sync<'js>(
    ctx: Ctx<'js>,
    descriptor: Value<'js>,
    buffers: Value<'js>,
    position: Opt<Value<'js>>,
) -> rquickjs::Result<u32> {
    let descriptor = descriptor_value(&ctx, descriptor)?;
    let write = vector_write(&ctx, buffers, defined(position.0))?;
    write_views(&ctx, descriptor, &write)
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

pub(super) fn symlink_sync<'js>(
    ctx: Ctx<'js>,
    target: Value<'js>,
    input: Value<'js>,
) -> rquickjs::Result<()> {
    let link = path(&ctx, input)?;
    let target = path(&ctx, target)?;
    vfs_call(&ctx, |vfs| vfs.symlink(&target, &link))
}

pub(super) fn read_link_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    check_encoding(&ctx, options.0.as_ref(), EncodingCheck::NullRejected)?;
    let path = path(&ctx, input)?;
    let target = vfs_call(&ctx, |vfs| vfs.read_link(&path))?;
    target_value(ctx, &target, options.0)
}

pub(super) fn realpath_sync<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    check_encoding(&ctx, options.0.as_ref(), EncodingCheck::NullRejected)?;
    let path = path(&ctx, input)?;
    let target = vfs_call(&ctx, |vfs| vfs.realpath(&path))?;
    target_value(ctx, &target, options.0)
}

/// A link target or real path, decoded with the encoding of `options` as
/// `Buffer.prototype.toString` decodes it.
fn target_value<'js>(
    ctx: Ctx<'js>,
    target: &str,
    options: Option<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let options = parse_options(&ctx, options)?;
    let encoding = options.encoding.as_deref().unwrap_or("utf8");
    output(ctx, target.as_bytes(), Some(encoding))
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
    descriptor: Value<'js>,
    buffer: Value<'js>,
    args: Rest<Value<'js>>,
) -> rquickjs::Result<u32> {
    let descriptor = descriptor_value(&ctx, descriptor)?;
    let read = sync_read(&ctx, buffer, args.0)?;
    read_views(&ctx, descriptor, &read)
}

pub(super) fn write_sync<'js>(
    ctx: Ctx<'js>,
    descriptor: Value<'js>,
    value: Value<'js>,
    args: Rest<Value<'js>>,
) -> rquickjs::Result<u32> {
    let descriptor = descriptor_value(&ctx, descriptor)?;
    let write = sync_write(&ctx, value, args.0)?;
    write_views(&ctx, descriptor, &write)
}

/// Perform `write`, returning the byte count. Writing no views writes nothing.
pub(super) fn write_views<'js>(
    ctx: &Ctx<'js>,
    descriptor: u32,
    write: &WriteRequest<'js>,
) -> rquickjs::Result<u32> {
    if write.buffers.is_empty() {
        return Ok(0);
    }
    let position = file_position(ctx, write.position.clone())?;
    let written = vfs_call(ctx, |vfs| vfs.writev(descriptor, &write.buffers, position))?;
    u32::try_from(written).map_err(|_| Exception::throw_range(ctx, "write is too large"))
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
    mode: Opt<Value<'js>>,
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
    if !error.code().is_empty() {
        exception.as_object().set("code", error.code())?;
    }
    if !error.path().is_empty() {
        exception.as_object().set("path", error.path())?;
    }
    Ok(exception)
}

/// Throws an `Error` carrying the Node.js error `code`.
pub(super) fn throw_error(ctx: &Ctx<'_>, code: &str, message: &str) -> rquickjs::Error {
    let _thrown = Exception::throw_message(ctx, message);
    with_code(ctx, code)
}

/// Throws a `TypeError` carrying the Node.js error `code`.
pub(super) fn throw_type_error(ctx: &Ctx<'_>, code: &str, message: &str) -> rquickjs::Error {
    let _thrown = Exception::throw_type(ctx, message);
    with_code(ctx, code)
}

/// Throws a `RangeError` carrying the Node.js error `code`.
pub(super) fn throw_range_error(ctx: &Ctx<'_>, code: &str, message: &str) -> rquickjs::Error {
    let _thrown = Exception::throw_range(ctx, message);
    with_code(ctx, code)
}

/// A primitive as `util.inspect` shows it in Node.js error messages.
pub(super) fn inspected<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> rquickjs::Result<String> {
    if let Some(text) = value.as_string() {
        return Ok(format!("'{}'", text.to_string()?));
    }
    ctx.globals()
        .get::<_, Function>("String")?
        .call((value.clone(),))
}

/// Throws the pending exception again with `code` set on it.
fn with_code(ctx: &Ctx<'_>, code: &str) -> rquickjs::Error {
    let exception = ctx.catch();
    if let Some(object) = exception.as_object()
        && let Err(error) = object.set("code", code)
    {
        return error;
    }
    ctx.throw(exception)
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
