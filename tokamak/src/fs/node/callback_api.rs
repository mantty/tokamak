#![allow(clippy::wildcard_imports)]

use super::*;

#[derive(Clone, Copy)]
pub(super) enum CallbackOperation {
    Access,
    Exists,
    AppendFile,
    Chmod,
    Chown,
    Close,
    CopyFile,
    Cp,
    Fchmod,
    Fchown,
    Fdatasync,
    Fstat,
    Fsync,
    Ftruncate,
    Futimes,
    Glob,
    Lchmod,
    Lchown,
    Link,
    Lstat,
    Lutimes,
    Mkdir,
    Mkdtemp,
    Open,
    Opendir,
    Read,
    ReadFile,
    Readlink,
    Readv,
    Readdir,
    Realpath,
    Rename,
    Rm,
    Rmdir,
    Stat,
    Statfs,
    Symlink,
    Truncate,
    Unlink,
    Utimes,
    Write,
    WriteFile,
    Writev,
}

pub(super) fn callback_operations() -> &'static [(&'static str, CallbackOperation)] {
    &[
        ("access", CallbackOperation::Access),
        ("exists", CallbackOperation::Exists),
        ("appendFile", CallbackOperation::AppendFile),
        ("chmod", CallbackOperation::Chmod),
        ("chown", CallbackOperation::Chown),
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
        ("glob", CallbackOperation::Glob),
        ("lchmod", CallbackOperation::Lchmod),
        ("lchown", CallbackOperation::Lchown),
        ("link", CallbackOperation::Link),
        ("lstat", CallbackOperation::Lstat),
        ("lutimes", CallbackOperation::Lutimes),
        ("mkdir", CallbackOperation::Mkdir),
        ("mkdtemp", CallbackOperation::Mkdtemp),
        ("open", CallbackOperation::Open),
        ("opendir", CallbackOperation::Opendir),
        ("read", CallbackOperation::Read),
        ("readFile", CallbackOperation::ReadFile),
        ("readlink", CallbackOperation::Readlink),
        ("readv", CallbackOperation::Readv),
        ("readdir", CallbackOperation::Readdir),
        ("realpath", CallbackOperation::Realpath),
        ("rename", CallbackOperation::Rename),
        ("rm", CallbackOperation::Rm),
        ("rmdir", CallbackOperation::Rmdir),
        ("stat", CallbackOperation::Stat),
        ("statfs", CallbackOperation::Statfs),
        ("symlink", CallbackOperation::Symlink),
        ("truncate", CallbackOperation::Truncate),
        ("unlink", CallbackOperation::Unlink),
        ("utimes", CallbackOperation::Utimes),
        ("write", CallbackOperation::Write),
        ("writeFile", CallbackOperation::WriteFile),
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
        .ok_or_else(|| Exception::throw_type(&ctx, "callback is required"))?;
    let callback = Function::from_value(callback)
        .map_err(|_| Exception::throw_type(&ctx, "callback must be a function"))?;
    let result = match operation {
        CallbackOperation::Access => {
            let input = take_arg(&ctx, &mut args)?;
            let mode = take_opt_u32(&ctx, &mut args)?;
            reply(&ctx, access_sync(ctx.clone(), input, mode))
        }
        CallbackOperation::Exists => {
            let input = take_arg(&ctx, &mut args)?;
            Ok(vec![exists_sync(ctx.clone(), input)?.into_js(&ctx)?])
        }
        CallbackOperation::AppendFile => {
            let input = take_arg(&ctx, &mut args)?;
            let data = take_arg(&ctx, &mut args)?;
            let options = take_opt_value(&mut args);
            reply(&ctx, append_file_sync(ctx.clone(), input, data, options))
        }
        CallbackOperation::Chmod => {
            let input = take_arg(&ctx, &mut args)?;
            let mode = take_arg(&ctx, &mut args)?;
            reply(&ctx, chmod_sync(ctx.clone(), input, mode))
        }
        CallbackOperation::Chown => {
            let input = take_arg(&ctx, &mut args)?;
            let uid = numeric_u32(&ctx, take_arg(&ctx, &mut args)?)?;
            let gid = numeric_u32(&ctx, take_arg(&ctx, &mut args)?)?;
            reply(&ctx, chown_sync(ctx.clone(), input, uid, gid))
        }
        CallbackOperation::Close => {
            let descriptor = descriptor_value(&ctx, take_arg(&ctx, &mut args)?)?;
            reply(&ctx, close_sync(ctx.clone(), descriptor))
        }
        CallbackOperation::CopyFile => {
            let from = take_arg(&ctx, &mut args)?;
            let to = take_arg(&ctx, &mut args)?;
            let mode = take_opt_u32(&ctx, &mut args)?;
            reply(&ctx, copy_file_sync(ctx.clone(), from, to, mode))
        }
        CallbackOperation::Cp => {
            let from = take_arg(&ctx, &mut args)?;
            let to = take_arg(&ctx, &mut args)?;
            let options = take_opt_value(&mut args);
            reply(&ctx, cp_sync(ctx.clone(), from, to, options))
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
        CallbackOperation::Fdatasync => {
            let descriptor = descriptor_value(&ctx, take_arg(&ctx, &mut args)?)?;
            reply(&ctx, fdatasync_sync(ctx.clone(), descriptor))
        }
        CallbackOperation::Fstat => {
            let descriptor = descriptor_value(&ctx, take_arg(&ctx, &mut args)?)?;
            let options = take_opt_value(&mut args);
            reply(&ctx, fstat_sync(ctx.clone(), descriptor, options))
        }
        CallbackOperation::Fsync => {
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
        CallbackOperation::Glob => {
            let pattern = take_arg(&ctx, &mut args)?;
            let options = take_opt_value(&mut args);
            reply(&ctx, glob_sync(ctx.clone(), pattern, options))
        }
        CallbackOperation::Lchmod => {
            let input = take_arg(&ctx, &mut args)?;
            let mode = take_arg(&ctx, &mut args)?;
            reply(&ctx, lchmod_sync(ctx.clone(), input, mode))
        }
        CallbackOperation::Lchown => {
            let input = take_arg(&ctx, &mut args)?;
            let uid = numeric_u32(&ctx, take_arg(&ctx, &mut args)?)?;
            let gid = numeric_u32(&ctx, take_arg(&ctx, &mut args)?)?;
            reply(&ctx, lchown_sync(ctx.clone(), input, uid, gid))
        }
        CallbackOperation::Link => {
            let existing = take_arg(&ctx, &mut args)?;
            let new = take_arg(&ctx, &mut args)?;
            reply(&ctx, link_sync(ctx.clone(), existing, new))
        }
        CallbackOperation::Lstat => {
            let input = take_arg(&ctx, &mut args)?;
            let options = take_opt_value(&mut args);
            reply(&ctx, lstat_sync(ctx.clone(), input, options))
        }
        CallbackOperation::Lutimes => {
            let input = take_arg(&ctx, &mut args)?;
            let atime = take_arg(&ctx, &mut args)?;
            let mtime = take_arg(&ctx, &mut args)?;
            reply(&ctx, lutimes_sync(ctx.clone(), input, atime, mtime))
        }
        CallbackOperation::Mkdir => {
            let input = take_arg(&ctx, &mut args)?;
            let options = take_opt_value(&mut args);
            reply(&ctx, mkdir_sync(ctx.clone(), input, options))
        }
        CallbackOperation::Mkdtemp => {
            let input = take_arg(&ctx, &mut args)?;
            let options = take_opt_value(&mut args);
            reply(&ctx, mkdtemp_sync(ctx.clone(), input, options))
        }
        CallbackOperation::Open => {
            let input = take_arg(&ctx, &mut args)?;
            let flags = take_front_value(&mut args);
            let mode = take_front_value(&mut args);
            reply(&ctx, open_sync(ctx.clone(), input, Opt(flags), Opt(mode)))
        }
        CallbackOperation::Opendir => {
            let input = take_arg(&ctx, &mut args)?;
            let options = take_opt_value(&mut args);
            reply(&ctx, opendir_sync(ctx.clone(), input, options))
        }
        CallbackOperation::Read => {
            let descriptor = descriptor_value(&ctx, take_arg(&ctx, &mut args)?)?;
            let ReadArguments {
                buffer,
                offset,
                length,
                position,
            } = normalize_read_arguments(&ctx, args)?;
            let count = read_sync(
                ctx.clone(),
                descriptor,
                buffer.clone(),
                optional_u32(&ctx, offset)?,
                optional_u32(&ctx, length)?,
                Opt(position),
            );
            reply_with_buffer(&ctx, count, buffer)
        }
        CallbackOperation::ReadFile => {
            let input = take_arg(&ctx, &mut args)?;
            let options = take_opt_value(&mut args);
            reply(&ctx, read_file_sync(ctx.clone(), input, options))
        }
        CallbackOperation::Readlink => {
            let input = take_arg(&ctx, &mut args)?;
            let options = take_opt_value(&mut args);
            reply(&ctx, read_link_sync(ctx.clone(), input, options))
        }
        CallbackOperation::Readv => {
            let descriptor = descriptor_value(&ctx, take_arg(&ctx, &mut args)?)?;
            let buffers = take_arg(&ctx, &mut args)?;
            let array = Array::from_value(buffers.clone())?;
            let position = take_opt_value(&mut args);
            let count = readv_sync(ctx.clone(), descriptor, array, position);
            reply_with_buffer(&ctx, count, buffers)
        }
        CallbackOperation::Readdir => {
            let input = take_arg(&ctx, &mut args)?;
            let options = take_opt_value(&mut args);
            reply(&ctx, readdir_sync(ctx.clone(), input, options))
        }
        CallbackOperation::Realpath => {
            let input = take_arg(&ctx, &mut args)?;
            let options = take_opt_value(&mut args);
            reply(&ctx, realpath_sync(ctx.clone(), input, options))
        }
        CallbackOperation::Rename => {
            let from = take_arg(&ctx, &mut args)?;
            let to = take_arg(&ctx, &mut args)?;
            reply(&ctx, rename_sync(ctx.clone(), from, to))
        }
        CallbackOperation::Rm => {
            let input = take_arg(&ctx, &mut args)?;
            let options = take_opt_value(&mut args);
            reply(&ctx, rm_sync(ctx.clone(), input, options))
        }
        CallbackOperation::Rmdir => {
            let input = take_arg(&ctx, &mut args)?;
            let options = take_opt_value(&mut args);
            reply(&ctx, rmdir_sync(ctx.clone(), input, options))
        }
        CallbackOperation::Stat => {
            let input = take_arg(&ctx, &mut args)?;
            let options = take_opt_value(&mut args);
            reply(&ctx, stat_sync(ctx.clone(), input, options))
        }
        CallbackOperation::Statfs => {
            let input = take_arg(&ctx, &mut args)?;
            let options = take_opt_value(&mut args);
            reply(&ctx, statfs_sync(ctx.clone(), input, options))
        }
        CallbackOperation::Symlink => {
            let target = take_arg(&ctx, &mut args)?;
            let input = take_arg(&ctx, &mut args)?;
            reply(&ctx, symlink_sync(ctx.clone(), target, input))
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
        CallbackOperation::Utimes => {
            let input = take_arg(&ctx, &mut args)?;
            let atime = take_arg(&ctx, &mut args)?;
            let mtime = take_arg(&ctx, &mut args)?;
            reply(&ctx, utimes_sync(ctx.clone(), input, atime, mtime))
        }
        CallbackOperation::Write => {
            let descriptor = descriptor_value(&ctx, take_arg(&ctx, &mut args)?)?;
            let value = take_arg(&ctx, &mut args)?;
            let count = write_sync(ctx.clone(), descriptor, value.clone(), Rest(args));
            reply_with_buffer(&ctx, count, value)
        }
        CallbackOperation::WriteFile => {
            let input = take_arg(&ctx, &mut args)?;
            let data = take_arg(&ctx, &mut args)?;
            let options = take_opt_value(&mut args);
            reply(&ctx, write_file_sync(ctx.clone(), input, data, options))
        }
        CallbackOperation::Writev => {
            let descriptor = descriptor_value(&ctx, take_arg(&ctx, &mut args)?)?;
            let buffers = take_arg(&ctx, &mut args)?;
            let array = Array::from_value(buffers.clone())?;
            let position = take_opt_value(&mut args);
            let count = writev_sync(ctx.clone(), descriptor, array, position);
            reply_with_buffer(&ctx, count, buffers)
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
    let first = args.first().cloned();
    match first {
        Some(value) if is_buffer_value(&value) => {
            args.remove(0);
            if args
                .first()
                .is_some_and(|value| value.is_object() && !is_buffer_value(value))
            {
                let options = args
                    .remove(0)
                    .try_into_object()
                    .map_err(|_| Exception::throw_type(ctx, "read options must be an object"))?;
                return Ok(ReadArguments {
                    buffer: value,
                    offset: options.get("offset")?,
                    length: options.get("length")?,
                    position: options.get("position")?,
                });
            }
            Ok(ReadArguments {
                buffer: value,
                offset: take_front_value(&mut args),
                length: take_front_value(&mut args),
                position: take_front_value(&mut args),
            })
        }
        Some(value) if value.is_object() => {
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
        }
        None => Ok(ReadArguments {
            buffer: default_read_buffer(ctx, None)?,
            offset: None,
            length: None,
            position: None,
        }),
        Some(_) => Err(Exception::throw_type(
            ctx,
            "buffer or read options are required",
        )),
    }
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

pub(super) fn optional_u32<'js>(
    ctx: &Ctx<'js>,
    value: Option<Value<'js>>,
) -> rquickjs::Result<Opt<u32>> {
    if value
        .as_ref()
        .is_some_and(|value| value.is_null() || value.is_undefined())
    {
        return Ok(Opt(None));
    }
    value
        .map(|value| {
            let value = Coerced::<i64>::from_js(ctx, value)?.0;
            u32::try_from(value).map_err(|_| Exception::throw_range(ctx, "value is out of range"))
        })
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
    if value
        .as_ref()
        .is_some_and(|value| value.is_null() || value.is_undefined())
    {
        return Ok(Opt(None));
    }
    value
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

pub(super) fn position_value<'js>(
    ctx: &Ctx<'js>,
    value: Option<Value<'js>>,
) -> rquickjs::Result<Option<u64>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() || value.is_undefined() {
        return Ok(None);
    }
    let position: Coerced<i64> = Coerced::from_js(ctx, value)?;
    if *position < -1 {
        return Err(Exception::throw_range(
            ctx,
            "file position must be -1 or non-negative",
        ));
    }
    Ok((*position >= 0).then_some(position.cast_unsigned()))
}

pub(super) fn write_arguments<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
    args: Vec<Value<'js>>,
) -> rquickjs::Result<(Vec<u8>, Option<u64>)> {
    let string = value.is_string();
    let (offset, length, position, encoding) = if string {
        let position = args.first().cloned();
        let encoding = args
            .get(1)
            .and_then(|value| value.as_string())
            .map(rquickjs::String::to_string)
            .transpose()?;
        (0_u64, None, position, encoding)
    } else {
        let offset = args
            .first()
            .map(|value| Coerced::<u64>::from_js(ctx, value.clone()))
            .transpose()?
            .map_or(0, |value| *value);
        let length = args
            .get(1)
            .map(|value| Coerced::<u64>::from_js(ctx, value.clone()))
            .transpose()?;
        (
            offset,
            length.map(|value| *value),
            args.get(2).cloned(),
            None,
        )
    };
    let bytes = bytes(ctx, value, encoding.as_deref())?;
    let offset =
        usize::try_from(offset).map_err(|_| Exception::throw_range(ctx, "offset is too large"))?;
    if offset > bytes.len() {
        return Err(Exception::throw_range(ctx, "offset is outside the buffer"));
    }
    let length = length
        .map(|length| {
            usize::try_from(length).map_err(|_| Exception::throw_range(ctx, "length is too large"))
        })
        .transpose()?
        .unwrap_or(bytes.len() - offset);
    if length > bytes.len() - offset {
        return Err(Exception::throw_range(ctx, "length is outside the buffer"));
    }
    Ok((
        bytes[offset..offset + length].to_vec(),
        position_value(ctx, position)?,
    ))
}

pub(super) fn normalize_write_options<'js>(
    ctx: &Ctx<'js>,
    data_is_string: bool,
    args: Vec<Value<'js>>,
) -> rquickjs::Result<Vec<Value<'js>>> {
    let Some(first) = args.first() else {
        return Ok(args);
    };
    if !first.is_object() {
        return Ok(args);
    }
    let options = first
        .clone()
        .try_into_object()
        .map_err(|_| Exception::throw_type(ctx, "write options must be an object"))?;
    let value = |name: &str| -> rquickjs::Result<Value<'js>> {
        Ok(options
            .get::<_, Option<Value>>(name)?
            .unwrap_or_else(|| Value::new_undefined(ctx.clone())))
    };
    if data_is_string {
        Ok(vec![value("position")?, value("encoding")?])
    } else {
        Ok(vec![value("offset")?, value("length")?, value("position")?])
    }
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
