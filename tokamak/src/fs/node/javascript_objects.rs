#![allow(clippy::wildcard_imports)]

use std::collections::VecDeque;

use super::*;

pub(super) fn illegal_constructor(ctx: Ctx<'_>) -> rquickjs::Result<()> {
    Err(Exception::throw_type(&ctx, "Illegal constructor"))
}

pub(super) fn constructor<'js>(
    ctx: &Ctx<'js>,
    name: &str,
    prototype_name: &str,
) -> rquickjs::Result<(Function<'js>, Object<'js>)> {
    let function = Function::new(ctx.clone(), illegal_constructor)?;
    function.set_name(name)?;
    let function_object = Object::from_value(function.clone().into_value())?;
    let prototype = Object::new(ctx.clone())?;
    function_object.set("prototype", prototype.clone())?;
    prototype.set("constructor", function.clone())?;
    ctx.globals().set(prototype_name, prototype.clone())?;
    Ok((function, prototype))
}

pub(super) fn dirent_constructor<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Function<'js>> {
    let (function, prototype) = constructor(ctx, "Dirent", "__tokamak_node_fs_dirent_proto")?;
    for (name, method) in [
        ("isFile", dirent_is_file as fn(This<Object<'_>>) -> bool),
        ("isDirectory", dirent_is_directory),
        ("isBlockDevice", never),
        ("isCharacterDevice", dirent_is_character_device),
        ("isSymbolicLink", dirent_is_symbolic_link),
        ("isFIFO", never),
        ("isSocket", never),
    ] {
        prototype.set(name, Function::new(ctx.clone(), method)?)?;
    }
    Ok(function)
}

pub(super) fn dir_constructor<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Function<'js>> {
    constructor(ctx, "Dir", "__tokamak_node_fs_dir_proto").map(|(function, _)| function)
}

#[derive(Clone)]
pub(super) struct DirState {
    entries: Arc<Mutex<Option<VecDeque<DirItem>>>>,
    path: String,
    encoding: Option<String>,
}

#[derive(Clone)]
pub(super) struct DirItem {
    pub(super) name: String,
    pub(super) parent: String,
    pub(super) entry: DirectoryEntry,
}

pub(super) fn dir_object<'js>(
    ctx: &Ctx<'js>,
    path: String,
    entries: Vec<DirItem>,
    encoding: Option<&str>,
) -> rquickjs::Result<Object<'js>> {
    let prototype: Option<Object> = ctx.globals().get("__tokamak_node_fs_dir_proto")?;
    let object = Object::new_proto(ctx.clone(), prototype.as_ref())?;
    let state = DirState {
        entries: Arc::new(Mutex::new(Some(VecDeque::from(entries)))),
        path,
        encoding: encoding.map(ToOwned::to_owned),
    };
    object.set("path", state.path.clone())?;

    let read_state = state.clone();
    object.set(
        "readSync",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
            dir_read_entry(&ctx, &read_state)
        })?,
    )?;
    let read_state = state.clone();
    object.set(
        "read",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
            let mut args = args.0;
            let callback = pop_callback(&mut args);
            let result = dir_read_entry(&ctx, &read_state);
            if let Some(callback) = callback {
                callback_values(&ctx, callback, reply(&ctx, result))?;
                Ok(Value::new_undefined(ctx))
            } else {
                promise(ctx.clone(), result).map(Promise::into_value)
            }
        })?,
    )?;
    let close_state = state.clone();
    object.set(
        "closeSync",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
            dir_close(&ctx, &close_state)
        })?,
    )?;
    let close_state = state.clone();
    object.set(
        "close",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
            let mut args = args.0;
            let callback = pop_callback(&mut args);
            let result = dir_close(&ctx, &close_state).map(|()| Value::new_undefined(ctx.clone()));
            if let Some(callback) = callback {
                callback_values(
                    &ctx,
                    callback,
                    result.map(|_| vec![Value::new_null(ctx.clone())]),
                )?;
                Ok(Value::new_undefined(ctx))
            } else {
                promise(ctx.clone(), result).map(Promise::into_value)
            }
        })?,
    )?;
    object.set("entries", dir_iterator_function(ctx, state.clone())?)?;
    object.set(
        Symbol::async_iterator(ctx.clone()),
        dir_iterator_function(ctx, state)?,
    )?;
    Ok(object)
}

fn dir_iterator_function<'js>(ctx: &Ctx<'js>, state: DirState) -> rquickjs::Result<Function<'js>> {
    Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
        dir_iterator(&ctx, state.clone())
    })
}

pub(super) fn dir_read_entry<'js>(
    ctx: &Ctx<'js>,
    state: &DirState,
) -> rquickjs::Result<Value<'js>> {
    let item = lock(&state.entries)
        .as_mut()
        .ok_or_else(|| Exception::throw_message(ctx, "ERR_DIR_CLOSED: directory is closed"))?
        .pop_front();
    if let Some(item) = item {
        item_dirent(ctx, &item, state.encoding.as_deref()).map(Object::into_value)
    } else {
        Ok(Value::new_null(ctx.clone()))
    }
}

pub(super) fn item_dirent<'js>(
    ctx: &Ctx<'js>,
    item: &DirItem,
    encoding: Option<&str>,
) -> rquickjs::Result<Object<'js>> {
    let name = text_value(ctx, &item.name, encoding)?;
    dirent(ctx, &item.entry, name, normalized_parent(&item.parent))
}

pub(super) fn dir_close(ctx: &Ctx<'_>, state: &DirState) -> rquickjs::Result<()> {
    let mut entries = lock(&state.entries);
    if entries.take().is_none() {
        return Err(Exception::throw_message(
            ctx,
            "ERR_DIR_CLOSED: directory is closed",
        ));
    }
    Ok(())
}

pub(super) fn dir_iterator<'js>(ctx: &Ctx<'js>, state: DirState) -> rquickjs::Result<Object<'js>> {
    let next = Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
        let value = dir_read_entry(&ctx, &state)?;
        iterator_result(ctx, value.is_null(), value)
    })?;
    async_iterator(ctx, next)
}

#[derive(Clone)]
pub(super) struct FileHandleState {
    pub(super) descriptor: Arc<Mutex<Option<u32>>>,
    pub(super) owns_descriptor: bool,
}

pub(super) fn file_handle_constructor<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Function<'js>> {
    constructor(ctx, "FileHandle", "__tokamak_node_fs_file_handle_proto")
        .map(|(function, _)| function)
}

#[allow(clippy::too_many_lines)]
pub(super) fn file_handle_object<'js>(
    ctx: &Ctx<'js>,
    state: FileHandleState,
) -> rquickjs::Result<Object<'js>> {
    let prototype: Option<Object> = ctx.globals().get("__tokamak_node_fs_file_handle_proto")?;
    let object = Object::new_proto(ctx.clone(), prototype.as_ref())?;
    install_event_emitter(ctx, &object)?;
    let descriptor = handle_descriptor(&state)
        .ok_or_else(|| Exception::throw_internal(ctx, "file descriptor was closed"))?;
    object.set("fd", descriptor)?;

    let read_state = state.clone();
    object.set(
        "read",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
            handle_read(&ctx, &read_state, args)
        })?,
    )?;
    let vector_read_state = state.clone();
    object.set(
        "readv",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, buffers: Array<'js>, position: Opt<Value<'js>>| {
                let descriptor = open_descriptor(&ctx, &vector_read_state)?;
                let bytes_read = readv_sync(ctx.clone(), descriptor, buffers.clone(), position)?;
                let buffers = buffers.into_value();
                transfer_result(&ctx, "bytesRead", bytes_read as usize, "buffers", buffers)
            },
        )?,
    )?;
    let read_file_state = state.clone();
    object.set(
        "readFile",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, options: Opt<Value<'js>>| {
                let descriptor = open_descriptor(&ctx, &read_file_state)?;
                let options = parse_options(&ctx, options.0)?;
                let bytes = read_descriptor(&ctx, descriptor)?;
                promise(
                    ctx.clone(),
                    output(ctx.clone(), &bytes, options.encoding.as_deref()),
                )
            },
        )?,
    )?;
    object.set(
        "readLines",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, _options: Opt<Value<'js>>| {
                Err::<(), rquickjs::Error>(Exception::throw_message(
                    &ctx,
                    "readLines is not implemented",
                ))
            },
        )?,
    )?;
    let write_state = state.clone();
    object.set(
        "write",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
            handle_write(&ctx, &write_state, args)
        })?,
    )?;
    let vector_write_state = state.clone();
    object.set(
        "writev",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, buffers: Array<'js>, position: Opt<Value<'js>>| {
                let descriptor = open_descriptor(&ctx, &vector_write_state)?;
                let bytes_written = write_buffers(&ctx, descriptor, &buffers, position.0)?;
                let buffers = buffers.into_value();
                transfer_result(&ctx, "bytesWritten", bytes_written, "buffers", buffers)
            },
        )?,
    )?;
    let write_file_state = state.clone();
    object.set(
        "writeFile",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, data: Value<'js>, options: Opt<Value<'js>>| {
                let descriptor = open_descriptor(&ctx, &write_file_state)?;
                let options = parse_options(&ctx, options.0)?;
                let bytes = bytes(&ctx, data.clone(), options.encoding.as_deref())?;
                let bytes_written = write_descriptor(&ctx, descriptor, &bytes, false)?;
                let buffer = write_buffer_value(&ctx, &data, &bytes)?;
                transfer_result(&ctx, "bytesWritten", bytes_written, "buffer", buffer)
            },
        )?,
    )?;
    let append_state = state.clone();
    object.set(
        "appendFile",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, data: Value<'js>, options: Opt<Value<'js>>| {
                let descriptor = open_descriptor(&ctx, &append_state)?;
                let options = parse_options(&ctx, options.0)?;
                let data = bytes(&ctx, data, options.encoding.as_deref())?;
                write_descriptor(&ctx, descriptor, &data, true)?;
                promise_unit(ctx, Ok(()))
            },
        )?,
    )?;
    let stat_state = state.clone();
    object.set(
        "stat",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, options: Opt<Value<'js>>| {
                let descriptor = open_descriptor(&ctx, &stat_state)?;
                let options = parse_options(&ctx, options.0)?;
                let stat = vfs_call(&ctx, |vfs| vfs.fstat(descriptor))?;
                promise(
                    ctx.clone(),
                    stat_object(&ctx, &stat, options.bigint).map(Object::into_value),
                )
            },
        )?,
    )?;
    let truncate_state = state.clone();
    object.set(
        "truncate",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, length: Opt<u64>| {
            let descriptor = open_descriptor(&ctx, &truncate_state)?;
            vfs_call(&ctx, |vfs| vfs.ftruncate(descriptor, length.0.unwrap_or(0)))?;
            promise_unit(ctx, Ok(()))
        })?,
    )?;
    for (name, function) in [
        ("sync", file_handle_sync(ctx.clone(), state.clone())?),
        ("datasync", file_handle_sync(ctx.clone(), state.clone())?),
    ] {
        object.set(name, function)?;
    }
    let chmod_state = state.clone();
    object.set(
        "chmod",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, mode: Value<'js>| {
            fchmod_sync(ctx.clone(), open_descriptor(&ctx, &chmod_state)?, mode)?;
            promise_unit(ctx, Ok(()))
        })?,
    )?;
    let chown_state = state.clone();
    object.set(
        "chown",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, uid: u32, gid: u32| {
            fchown_sync(ctx.clone(), open_descriptor(&ctx, &chown_state)?, uid, gid)?;
            promise_unit(ctx, Ok(()))
        })?,
    )?;
    let utimes_state = state.clone();
    object.set(
        "utimes",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, atime: Value<'js>, mtime: Value<'js>| {
                let descriptor = open_descriptor(&ctx, &utimes_state)?;
                futimes_sync(ctx.clone(), descriptor, atime, mtime)?;
                promise_unit(ctx, Ok(()))
            },
        )?,
    )?;
    let close_state = state.clone();
    object.set(
        "close",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, this: This<Object<'js>>| {
                promise_unit(ctx.clone(), close_handle(&ctx, &close_state, &this.0))
            },
        )?,
    )?;
    object.set("createReadStream", file_handle_stream(ctx, &state, true)?)?;
    object.set("createWriteStream", file_handle_stream(ctx, &state, false)?)?;
    let web_stream_state = state;
    object.set(
        "readableWebStream",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, _options: Opt<Value<'js>>| {
                readable_web_stream(&ctx, open_descriptor(&ctx, &web_stream_state)?)
            },
        )?,
    )?;
    Ok(object)
}

fn file_handle_stream<'js>(
    ctx: &Ctx<'js>,
    state: &FileHandleState,
    readable: bool,
) -> rquickjs::Result<Function<'js>> {
    let state = state.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, options: Opt<Value<'js>>| {
            let descriptor = open_descriptor(&ctx, &state)?;
            create_stream(&ctx, readable, None, Some(descriptor), options.0)
        },
    )
}

pub(super) fn handle_descriptor(state: &FileHandleState) -> Option<u32> {
    *lock(&state.descriptor)
}

/// The handle's descriptor, or an `EBADF` error once it is closed.
fn open_descriptor(ctx: &Ctx<'_>, state: &FileHandleState) -> rquickjs::Result<u32> {
    handle_descriptor(state)
        .ok_or_else(|| Exception::throw_message(ctx, "EBADF: file descriptor closed"))
}

/// A promise of `{ [count_name]: count, [buffer_name]: buffer }`.
fn transfer_result<'js>(
    ctx: &Ctx<'js>,
    count_name: &str,
    count: usize,
    buffer_name: &str,
    buffer: Value<'js>,
) -> rquickjs::Result<Promise<'js>> {
    let result = Object::new(ctx.clone())?;
    result.set(count_name, count)?;
    result.set(buffer_name, buffer)?;
    promise(ctx.clone(), Ok(result.into_value()))
}

pub(super) fn take_descriptor(state: &FileHandleState) -> Option<u32> {
    lock(&state.descriptor).take()
}

pub(super) fn close_handle<'js>(
    ctx: &Ctx<'js>,
    state: &FileHandleState,
    object: &Object<'js>,
) -> rquickjs::Result<()> {
    let descriptor = take_descriptor(state);
    if let Some(descriptor) = descriptor {
        vfs_call(ctx, |vfs| vfs.close(descriptor))?;
        object.set("fd", Value::new_undefined(ctx.clone()))?;
        if let Ok(Some(emit)) = object.get::<_, Option<Function>>("emit") {
            emit.call::<_, bool>((This(object.clone()), "close"))?;
        }
    }
    Ok(())
}

pub(super) fn install_event_emitter<'js>(
    ctx: &Ctx<'js>,
    object: &Object<'js>,
) -> rquickjs::Result<()> {
    let process: Option<Object> = ctx.globals().get("process")?;
    let Some(process) = process else {
        return Ok(());
    };
    let Ok(get_builtin) = process.get::<_, Function>("getBuiltinModule") else {
        return Ok(());
    };
    let Ok(Some(module)) = get_builtin.call::<_, Option<Object>>(("node:events",)) else {
        return Ok(());
    };
    let Ok(Some(constructor)) = module.get::<_, Option<Constructor>>("EventEmitter") else {
        return Ok(());
    };
    let Ok(Some(prototype)) = constructor.get::<_, Option<Object>>("prototype") else {
        return Ok(());
    };
    object.set("__events", ctx.eval::<Object, _>("new Map()")?)?;
    for name in [
        "on",
        "addEventListener",
        "addListener",
        "once",
        "off",
        "removeListener",
        "removeEventListener",
        "removeAllListeners",
        "emit",
        "listeners",
        "listenerCount",
        "eventNames",
        "setMaxListeners",
        "getMaxListeners",
        "prependListener",
        "prependOnceListener",
    ] {
        if let Some(method) = prototype.get::<_, Option<Function>>(name)? {
            object.set(name, method)?;
        }
    }
    Ok(())
}

pub(super) fn file_handle_sync<'js>(
    ctx: Ctx<'js>,
    state: FileHandleState,
) -> rquickjs::Result<Function<'js>> {
    Function::new(ctx, move |ctx: Ctx<'js>| {
        require_descriptor(&ctx, open_descriptor(&ctx, &state)?)?;
        promise_unit(ctx, Ok(()))
    })
}

pub(super) fn handle_read<'js>(
    ctx: &Ctx<'js>,
    state: &FileHandleState,
    args: Rest<Value<'js>>,
) -> rquickjs::Result<Promise<'js>> {
    let descriptor = open_descriptor(ctx, state)?;
    let ReadArguments {
        buffer,
        offset: offset_value,
        length: length_value,
        position: position_arg,
    } = normalize_read_arguments(ctx, args.0)?;
    let writable = writable_bytes(ctx, buffer.clone())?;
    let offset = offset_value
        .map(|value| Coerced::<u64>::from_js(ctx, value))
        .transpose()?
        .map_or(0, |value| usize::try_from(*value).unwrap_or(usize::MAX));
    let length = length_value
        .map(|value| Coerced::<u64>::from_js(ctx, value))
        .transpose()?
        .map_or(writable.len.saturating_sub(offset), |value| {
            usize::try_from(*value).unwrap_or(usize::MAX)
        });
    let position = position_value(ctx, position_arg)?;
    if offset > writable.len || length > writable.len - offset {
        return Err(Exception::throw_range(ctx, "read buffer range is invalid"));
    }
    let bytes = vfs_call(ctx, |vfs| vfs.read(descriptor, length, position))?;
    writable.write(offset, &bytes);
    transfer_result(ctx, "bytesRead", bytes.len(), "buffer", buffer)
}

pub(super) fn handle_write<'js>(
    ctx: &Ctx<'js>,
    state: &FileHandleState,
    args: Rest<Value<'js>>,
) -> rquickjs::Result<Promise<'js>> {
    let descriptor = open_descriptor(ctx, state)?;
    let mut args = args.0;
    if args.is_empty() {
        return Err(Exception::throw_type(ctx, "data is required"));
    }
    let value = args.remove(0);
    let args = normalize_write_options(ctx, value.is_string(), args)?;
    let (data, position) = write_arguments(ctx, value.clone(), args)?;
    let bytes_written = vfs_call(ctx, |vfs| vfs.write(descriptor, &data, position))?;
    let buffer = write_buffer_value(ctx, &value, &data)?;
    transfer_result(ctx, "bytesWritten", bytes_written, "buffer", buffer)
}

/// A `createReadStream` or `createWriteStream` export.
pub(super) fn stream_function<'js>(
    ctx: &Ctx<'js>,
    readable: bool,
) -> rquickjs::Result<Function<'js>> {
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, input: Value<'js>, options: Opt<Value<'js>>| {
            create_stream(&ctx, readable, Some(input), None, options.0)
        },
    )
}

pub(super) fn create_stream<'js>(
    ctx: &Ctx<'js>,
    readable: bool,
    input: Option<Value<'js>>,
    descriptor: Option<u32>,
    options: Option<Value<'js>>,
) -> rquickjs::Result<Object<'js>> {
    if readable {
        create_read_stream(ctx, input, descriptor, options)
    } else {
        create_write_stream(ctx, input, descriptor, options)
    }
}

fn create_read_stream<'js>(
    ctx: &Ctx<'js>,
    input: Option<Value<'js>>,
    descriptor: Option<u32>,
    options: Option<Value<'js>>,
) -> rquickjs::Result<Object<'js>> {
    let (path, descriptor, owns_descriptor) =
        stream_descriptor(ctx, input, descriptor, options.as_ref(), None)?;
    let bytes = read_descriptor(ctx, descriptor)?;
    let stream = stream_object(ctx, "Readable")?;
    stream.set("path", path.unwrap_or_default())?;
    stream.set("fd", descriptor)?;
    stream.set("bytesRead", bytes.len())?;
    let push: Function = ctx.eval("(stream, chunk) => stream.push(chunk)")?;
    push.call::<_, ()>((
        stream.clone(),
        TypedArray::<u8>::new_copy(ctx.clone(), &bytes)?,
    ))?;
    push.call::<_, ()>((stream.clone(), Value::new_null(ctx.clone())))?;
    if owns_descriptor {
        let _ = vfs_call(ctx, |vfs| vfs.close(descriptor));
        stream.set("fd", Value::new_null(ctx.clone()))?;
    }
    Ok(stream)
}

fn create_write_stream<'js>(
    ctx: &Ctx<'js>,
    input: Option<Value<'js>>,
    descriptor: Option<u32>,
    options: Option<Value<'js>>,
) -> rquickjs::Result<Object<'js>> {
    let (path, descriptor, owns_descriptor) =
        stream_descriptor(ctx, input, descriptor, options.as_ref(), Some("w"))?;
    let stream = stream_object(ctx, "Writable")?;
    stream.set("path", path.unwrap_or_default())?;
    stream.set("fd", descriptor)?;
    let state = FileHandleState {
        descriptor: Arc::new(Mutex::new(Some(descriptor))),
        owns_descriptor,
    };
    let write_state = state.clone();
    stream.set(
        "write",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
            stream_write(&ctx, &write_state, args)
        })?,
    )?;
    let end_state = state.clone();
    stream.set(
        "end",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, this: This<Object<'js>>, args: Rest<Value<'js>>| {
                stream_end(&ctx, &end_state, this.0, args)
            },
        )?,
    )?;
    let destroy_state = state;
    stream.set(
        "destroy",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, this: This<Object<'js>>, _error: Opt<Value<'js>>| {
                if destroy_state.owns_descriptor
                    && let Some(descriptor) = take_descriptor(&destroy_state)
                {
                    let _ = vfs_call(&ctx, |vfs| vfs.close(descriptor));
                }
                Ok::<Object<'js>, rquickjs::Error>(this.0)
            },
        )?,
    )?;
    Ok(stream)
}

/// The stream's path and descriptor, and whether the stream opened that
/// descriptor itself, with `default_flags` when `options` names none.
fn stream_descriptor<'js>(
    ctx: &Ctx<'js>,
    input: Option<Value<'js>>,
    descriptor: Option<u32>,
    options: Option<&Value<'js>>,
    default_flags: Option<&str>,
) -> rquickjs::Result<(Option<String>, u32, bool)> {
    let path = input.map(|value| path(ctx, value)).transpose()?;
    let option_descriptor = option_descriptor(ctx, options)?;
    if let Some(descriptor) = descriptor.or(option_descriptor) {
        return Ok((path, descriptor, false));
    }
    let open_path = path
        .as_deref()
        .ok_or_else(|| Exception::throw_type(ctx, "stream path is required"))?;
    let flags = option_property(ctx, options, "flags")?
        .or_else(|| default_flags.and_then(|flags| flags.into_js(ctx).ok()));
    let open_options = open_options(ctx, flags)?;
    let descriptor = vfs_call(ctx, |vfs| vfs.open(open_path, open_options))?;
    Ok((path, descriptor, true))
}

pub(super) fn option_descriptor<'js>(
    ctx: &Ctx<'js>,
    options: Option<&Value<'js>>,
) -> rquickjs::Result<Option<u32>> {
    let Some(value) = defined(option_property(ctx, options, "fd")?) else {
        return Ok(None);
    };
    if value.is_object() {
        let object = value
            .try_into_object()
            .map_err(|_| Exception::throw_type(ctx, "options.fd must be a file descriptor"))?;
        return object
            .get::<_, Option<Value>>("fd")?
            .map(|value| descriptor_value(ctx, value))
            .transpose();
    }
    descriptor_value(ctx, value).map(Some)
}

pub(super) fn stream_object<'js>(ctx: &Ctx<'js>, kind: &str) -> rquickjs::Result<Object<'js>> {
    let process: Option<Object> = ctx.globals().get("process")?;
    if let Some(process) = process
        && let Ok(get_builtin) = process.get::<_, Function>("getBuiltinModule")
        && let Ok(module) = get_builtin.call::<_, Option<Object>>(("node:stream",))
        && let Some(module) = module
        && let Ok(constructor) = module.get::<_, Constructor>(kind)
    {
        return constructor.construct(());
    }
    Object::new(ctx.clone())
}

pub(super) fn stream_write<'js>(
    ctx: &Ctx<'js>,
    state: &FileHandleState,
    args: Rest<Value<'js>>,
) -> rquickjs::Result<bool> {
    let mut args = args.0;
    if args.is_empty() {
        return Err(Exception::throw_type(ctx, "chunk is required"));
    }
    let value = args.remove(0);
    let callback = pop_callback(&mut args);
    let (data, position) = if value.is_string() {
        let encoding = args
            .first()
            .and_then(|value| value.as_string())
            .map(rquickjs::String::to_string)
            .transpose()?;
        (bytes(ctx, value, encoding.as_deref())?, None)
    } else {
        let args = normalize_write_options(ctx, false, args)?;
        write_arguments(ctx, value, args)?
    };
    let descriptor = open_descriptor(ctx, state)?;
    let result = vfs_call(ctx, |vfs| vfs.write(descriptor, &data, position));
    if let Some(callback) = callback {
        match result {
            Ok(_) => callback.call::<_, ()>((Value::new_null(ctx.clone()),))?,
            Err(_error) if ctx.has_exception() => callback.call::<_, ()>((ctx.catch(),))?,
            Err(error) => return Err(error),
        }
    }
    Ok(true)
}

pub(super) fn stream_end<'js>(
    ctx: &Ctx<'js>,
    state: &FileHandleState,
    stream: Object<'js>,
    args: Rest<Value<'js>>,
) -> rquickjs::Result<Object<'js>> {
    let mut args = args.0;
    let callback = pop_callback(&mut args);
    if let Some(value) = args.into_iter().next() {
        stream_write(ctx, state, Rest(vec![value]))?;
    }
    if state.owns_descriptor
        && let Some(descriptor) = take_descriptor(state)
    {
        vfs_call(ctx, |vfs| vfs.close(descriptor))?;
    }
    if let Some(callback) = callback {
        callback.call::<_, ()>((Value::new_null(ctx.clone()),))?;
    }
    Ok(stream)
}

pub(super) fn readable_web_stream<'js>(
    ctx: &Ctx<'js>,
    descriptor: u32,
) -> rquickjs::Result<Object<'js>> {
    let bytes = read_descriptor(ctx, descriptor)?;
    let constructor: Constructor = ctx
        .globals()
        .get("ReadableStream")
        .map_err(|_| Exception::throw_internal(ctx, "ReadableStream is not installed"))?;
    let source = Object::new(ctx.clone())?;
    source.set(
        "start",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, controller: Object<'js>| {
                let enqueue: Function = controller.get("enqueue")?;
                enqueue.call::<_, ()>((TypedArray::<u8>::new_copy(ctx.clone(), &bytes)?,))?;
                let close: Function = controller.get("close")?;
                close.call::<_, ()>(())?;
                Ok::<(), rquickjs::Error>(())
            },
        )?,
    )?;
    constructor.construct((source,))
}

pub(super) fn stats_constructor<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Function<'js>> {
    let (function, prototype) = constructor(ctx, "Stats", "__tokamak_node_fs_stats_proto")?;
    for (name, file_type) in [
        ("isFile", 0o100_000),
        ("isDirectory", 0o040_000),
        ("isBlockDevice", 0o060_000),
        ("isCharacterDevice", 0o020_000),
        ("isSymbolicLink", 0o120_000),
        ("isFIFO", 0o010_000),
        ("isSocket", 0o140_000),
    ] {
        let method = move |this: This<Object<'js>>| stats_type(&this) == Some(file_type);
        prototype.set(name, Function::new(ctx.clone(), method)?)?;
    }
    Ok(function)
}

pub(super) fn stats_mode(this: &This<Object<'_>>) -> Option<u32> {
    let mode: Value = this.0.get("mode").ok()?;
    if mode.is_big_int() {
        let bigint = BigInt::from_value(mode).ok()?;
        return u32::try_from(bigint.to_i64().ok()?).ok();
    }
    let ctx = mode.ctx().clone();
    Coerced::<i64>::from_js(&ctx, mode)
        .ok()
        .and_then(|value| u32::try_from(*value).ok())
}

pub(super) fn stats_type(this: &This<Object<'_>>) -> Option<u32> {
    Some(stats_mode(this)? & 0o170_000)
}

fn dirent_device(object: &Object<'_>) -> bool {
    matches!(object.get::<_, Option<bool>>("device"), Ok(Some(true)))
}

fn dirent_has_type(object: &Object<'_>, kind: &str) -> bool {
    object
        .get::<_, Option<String>>("type")
        .ok()
        .flatten()
        .is_some_and(|value| value == kind)
}

fn dirent_is_file(this: This<Object<'_>>) -> bool {
    !dirent_device(&this.0) && dirent_has_type(&this.0, "file")
}

fn dirent_is_directory(this: This<Object<'_>>) -> bool {
    dirent_has_type(&this.0, "directory")
}

fn dirent_is_character_device(this: This<Object<'_>>) -> bool {
    dirent_device(&this.0)
}

fn dirent_is_symbolic_link(this: This<Object<'_>>) -> bool {
    dirent_has_type(&this.0, "symlink")
}

fn never(_: This<Object<'_>>) -> bool {
    false
}

pub(super) fn stat_object<'js>(
    ctx: &Ctx<'js>,
    stat: &Stat,
    bigint: bool,
) -> rquickjs::Result<Object<'js>> {
    let prototype: Option<Object> = ctx.globals().get("__tokamak_node_fs_stats_proto")?;
    let object = Object::new_proto(ctx.clone(), prototype.as_ref())?;
    let is_file = stat.kind == NodeType::File && !stat.device;
    let is_directory = stat.kind == NodeType::Directory;
    let mode: u32 = if stat.device {
        0o020_666
    } else if is_file {
        if stat.writable { 0o100_666 } else { 0o100_444 }
    } else if is_directory {
        if stat.writable { 0o40_777 } else { 0o40_555 }
    } else {
        0o120_777
    };
    object.set("type", node_type_name(stat.kind))?;
    for (name, value) in [
        ("dev", u64::from(stat.device)),
        ("ino", 0),
        ("mode", u64::from(mode)),
        ("nlink", 1),
        ("uid", 0),
        ("gid", 0),
        ("rdev", 0),
        ("size", stat.size),
        ("blksize", 0),
        ("blocks", 0),
    ] {
        set_number_or_bigint(ctx, &object, name, value, bigint)?;
    }
    object.set("writable", stat.writable)?;
    object.set("device", stat.device)?;
    for name in ["atimeMs", "mtimeMs", "ctimeMs", "birthtimeMs"] {
        set_number_or_bigint(ctx, &object, name, 0, bigint)?;
    }
    if bigint {
        for name in ["atimeNs", "mtimeNs", "ctimeNs", "birthtimeNs"] {
            object.set(name, BigInt::from_i64(ctx.clone(), 0)?)?;
        }
    }
    let date: Object = ctx.eval("new Date(0)")?;
    for name in ["atime", "mtime", "ctime", "birthtime"] {
        object.set(name, date.clone())?;
    }
    Ok(object)
}

pub(super) fn set_number_or_bigint<'js>(
    ctx: &Ctx<'js>,
    object: &Object<'js>,
    name: &str,
    value: u64,
    bigint: bool,
) -> rquickjs::Result<()> {
    if bigint {
        object.set(name, BigInt::from_u64(ctx.clone(), value)?)
    } else {
        object.set(name, value)
    }
}

pub(super) fn text_value<'js>(
    ctx: &Ctx<'js>,
    value: &str,
    encoding: Option<&str>,
) -> rquickjs::Result<Value<'js>> {
    match encoding.map(str::to_ascii_lowercase).as_deref() {
        None | Some("utf8" | "utf-8") => value.to_owned().into_js(ctx),
        Some(_) => output(ctx.clone(), value.as_bytes(), encoding),
    }
}

pub(super) fn normalized_parent(path: &str) -> &str {
    if path.is_empty() { "/" } else { path }
}

pub(super) fn validate_mode<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<()> {
    if value.is_number() {
        let mode: Coerced<i64> = Coerced::from_js(ctx, value)?;
        if (0..=i64::from(u32::MAX)).contains(&*mode) {
            return Ok(());
        }
    } else if let Some(string) = value.as_string()
        && u32::from_str_radix(&string.to_string()?, 8).is_ok()
    {
        return Ok(());
    }
    Err(Exception::throw_type(ctx, "invalid file mode"))
}

pub(super) fn statfs_object<'js>(ctx: &Ctx<'js>, bigint: bool) -> rquickjs::Result<Object<'js>> {
    let object = Object::new(ctx.clone())?;
    for name in [
        "type", "bsize", "blocks", "bfree", "bavail", "files", "ffree",
    ] {
        set_number_or_bigint(ctx, &object, name, 0, bigint)?;
    }
    Ok(object)
}

pub(super) fn glob_matches(
    ctx: &Ctx<'_>,
    cwd: &str,
    pattern: &str,
) -> rquickjs::Result<Vec<(String, DirectoryEntry)>> {
    let entries = vfs_call(ctx, |vfs| vfs.walk(cwd))?;
    let patterns = expand_braces(pattern);
    let mut result = Vec::new();
    for (path, entry) in entries {
        let candidate = glob_candidate(&path, cwd, pattern);
        if patterns
            .iter()
            .any(|expanded| glob_match(expanded, &candidate))
        {
            result.push((path, entry));
        }
    }
    result.sort_by(|left, right| left.0.cmp(&right.0));
    result.dedup_by(|left, right| left.0 == right.0);
    Ok(result)
}

pub(super) fn expand_braces(pattern: &str) -> Vec<String> {
    let Some(start) = pattern.find('{') else {
        return vec![pattern.to_owned()];
    };
    let Some(end) = pattern[start + 1..].find('}') else {
        return vec![pattern.to_owned()];
    };
    let end = start + 1 + end;
    let prefix = &pattern[..start];
    let suffix = &pattern[end + 1..];
    pattern[start + 1..end]
        .split(',')
        .flat_map(|part| expand_braces(&format!("{prefix}{part}{suffix}")))
        .collect()
}

pub(super) fn glob_match(pattern: &str, path: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').filter(|part| !part.is_empty()).collect();
    let path: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    glob_segments(&pattern, &path)
}

pub(super) fn glob_segments(pattern: &[&str], path: &[&str]) -> bool {
    if pattern.is_empty() {
        return path.is_empty();
    }
    if pattern[0] == "**" {
        return glob_segments(&pattern[1..], path)
            || (!path.is_empty() && glob_segments(pattern, &path[1..]));
    }
    !path.is_empty()
        && segment_match(pattern[0], path[0])
        && glob_segments(&pattern[1..], &path[1..])
}

pub(super) fn segment_match(pattern: &str, value: &str) -> bool {
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    let mut states = vec![false; value.len() + 1];
    let mut next = vec![false; value.len() + 1];
    states[0] = true;
    for &character in pattern {
        next.fill(false);
        if character == b'*' {
            next[0] = states[0];
            for index in 1..=value.len() {
                next[index] = states[index] || next[index - 1];
            }
        } else {
            for index in 1..=value.len() {
                next[index] =
                    states[index - 1] && (character == b'?' || character == value[index - 1]);
            }
        }
        std::mem::swap(&mut states, &mut next);
    }
    states[value.len()]
}

/// Removes a trailing callback argument, leaving other trailing values in place.
fn pop_callback<'js>(args: &mut Vec<Value<'js>>) -> Option<Function<'js>> {
    args.pop_if(|value| value.is_function())
        .and_then(Value::into_function)
}

pub(super) fn blob_object<'js>(
    ctx: &Ctx<'js>,
    bytes: Vec<u8>,
    mime_type: &str,
) -> rquickjs::Result<Object<'js>> {
    if let Some(constructor) = ctx.globals().get::<_, Option<Constructor>>("Blob")? {
        let parts = Array::new(ctx.clone())?;
        parts.set(0, TypedArray::<u8>::new_copy(ctx.clone(), &bytes)?)?;
        let options = Object::new(ctx.clone())?;
        options.set("type", mime_type)?;
        return constructor.construct((parts, options));
    }
    let object = Object::new(ctx.clone())?;
    object.set("size", bytes.len())?;
    object.set("type", mime_type)?;
    let text_bytes = bytes.clone();
    object.set(
        "text",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
            promise(
                ctx.clone(),
                String::from_utf8_lossy(&text_bytes)
                    .into_owned()
                    .into_js(&ctx),
            )
        })?,
    )?;
    object.set(
        "arrayBuffer",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
            promise(
                ctx.clone(),
                ArrayBuffer::new_copy(ctx.clone(), &bytes).map(ArrayBuffer::into_value),
            )
        })?,
    )?;
    Ok(object)
}

const fn node_type_name(kind: NodeType) -> &'static str {
    match kind {
        NodeType::File => "file",
        NodeType::Directory => "directory",
        NodeType::Symlink => "symlink",
    }
}

pub(super) fn dirent<'js>(
    ctx: &Ctx<'js>,
    entry: &DirectoryEntry,
    name: Value<'js>,
    parent_path: &str,
) -> rquickjs::Result<Object<'js>> {
    let prototype: Option<Object> = ctx.globals().get("__tokamak_node_fs_dirent_proto")?;
    let object = Object::new_proto(ctx.clone(), prototype.as_ref())?;
    object.set("name", name)?;
    object.set("parentPath", parent_path)?;
    object.set("type", node_type_name(entry.kind))?;
    object.set("device", entry.device)?;
    set_type_methods(ctx, &object, entry.kind, entry.device)?;
    Ok(object)
}

pub(super) fn set_type_methods<'js>(
    ctx: &Ctx<'js>,
    object: &Object<'js>,
    kind: NodeType,
    device: bool,
) -> rquickjs::Result<()> {
    for (name, result) in [
        ("isFile", kind == NodeType::File && !device),
        ("isDirectory", kind == NodeType::Directory),
        ("isSymbolicLink", kind == NodeType::Symlink),
        ("isCharacterDevice", device),
        ("isBlockDevice", false),
        ("isFIFO", false),
        ("isSocket", false),
    ] {
        object.set(name, Function::new(ctx.clone(), move || result)?)?;
    }
    Ok(())
}
