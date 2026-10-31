#![allow(clippy::wildcard_imports)]

use super::*;

#[derive(Clone, Copy)]
pub(super) enum CallbackOperation {
    /// `(input, options?)`, answering with a value, with the options checked
    /// before the call when it names a check.
    Value(ValueOperation, Option<EncodingCheck>),
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
    /// `(from, to, options?)`, with the arguments checked before the call,
    /// as workerd checks them.
    Cp,
    Fchmod,
    Fchown,
    Fdatasync,
    Fstat,
    Fsync,
    Ftruncate,
    Futimes,
    Open,
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
        ("chown", CallbackOperation::Owner(chown_callback)),
        ("close", CallbackOperation::Close),
        ("copyFile", CallbackOperation::CopyFile),
        ("cp", CallbackOperation::Cp),
        ("fchmod", CallbackOperation::Fchmod),
        ("fchown", CallbackOperation::Fchown),
        ("fdatasync", CallbackOperation::Fdatasync),
        ("fstat", CallbackOperation::Fstat),
        ("fsync", CallbackOperation::Fsync),
        ("ftruncate", CallbackOperation::Ftruncate),
        ("futimes", CallbackOperation::Futimes),
        ("glob", CallbackOperation::Value(glob_sync, None)),
        ("lchmod", CallbackOperation::Pair(lchmod_sync)),
        ("lchown", CallbackOperation::Owner(lchown_sync)),
        ("link", CallbackOperation::Pair(link_sync)),
        ("lstat", CallbackOperation::Value(lstat_sync, None)),
        ("lutimes", CallbackOperation::Times(lutimes_sync)),
        ("mkdir", CallbackOperation::Value(mkdir_sync, None)),
        (
            "mkdtemp",
            CallbackOperation::Value(mkdtemp_sync, Some(EncodingCheck::NullAllowed)),
        ),
        ("open", CallbackOperation::Open),
        (
            "opendir",
            CallbackOperation::Value(opendir_sync, Some(EncodingCheck::ObjectOnly)),
        ),
        (
            "readFile",
            CallbackOperation::Value(read_file_sync, Some(EncodingCheck::NullAllowed)),
        ),
        (
            "readlink",
            CallbackOperation::Value(read_link_sync, Some(EncodingCheck::NullAllowed)),
        ),
        ("readv", CallbackOperation::Readv),
        (
            "readdir",
            CallbackOperation::Value(readdir_sync, Some(EncodingCheck::NullRejected)),
        ),
        (
            "realpath",
            CallbackOperation::Value(realpath_sync, Some(EncodingCheck::NullAllowed)),
        ),
        ("rename", CallbackOperation::Pair(rename_sync)),
        ("rm", CallbackOperation::Options(rm_sync)),
        ("rmdir", CallbackOperation::Options(rmdir_sync)),
        ("stat", CallbackOperation::Value(stat_sync, None)),
        ("statfs", CallbackOperation::Value(statfs_sync, None)),
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
        CallbackOperation::Value(operation, check) => {
            let input = take_arg(&ctx, &mut args)?;
            let options = Opt(args.pop());
            if let Some(check) = check {
                check_encoding(&ctx, options.0.as_ref(), check)?;
            }
            reply(&ctx, operation(ctx.clone(), input, options))
        }
        CallbackOperation::Options(operation) => {
            let input = take_arg(&ctx, &mut args)?;
            let options = Opt(args.pop());
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
            let options = Opt(args.pop());
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
            reply(
                &ctx,
                vfs_call(&ctx, "copy", |vfs| vfs.copy_file(&from, &to)),
            )
        }
        CallbackOperation::Cp => {
            let from = take_arg(&ctx, &mut args)?;
            let to = take_arg(&ctx, &mut args)?;
            let (from, to, options) = copy_arguments(&ctx, from, to, args.pop())?;
            reply(
                &ctx,
                vfs_call(&ctx, "cp", |vfs| vfs.copy(&from, &to, options)),
            )
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
            let options = Opt(args.pop());
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
        CallbackOperation::Readv => {
            let descriptor = take_arg(&ctx, &mut args)?;
            let buffers = take_arg(&ctx, &mut args)?;
            let position = Opt(args.pop());
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
            let write = write_request(&ctx, value, args, callback_string_write)?;
            let count = write_views(&ctx, descriptor, &write);
            reply_with_buffer(&ctx, count, write.data)
        }
        CallbackOperation::Writev => {
            let descriptor = descriptor_value(&ctx, take_arg(&ctx, &mut args)?)?;
            let buffers = take_arg(&ctx, &mut args)?;
            let write = vector_write(&ctx, buffers, args.pop())?;
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

/// `fs.read(fd, ...args)`, which takes its callback from among `args` and,
/// as workerd does, passes it the error of an invalid descriptor.
pub(super) fn read_callback<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> rquickjs::Result<()> {
    let mut args = args.0;
    let descriptor =
        take_front_value(&mut args).unwrap_or_else(|| Value::new_undefined(ctx.clone()));
    let (read, callback) = callback_read(&ctx, &args)?;
    let count = if read.is_empty(&ctx)? {
        Ok(0)
    } else {
        descriptor_value(&ctx, descriptor)
            .and_then(|descriptor| read_views(&ctx, descriptor, &read.request(&ctx)?))
    };
    callback_values(&ctx, callback, reply_with_buffer(&ctx, count, read.buffer))
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

pub(super) fn take_front_value<'js>(args: &mut Vec<Value<'js>>) -> Option<Value<'js>> {
    (!args.is_empty()).then(|| args.remove(0))
}

fn take_opt_u32<'js>(ctx: &Ctx<'js>, args: &mut Vec<Value<'js>>) -> rquickjs::Result<Opt<u32>> {
    defined(args.pop())
        .map(|value| numeric_u32(ctx, value))
        .transpose()
        .map(Opt)
}

/// `value` unless it is missing, `null`, or `undefined`.
pub(super) fn defined(value: Option<Value<'_>>) -> Option<Value<'_>> {
    value.filter(|value| !value.is_null() && !value.is_undefined())
}

pub(super) fn numeric_u32<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<u32> {
    let value = Coerced::<i64>::from_js(ctx, value)?.0;
    u32::try_from(value).map_err(|_| Exception::throw_range(ctx, "value is out of range"))
}

fn take_opt_u64<'js>(ctx: &Ctx<'js>, args: &mut Vec<Value<'js>>) -> rquickjs::Result<Opt<u64>> {
    defined(args.pop())
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
    })
}

pub(super) struct WritableBytes<'js> {
    pub(super) _value: Value<'js>,
    pub(super) ptr: *mut u8,
    pub(super) len: usize,
}

/// The memory of the `ArrayBuffer` `value`.
pub(super) fn writable_bytes<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
) -> rquickjs::Result<WritableBytes<'js>> {
    let buffer = ArrayBuffer::from_value(value.clone())
        .ok_or_else(|| Exception::throw_type(ctx, "buffer must be an ArrayBuffer"))?;
    let raw = buffer
        .as_raw()
        .ok_or_else(|| Exception::throw_type(ctx, "buffer is detached"))?;
    Ok(WritableBytes {
        _value: value,
        ptr: raw.ptr.as_ptr(),
        len: raw.len,
    })
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
