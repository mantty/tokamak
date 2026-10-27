#![allow(clippy::wildcard_imports)]

use super::*;

#[derive(Clone, Copy)]
pub(super) enum CallbackOperation {
    /// `(input, options?)`, answering with a value.
    Value(ValueOperation),
    /// `(input, options?)`, answering with a value, with the options checked
    /// before the call.
    Encoded(ValueOperation, EncodingCheck),
    /// `(path, options?)`.
    Options(OptionsOperation),
    /// `(first, second)`.
    Pair(PairOperation),
    /// `(first, second, options?)`.
    PairOptions(PairOptionsOperation),
    /// `(path, uid, gid)`.
    Owner(OwnerOperation),
    /// `(path, atime, mtime)`.
    Times(TimesOperation),
    Access,
    /// `(path, data, options?)`, written as `writeFile` writes when given
    /// options, as workerd does.
    AppendFile,
    Exists,
    Close,
    CopyFile,
    Fchmod,
    Fchown,
    Fdatasync,
    Fstat,
    Fsync,
    Ftruncate,
    Futimes,
    Open,
    Read,
    Readv,
    Truncate,
    Unlink,
    Write,
    Writev,
}

pub(super) fn callback_operations() -> &'static [(&'static str, CallbackOperation)] {
    &[
        ("access", CallbackOperation::Access),
        ("exists", CallbackOperation::Exists),
        ("appendFile", CallbackOperation::AppendFile),
        ("chmod", CallbackOperation::Pair(chmod_sync)),
        ("chown", CallbackOperation::Owner(chown_sync)),
        ("close", CallbackOperation::Close),
        ("copyFile", CallbackOperation::CopyFile),
        ("cp", CallbackOperation::PairOptions(cp_sync)),
        ("fchmod", CallbackOperation::Fchmod),
        ("fchown", CallbackOperation::Fchown),
        ("fdatasync", CallbackOperation::Fdatasync),
        ("fstat", CallbackOperation::Fstat),
        ("fsync", CallbackOperation::Fsync),
        ("ftruncate", CallbackOperation::Ftruncate),
        ("futimes", CallbackOperation::Futimes),
        ("glob", CallbackOperation::Value(glob_sync)),
        ("lchmod", CallbackOperation::Pair(lchmod_sync)),
        ("lchown", CallbackOperation::Owner(lchown_sync)),
        ("link", CallbackOperation::Pair(link_sync)),
        ("lstat", CallbackOperation::Value(lstat_sync)),
        ("lutimes", CallbackOperation::Times(lutimes_sync)),
        ("mkdir", CallbackOperation::Options(mkdir_sync)),
        (
            "mkdtemp",
            CallbackOperation::Encoded(mkdtemp_sync, EncodingCheck::NullAllowed),
        ),
        ("open", CallbackOperation::Open),
        (
            "opendir",
            CallbackOperation::Encoded(opendir_sync, EncodingCheck::ObjectOnly),
        ),
        ("read", CallbackOperation::Read),
        (
            "readFile",
            CallbackOperation::Encoded(read_file_sync, EncodingCheck::NullAllowed),
        ),
        (
            "readlink",
            CallbackOperation::Encoded(read_link_sync, EncodingCheck::NullAllowed),
        ),
        ("readv", CallbackOperation::Readv),
        (
            "readdir",
            CallbackOperation::Encoded(readdir_sync, EncodingCheck::NullRejected),
        ),
        (
            "realpath",
            CallbackOperation::Encoded(realpath_sync, EncodingCheck::NullAllowed),
        ),
        ("rename", CallbackOperation::Pair(rename_sync)),
        ("rm", CallbackOperation::Options(rm_sync)),
        ("rmdir", CallbackOperation::Options(rmdir_sync)),
        ("stat", CallbackOperation::Value(stat_sync)),
        ("statfs", CallbackOperation::Value(statfs_sync)),
        ("symlink", CallbackOperation::Pair(symlink_sync)),
        ("truncate", CallbackOperation::Truncate),
        ("unlink", CallbackOperation::Unlink),
        ("utimes", CallbackOperation::Times(utimes_sync)),
        ("write", CallbackOperation::Write),
        ("writeFile", CallbackOperation::PairOptions(write_file_sync)),
        ("writev", CallbackOperation::Writev),
    ]
}

#[allow(clippy::too_many_lines)]
pub(super) fn callback_call<'js>(
    ctx: Ctx<'js>,
    operation: CallbackOperation,
    args: Rest<Value<'js>>,
) -> rquickjs::Result<()> {
    let mut args = args.0;
    let without_callback = matches!(
        operation,
        CallbackOperation::Close | CallbackOperation::Fsync
    ) && args.len() == 1;
    if without_callback {
        let descriptor = descriptor_value(&ctx, take_arg(&ctx, &mut args)?)?;
        return match operation {
            CallbackOperation::Close => close_sync(ctx, descriptor),
            _ => fsync_sync(ctx, descriptor),
        };
    }
    let callback = args
        .pop()
        .and_then(|callback| Function::from_value(callback).ok())
        .ok_or_else(|| {
            throw_type_error(
                &ctx,
                "ERR_INVALID_ARG_TYPE",
                "The \"callback\" argument must be of type function",
            )
        })?;
    let result = match operation {
        CallbackOperation::Value(operation) => {
            let input = take_arg(&ctx, &mut args)?;
            let options = take_opt_value(&mut args);
            reply(&ctx, operation(ctx.clone(), input, options))
        }
        CallbackOperation::Encoded(operation, check) => {
            let input = take_arg(&ctx, &mut args)?;
            let options = take_opt_value(&mut args);
            check_encoding(&ctx, options.0.as_ref(), check)?;
            reply(&ctx, operation(ctx.clone(), input, options))
        }
        CallbackOperation::Options(operation) => {
            let input = take_arg(&ctx, &mut args)?;
            let options = take_opt_value(&mut args);
            reply(&ctx, operation(ctx.clone(), input, options))
        }
        CallbackOperation::Pair(operation) => {
            let first = take_arg(&ctx, &mut args)?;
            let second = take_arg(&ctx, &mut args)?;
            reply(&ctx, operation(ctx.clone(), first, second))
        }
        CallbackOperation::PairOptions(operation) => {
            let first = take_arg(&ctx, &mut args)?;
            let second = take_arg(&ctx, &mut args)?;
            let options = take_opt_value(&mut args);
            reply(&ctx, operation(ctx.clone(), first, second, options))
        }
        CallbackOperation::Owner(operation) => {
            let input = take_arg(&ctx, &mut args)?;
            let uid = numeric_u32(&ctx, take_arg(&ctx, &mut args)?)?;
            let gid = numeric_u32(&ctx, take_arg(&ctx, &mut args)?)?;
            reply(&ctx, operation(ctx.clone(), input, uid, gid))
        }
        CallbackOperation::Times(operation) => {
            let input = take_arg(&ctx, &mut args)?;
            let atime = take_arg(&ctx, &mut args)?;
            let mtime = take_arg(&ctx, &mut args)?;
            reply(&ctx, operation(ctx.clone(), input, atime, mtime))
        }
        CallbackOperation::Access => {
            let input = take_arg(&ctx, &mut args)?;
            let mode = take_opt_u32(&ctx, &mut args)?;
            reply(&ctx, access_sync(ctx.clone(), input, mode))
        }
        CallbackOperation::AppendFile => {
            let input = take_arg(&ctx, &mut args)?;
            let data = take_arg(&ctx, &mut args)?;
            let write: PairOptionsOperation = if args.is_empty() {
                append_file_sync
            } else {
                write_file_sync
            };
            reply(&ctx, write(ctx.clone(), input, data, Opt(args.pop())))
        }
        CallbackOperation::Exists => {
            let input = take_arg(&ctx, &mut args)?;
            Ok(vec![exists_sync(ctx.clone(), input)?.into_js(&ctx)?])
        }
        CallbackOperation::Close => {
            let descriptor = descriptor_value(&ctx, take_arg(&ctx, &mut args)?)?;
            reply(&ctx, close_sync(ctx.clone(), descriptor))
        }
        CallbackOperation::CopyFile => {
            let from = path(&ctx, take_arg(&ctx, &mut args)?)?;
            let to = path(&ctx, take_arg(&ctx, &mut args)?)?;
            check_copy_mode(&ctx, &to, take_front_value(&mut args))?;
            reply(&ctx, vfs_call(&ctx, |vfs| vfs.copy_file(&from, &to)))
        }
        CallbackOperation::Fchmod => {
            let descriptor = descriptor_value(&ctx, take_arg(&ctx, &mut args)?)?;
            let mode = take_arg(&ctx, &mut args)?;
            reply(&ctx, fchmod_sync(ctx.clone(), descriptor, mode))
        }
        CallbackOperation::Fchown => {
            let descriptor = descriptor_value(&ctx, take_arg(&ctx, &mut args)?)?;
            let uid = numeric_u32(&ctx, take_arg(&ctx, &mut args)?)?;
            let gid = numeric_u32(&ctx, take_arg(&ctx, &mut args)?)?;
            reply(&ctx, fchown_sync(ctx.clone(), descriptor, uid, gid))
        }
        CallbackOperation::Fstat => {
            let descriptor = descriptor_value(&ctx, take_arg(&ctx, &mut args)?)?;
            let options = take_opt_value(&mut args);
            reply(&ctx, fstat_sync(ctx.clone(), descriptor, options))
        }
        CallbackOperation::Fdatasync | CallbackOperation::Fsync => {
            let descriptor = descriptor_value(&ctx, take_arg(&ctx, &mut args)?)?;
            reply(&ctx, fsync_sync(ctx.clone(), descriptor))
        }
        CallbackOperation::Ftruncate => {
            let descriptor = descriptor_value(&ctx, take_arg(&ctx, &mut args)?)?;
            let length = take_opt_u64(&ctx, &mut args)?;
            reply(&ctx, ftruncate_sync(ctx.clone(), descriptor, length))
        }
        CallbackOperation::Futimes => {
            let descriptor = descriptor_value(&ctx, take_arg(&ctx, &mut args)?)?;
            let atime = take_arg(&ctx, &mut args)?;
            let mtime = take_arg(&ctx, &mut args)?;
            reply(&ctx, futimes_sync(ctx.clone(), descriptor, atime, mtime))
        }
        CallbackOperation::Open => {
            let input = take_arg(&ctx, &mut args)?;
            let flags = take_front_value(&mut args);
            let mode = take_front_value(&mut args);
            reply(&ctx, open_sync(ctx.clone(), input, Opt(flags), Opt(mode)))
        }
        CallbackOperation::Read => {
            let descriptor = descriptor_value(&ctx, take_arg(&ctx, &mut args)?)?;
            let arguments = normalize_read_arguments(&ctx, args)?;
            let buffer = arguments.buffer.clone();
            let read = node_read(&ctx, arguments)?;
            reply_with_buffer(&ctx, read_views(&ctx, descriptor, &read), buffer)
        }
        CallbackOperation::Readv => {
            let descriptor = take_arg(&ctx, &mut args)?;
            let buffers = take_arg(&ctx, &mut args)?;
            let position = take_opt_value(&mut args);
            optional_position(&ctx, position.0.clone())?;
            let count = readv_sync(ctx.clone(), descriptor, buffers.clone(), position);
            reply_with_buffer(&ctx, count, buffers)
        }
        CallbackOperation::Truncate => {
            let input = take_arg(&ctx, &mut args)?;
            let length = take_opt_u64(&ctx, &mut args)?;
            reply(&ctx, truncate_sync(ctx.clone(), input, length))
        }
        CallbackOperation::Unlink => {
            let input = take_arg(&ctx, &mut args)?;
            reply(&ctx, unlink_sync(ctx.clone(), input))
        }
        CallbackOperation::Write => {
            let descriptor = descriptor_value(&ctx, take_arg(&ctx, &mut args)?)?;
            let value = take_arg(&ctx, &mut args)?;
            let write = callback_write(&ctx, value, args)?;
            let count = write_views(&ctx, descriptor, &write);
            reply_with_buffer(&ctx, count, write.data)
        }
        CallbackOperation::Writev => {
            let descriptor = descriptor_value(&ctx, take_arg(&ctx, &mut args)?)?;
            let buffers = take_arg(&ctx, &mut args)?;
            let write = vector_write(&ctx, buffers, take_opt_value(&mut args).0)?;
            let count = write_views(&ctx, descriptor, &write);
            reply_with_buffer(&ctx, count, write.data)
        }
    };
    callback_values(&ctx, callback, result)
}

/// Callback arguments for an operation result: `(null, value)`.
pub(super) fn reply<'js>(
    ctx: &Ctx<'js>,
    result: rquickjs::Result<impl IntoJs<'js>>,
) -> rquickjs::Result<Vec<Value<'js>>> {
    Ok(vec![Value::new_null(ctx.clone()), result?.into_js(ctx)?])
}

/// Callback arguments for a buffered transfer: `(null, count, buffer)`.
fn reply_with_buffer<'js>(
    ctx: &Ctx<'js>,
    count: rquickjs::Result<u32>,
    buffer: Value<'js>,
) -> rquickjs::Result<Vec<Value<'js>>> {
    Ok(vec![
        Value::new_null(ctx.clone()),
        count?.into_js(ctx)?,
        buffer,
    ])
}

pub(super) fn take_arg<'js>(
    ctx: &Ctx<'js>,
    args: &mut Vec<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    if args.is_empty() {
        return Err(Exception::throw_type(ctx, "missing filesystem argument"));
    }
    Ok(args.remove(0))
}

pub(super) fn take_opt_value<'js>(args: &mut Vec<Value<'js>>) -> Opt<Value<'js>> {
    Opt(args.pop())
}

pub(super) fn take_front_value<'js>(args: &mut Vec<Value<'js>>) -> Option<Value<'js>> {
    (!args.is_empty()).then(|| args.remove(0))
}

pub(super) fn take_opt_u32<'js>(
    ctx: &Ctx<'js>,
    args: &mut Vec<Value<'js>>,
) -> rquickjs::Result<Opt<u32>> {
    optional_u32(ctx, args.pop())
}

pub(super) struct ReadArguments<'js> {
    pub(super) buffer: Value<'js>,
    pub(super) offset: Option<Value<'js>>,
    pub(super) length: Option<Value<'js>>,
    pub(super) position: Option<Value<'js>>,
}

pub(super) fn normalize_read_arguments<'js>(
    ctx: &Ctx<'js>,
    mut args: Vec<Value<'js>>,
) -> rquickjs::Result<ReadArguments<'js>> {
    let Some(value) = args.first().cloned() else {
        return Ok(ReadArguments {
            buffer: default_read_buffer(ctx, None)?,
            offset: None,
            length: None,
            position: None,
        });
    };
    if is_array_buffer_view(&value) {
        args.remove(0);
        if args
            .first()
            .is_some_and(|value| value.is_object() && !is_array_buffer_view(value))
        {
            let (offset, length, position) = read_options(ctx, args.remove(0))?;
            return Ok(ReadArguments {
                buffer: value,
                offset,
                length,
                position,
            });
        }
        Ok(ReadArguments {
            buffer: value,
            offset: take_front_value(&mut args),
            length: take_front_value(&mut args),
            position: take_front_value(&mut args),
        })
    } else if value.is_object() {
        let options = value
            .try_into_object()
            .map_err(|_| Exception::throw_type(ctx, "read options must be an object"))?;
        let length = options.get::<_, Option<Value>>("length")?;
        let buffer = options
            .get::<_, Option<Value>>("buffer")?
            .map_or_else(|| default_read_buffer(ctx, length.as_ref()), Ok)?;
        Ok(ReadArguments {
            buffer,
            offset: options.get("offset")?,
            length,
            position: options.get("position")?,
        })
    } else {
        Err(Exception::throw_type(
            ctx,
            "buffer or read options are required",
        ))
    }
}

/// The `offset`, `length`, and `position` of a read options object.
pub(super) fn read_options<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
) -> rquickjs::Result<(Option<Value<'js>>, Option<Value<'js>>, Option<Value<'js>>)> {
    let options = value
        .try_into_object()
        .map_err(|_| Exception::throw_type(ctx, "read options must be an object"))?;
    Ok((
        options.get("offset")?,
        options.get("length")?,
        options.get("position")?,
    ))
}

pub(super) fn default_read_buffer<'js>(
    ctx: &Ctx<'js>,
    length: Option<&Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let length = length
        .map(|value| Coerced::<u64>::from_js(ctx, value.clone()))
        .transpose()?
        .map_or(16_384, |value| {
            usize::try_from(*value).unwrap_or(usize::MAX)
        });
    if u32::try_from(length).is_err() {
        return Err(Exception::throw_range(ctx, "read buffer is too large"));
    }
    TypedArray::<u8>::new(ctx.clone(), vec![0; length]).map(TypedArray::into_value)
}

/// `value` unless it is missing, `null`, or `undefined`.
pub(super) fn defined(value: Option<Value<'_>>) -> Option<Value<'_>> {
    value.filter(|value| !value.is_null() && !value.is_undefined())
}

pub(super) fn optional_u32<'js>(
    ctx: &Ctx<'js>,
    value: Option<Value<'js>>,
) -> rquickjs::Result<Opt<u32>> {
    defined(value)
        .map(|value| numeric_u32(ctx, value))
        .transpose()
        .map(Opt)
}

pub(super) fn numeric_u32<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<u32> {
    let value = Coerced::<i64>::from_js(ctx, value)?.0;
    u32::try_from(value).map_err(|_| Exception::throw_range(ctx, "value is out of range"))
}

pub(super) fn take_opt_u64<'js>(
    ctx: &Ctx<'js>,
    args: &mut Vec<Value<'js>>,
) -> rquickjs::Result<Opt<u64>> {
    optional_u64(ctx, args.pop())
}

pub(super) fn optional_u64<'js>(
    ctx: &Ctx<'js>,
    value: Option<Value<'js>>,
) -> rquickjs::Result<Opt<u64>> {
    defined(value)
        .map(|value| Coerced::<u64>::from_js(ctx, value).map(|value| value.0))
        .transpose()
        .map(Opt)
}

pub(super) fn callback_values<'js>(
    ctx: &Ctx<'js>,
    callback: Function<'js>,
    result: rquickjs::Result<Vec<Value<'js>>>,
) -> rquickjs::Result<()> {
    let values = match result {
        Ok(values) => values,
        Err(_error) if ctx.has_exception() => vec![ctx.catch()],
        Err(error) => return Err(error),
    };
    let task = Function::new(ctx.clone(), move |_: Ctx<'js>| {
        callback.call::<_, ()>((Rest(values.clone()),))
    })?;
    if let Some(queue_microtask) = ctx.globals().get::<_, Option<Function>>("queueMicrotask")? {
        queue_microtask.call::<_, ()>((task,))
    } else {
        task.call::<_, ()>(())
    }
}

pub(super) fn flag_string<'js>(value: Option<&Value<'js>>) -> rquickjs::Result<Option<String>> {
    value
        .map(|value| {
            value
                .as_string()
                .ok_or_else(|| rquickjs::Error::new_from_js("value", "string"))?
                .to_string()
        })
        .transpose()
}

pub(super) fn open_options<'js>(
    ctx: &Ctx<'js>,
    value: Option<Value<'js>>,
) -> rquickjs::Result<OpenOptions> {
    let Some(value) = value else {
        return Ok(OpenOptions::default());
    };
    if value.is_number() {
        let flags = *Coerced::<i32>::from_js(ctx, value)?;
        let access = flags & 3;
        return Ok(OpenOptions {
            read: access != 1,
            write: access != 0,
            append: flags & 1024 != 0,
            create: flags & 64 != 0,
            exclusive: flags & 128 != 0,
            truncate: flags & 512 != 0,
            ..OpenOptions::default()
        });
    }
    let flags = Coerced::<String>::from_js(ctx, value)?.0;
    let value = flags.as_str();
    let first = value.chars().next().unwrap_or('r');
    if !matches!(first, 'r' | 'w' | 'a')
        || value
            .chars()
            .skip(1)
            .any(|character| !matches!(character, '+' | 'x' | 's'))
    {
        return Err(Exception::throw_type(ctx, "invalid file open flags"));
    }
    let plus = value.contains('+');
    Ok(OpenOptions {
        read: first == 'r' || plus,
        write: first != 'r' || plus,
        append: first == 'a',
        create: matches!(first, 'w' | 'a') || value.contains('x'),
        exclusive: value.contains('x'),
        truncate: first == 'w',
        ..OpenOptions::default()
    })
}

pub(super) struct WritableBytes<'js> {
    pub(super) _value: Value<'js>,
    pub(super) ptr: *mut u8,
    pub(super) len: usize,
}

pub(super) fn writable_bytes<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
) -> rquickjs::Result<WritableBytes<'js>> {
    if let Some(buffer) = ArrayBuffer::from_value(value.clone()) {
        let raw = buffer
            .as_raw()
            .ok_or_else(|| Exception::throw_type(ctx, "buffer is detached"))?;
        return Ok(WritableBytes {
            _value: value,
            ptr: raw.ptr.as_ptr(),
            len: raw.len,
        });
    }
    let object = value
        .clone()
        .try_into_object()
        .map_err(|_| Exception::throw_type(ctx, "buffer must be an ArrayBuffer or typed array"))?;
    macro_rules! typed_array {
        ($type:ty) => {
            if object.is_typed_array::<$type>() {
                let array = TypedArray::<$type>::from_object(object.clone())?;
                let raw = array
                    .as_raw()
                    .ok_or_else(|| Exception::throw_type(ctx, "buffer is detached"))?;
                return Ok(WritableBytes {
                    _value: value,
                    ptr: raw.ptr.as_ptr(),
                    len: raw.len,
                });
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
    Err(Exception::throw_type(
        ctx,
        "buffer must be an ArrayBuffer or typed array",
    ))
}

impl WritableBytes<'_> {
    pub(super) fn write(&self, offset: usize, bytes: &[u8]) {
        // QuickJS keeps the typed array alive for the duration of this native
        // call, so the raw view remains valid while we copy into it.
        unsafe {
            std::slice::from_raw_parts_mut(self.ptr.add(offset), bytes.len())
                .copy_from_slice(bytes);
        }
    }
}
