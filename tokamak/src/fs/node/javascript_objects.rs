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
        ("isBlockDevice", dirent_is_block_device),
        ("isCharacterDevice", dirent_is_character_device),
        ("isSymbolicLink", dirent_is_symbolic_link),
        ("isFIFO", dirent_is_fifo),
        ("isSocket", dirent_is_socket),
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

#[allow(clippy::too_many_lines)]
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
    let entries_state = state.clone();
    object.set(
        "entries",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
            dir_iterator(&ctx, entries_state.clone())
        })?,
    )?;
    let iterator_state = state;
    object.set(
        Symbol::async_iterator(ctx.clone()),
        Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
            dir_iterator(&ctx, iterator_state.clone())
        })?,
    )?;
    Ok(object)
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
        dirent(
            ctx,
            &item.entry,
            name_value(ctx, &item.name, state.encoding.as_deref())?,
            normalized_parent(&item.parent),
        )
        .map(Object::into_value)
    } else {
        Ok(Value::new_null(ctx.clone()))
    }
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
    let iterator = Object::new(ctx.clone())?;
    iterator.set(
        "next",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
            let value = dir_read_entry(&ctx, &state)?;
            let next = Object::new(ctx.clone())?;
            next.set("done", value.is_null())?;
            next.set("value", value)?;
            promise(ctx.clone(), Ok(next.into_value())).map(Promise::into_value)
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
                let descriptor = handle_descriptor(&vector_read_state).ok_or_else(|| {
                    Exception::throw_message(&ctx, "EBADF: file descriptor closed")
                })?;
                let bytes_read = readv_sync(ctx.clone(), descriptor, buffers.clone(), position)?;
                let result = Object::new(ctx.clone())?;
                result.set("bytesRead", bytes_read)?;
                result.set("buffers", buffers)?;
                promise(ctx.clone(), Ok(result.into_value()))
            },
        )?,
    )?;
    let read_file_state = state.clone();
    object.set(
        "readFile",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, options: Opt<Value<'js>>| {
                let descriptor = handle_descriptor(&read_file_state).ok_or_else(|| {
                    Exception::throw_message(&ctx, "EBADF: file descriptor closed")
                })?;
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
                let descriptor = handle_descriptor(&vector_write_state).ok_or_else(|| {
                    Exception::throw_message(&ctx, "EBADF: file descriptor closed")
                })?;
                let position = position_value(&ctx, position.0)?;
                let values = buffers
                    .iter::<Value>()
                    .map(|value| bytes(&ctx, value?, None))
                    .collect::<rquickjs::Result<Vec<_>>>()?;
                let bytes_written =
                    vfs_call(&ctx, |vfs| vfs.writev(descriptor, &values, position))?;
                let result = Object::new(ctx.clone())?;
                result.set("bytesWritten", bytes_written)?;
                result.set("buffers", buffers)?;
                promise(ctx.clone(), Ok(result.into_value()))
            },
        )?,
    )?;
    let write_file_state = state.clone();
    object.set(
        "writeFile",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, data: Value<'js>, options: Opt<Value<'js>>| {
                let descriptor = handle_descriptor(&write_file_state).ok_or_else(|| {
                    Exception::throw_message(&ctx, "EBADF: file descriptor closed")
                })?;
                let options = parse_options(&ctx, options.0)?;
                let bytes = bytes(&ctx, data.clone(), options.encoding.as_deref())?;
                let bytes_written = write_descriptor(&ctx, descriptor, &bytes, false)?;
                let result = Object::new(ctx.clone())?;
                result.set("bytesWritten", bytes_written)?;
                result.set("buffer", write_buffer_value(&ctx, &data, &bytes)?)?;
                promise(ctx.clone(), Ok(result.into_value()))
            },
        )?,
    )?;
    let append_state = state.clone();
    object.set(
        "appendFile",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, data: Value<'js>, options: Opt<Value<'js>>| {
                let descriptor = handle_descriptor(&append_state).ok_or_else(|| {
                    Exception::throw_message(&ctx, "EBADF: file descriptor closed")
                })?;
                let options = parse_options(&ctx, options.0)?;
                let data = bytes(&ctx, data, options.encoding.as_deref())?;
                write_descriptor(&ctx, descriptor, &data, true)?;
                promise(ctx.clone(), Ok(Value::new_undefined(ctx)))
            },
        )?,
    )?;
    let stat_state = state.clone();
    object.set(
        "stat",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, options: Opt<Value<'js>>| {
                let descriptor = handle_descriptor(&stat_state).ok_or_else(|| {
                    Exception::throw_message(&ctx, "EBADF: file descriptor closed")
                })?;
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
            let descriptor = handle_descriptor(&truncate_state)
                .ok_or_else(|| Exception::throw_message(&ctx, "EBADF: file descriptor closed"))?;
            vfs_call(&ctx, |vfs| vfs.ftruncate(descriptor, length.0.unwrap_or(0)))?;
            promise(ctx.clone(), Ok(Value::new_undefined(ctx)))
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
            let descriptor = handle_descriptor(&chmod_state)
                .ok_or_else(|| Exception::throw_message(&ctx, "EBADF: file descriptor closed"))?;
            fchmod_sync(ctx.clone(), descriptor, mode)?;
            promise(ctx.clone(), Ok(Value::new_undefined(ctx)))
        })?,
    )?;
    let chown_state = state.clone();
    object.set(
        "chown",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, uid: u32, gid: u32| {
            let descriptor = handle_descriptor(&chown_state)
                .ok_or_else(|| Exception::throw_message(&ctx, "EBADF: file descriptor closed"))?;
            fchown_sync(ctx.clone(), descriptor, uid, gid)?;
            promise(ctx.clone(), Ok(Value::new_undefined(ctx)))
        })?,
    )?;
    let utimes_state = state.clone();
    object.set(
        "utimes",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, atime: Value<'js>, mtime: Value<'js>| {
                let descriptor = handle_descriptor(&utimes_state).ok_or_else(|| {
                    Exception::throw_message(&ctx, "EBADF: file descriptor closed")
                })?;
                futimes_sync(ctx.clone(), descriptor, atime, mtime)?;
                promise(ctx.clone(), Ok(Value::new_undefined(ctx)))
            },
        )?,
    )?;
    let close_state = state.clone();
    object.set(
        "close",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, this: This<Object<'js>>| {
                let result = close_handle(&ctx, &close_state, &this.0)
                    .map(|()| Value::new_undefined(ctx.clone()));
                promise(ctx.clone(), result)
            },
        )?,
    )?;
    let stream_state = state.clone();
    object.set(
        "createReadStream",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, options: Opt<Value<'js>>| {
                let descriptor = handle_descriptor(&stream_state).ok_or_else(|| {
                    Exception::throw_message(&ctx, "EBADF: file descriptor closed")
                })?;
                create_read_stream(&ctx, None, Some(descriptor), options.0)
            },
        )?,
    )?;
    let stream_state = state.clone();
    object.set(
        "createWriteStream",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, options: Opt<Value<'js>>| {
                let descriptor = handle_descriptor(&stream_state).ok_or_else(|| {
                    Exception::throw_message(&ctx, "EBADF: file descriptor closed")
                })?;
                create_write_stream(&ctx, None, Some(descriptor), options.0)
            },
        )?,
    )?;
    let web_stream_state = state.clone();
    object.set(
        "readableWebStream",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, _options: Opt<Value<'js>>| {
                let descriptor = handle_descriptor(&web_stream_state).ok_or_else(|| {
                    Exception::throw_message(&ctx, "EBADF: file descriptor closed")
                })?;
                readable_web_stream(&ctx, descriptor)
            },
        )?,
    )?;
    Ok(object)
}

pub(super) fn handle_descriptor(state: &FileHandleState) -> Option<u32> {
    *lock(&state.descriptor)
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
        let descriptor = handle_descriptor(&state)
            .ok_or_else(|| Exception::throw_message(&ctx, "EBADF: file descriptor closed"))?;
        vfs_call(&ctx, |vfs| vfs.fstat(descriptor))?;
        promise(ctx.clone(), Ok(Value::new_undefined(ctx)))
    })
}

pub(super) fn handle_read<'js>(
    ctx: &Ctx<'js>,
    state: &FileHandleState,
    args: Rest<Value<'js>>,
) -> rquickjs::Result<Promise<'js>> {
    let descriptor = handle_descriptor(state)
        .ok_or_else(|| Exception::throw_message(ctx, "EBADF: file descriptor closed"))?;
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
    let result = Object::new(ctx.clone())?;
    result.set("bytesRead", bytes.len())?;
    result.set("buffer", buffer)?;
    promise(ctx.clone(), Ok(result.into_value()))
}

pub(super) fn handle_write<'js>(
    ctx: &Ctx<'js>,
    state: &FileHandleState,
    args: Rest<Value<'js>>,
) -> rquickjs::Result<Promise<'js>> {
    let descriptor = handle_descriptor(state)
        .ok_or_else(|| Exception::throw_message(ctx, "EBADF: file descriptor closed"))?;
    let mut args = args.0;
    if args.is_empty() {
        return Err(Exception::throw_type(ctx, "data is required"));
    }
    let value = args.remove(0);
    let args = normalize_write_options(ctx, value.is_string(), args)?;
    let (data, position) = write_arguments(ctx, value.clone(), args)?;
    let bytes_written = vfs_call(ctx, |vfs| vfs.write(descriptor, &data, position))?;
    let result = Object::new(ctx.clone())?;
    result.set("bytesWritten", bytes_written)?;
    result.set("buffer", write_buffer_value(ctx, &value, &data)?)?;
    promise(ctx.clone(), Ok(result.into_value()))
}

pub(super) fn create_read_stream_export<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Object<'js>> {
    create_read_stream(&ctx, Some(input), None, options.0)
}

pub(super) fn create_write_stream_export<'js>(
    ctx: Ctx<'js>,
    input: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Object<'js>> {
    create_write_stream(&ctx, Some(input), None, options.0)
}

pub(super) fn create_read_stream<'js>(
    ctx: &Ctx<'js>,
    input: Option<Value<'js>>,
    descriptor: Option<u32>,
    options: Option<Value<'js>>,
) -> rquickjs::Result<Object<'js>> {
    let path = input
        .as_ref()
        .map(|value| path(ctx, value.clone()))
        .transpose()?;
    let option_descriptor = option_descriptor(ctx, options.as_ref())?;
    let owns_descriptor = descriptor.is_none() && option_descriptor.is_none();
    let descriptor = if let Some(descriptor) = descriptor.or(option_descriptor) {
        descriptor
    } else {
        let path = path
            .as_deref()
            .ok_or_else(|| Exception::throw_type(ctx, "stream path is required"))?;
        let flags = option_property(ctx, options.as_ref(), "flags")?;
        let options = open_options(ctx, flags)?;
        vfs_call(ctx, |vfs| vfs.open(path, options))?
    };
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

pub(super) fn create_write_stream<'js>(
    ctx: &Ctx<'js>,
    input: Option<Value<'js>>,
    descriptor: Option<u32>,
    options: Option<Value<'js>>,
) -> rquickjs::Result<Object<'js>> {
    let path = input
        .as_ref()
        .map(|value| path(ctx, value.clone()))
        .transpose()?;
    let option_descriptor = option_descriptor(ctx, options.as_ref())?;
    let owns_descriptor = descriptor.is_none() && option_descriptor.is_none();
    let descriptor = if let Some(descriptor) = descriptor.or(option_descriptor) {
        descriptor
    } else {
        let path = path
            .as_deref()
            .ok_or_else(|| Exception::throw_type(ctx, "stream path is required"))?;
        let flags = option_property(ctx, options.as_ref(), "flags")?;
        let flags = flags.or_else(|| "w".into_js(ctx).ok());
        let options = open_options(ctx, flags)?;
        vfs_call(ctx, |vfs| vfs.open(path, options))?
    };
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

pub(super) fn option_descriptor<'js>(
    ctx: &Ctx<'js>,
    options: Option<&Value<'js>>,
) -> rquickjs::Result<Option<u32>> {
    let Some(value) = option_property(ctx, options, "fd")? else {
        return Ok(None);
    };
    if value.is_null() || value.is_undefined() {
        return Ok(None);
    }
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
    let descriptor = handle_descriptor(state)
        .ok_or_else(|| Exception::throw_message(ctx, "EBADF: file descriptor closed"))?;
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
    for (name, method) in [
        ("isFile", stats_is_file as fn(This<Object<'_>>) -> bool),
        ("isDirectory", stats_is_directory),
        ("isBlockDevice", stats_is_block_device),
        ("isCharacterDevice", stats_is_character_device),
        ("isSymbolicLink", stats_is_symbolic_link),
        ("isFIFO", stats_is_fifo),
        ("isSocket", stats_is_socket),
    ] {
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

pub(super) fn stats_is_file(this: This<Object<'_>>) -> bool {
    stats_type(&this).is_some_and(|mode| mode == 0o100_000)
}

pub(super) fn stats_is_directory(this: This<Object<'_>>) -> bool {
    stats_type(&this).is_some_and(|mode| mode == 0o040_000)
}

pub(super) fn stats_is_block_device(this: This<Object<'_>>) -> bool {
    stats_type(&this).is_some_and(|mode| mode == 0o060_000)
}

pub(super) fn stats_is_character_device(this: This<Object<'_>>) -> bool {
    stats_type(&this).is_some_and(|mode| mode == 0o020_000)
}

pub(super) fn stats_is_symbolic_link(this: This<Object<'_>>) -> bool {
    stats_type(&this).is_some_and(|mode| mode == 0o120_000)
}

pub(super) fn stats_is_fifo(this: This<Object<'_>>) -> bool {
    stats_type(&this).is_some_and(|mode| mode == 0o010_000)
}

pub(super) fn stats_is_socket(this: This<Object<'_>>) -> bool {
    stats_type(&this).is_some_and(|mode| mode == 0o140_000)
}

fn dirent_device(this: &This<Object<'_>>) -> bool {
    matches!(this.0.get::<_, Option<bool>>("device"), Ok(Some(true)))
}

pub(super) fn dirent_is_file(this: This<Object<'_>>) -> bool {
    !dirent_device(&this)
        && this
            .0
            .get::<_, Option<String>>("type")
            .ok()
            .flatten()
            .is_some_and(|kind| kind == "file")
}

pub(super) fn dirent_is_directory(this: This<Object<'_>>) -> bool {
    this.0
        .get::<_, Option<String>>("type")
        .ok()
        .flatten()
        .is_some_and(|kind| kind == "directory")
}

pub(super) fn dirent_is_block_device(_: This<Object<'_>>) -> bool {
    false
}

pub(super) fn dirent_is_character_device(this: This<Object<'_>>) -> bool {
    dirent_device(&this)
}

pub(super) fn dirent_is_symbolic_link(this: This<Object<'_>>) -> bool {
    this.0
        .get::<_, Option<String>>("type")
        .ok()
        .flatten()
        .is_some_and(|kind| kind == "symlink")
}

pub(super) fn dirent_is_fifo(_: This<Object<'_>>) -> bool {
    false
}

pub(super) fn dirent_is_socket(_: This<Object<'_>>) -> bool {
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
    set_number_or_bigint(ctx, &object, "dev", u64::from(stat.device), bigint)?;
    set_number_or_bigint(ctx, &object, "ino", 0, bigint)?;
    set_number_or_bigint(ctx, &object, "mode", u64::from(mode), bigint)?;
    set_number_or_bigint(ctx, &object, "nlink", 1, bigint)?;
    set_number_or_bigint(ctx, &object, "uid", 0, bigint)?;
    set_number_or_bigint(ctx, &object, "gid", 0, bigint)?;
    set_number_or_bigint(ctx, &object, "rdev", 0, bigint)?;
    set_number_or_bigint(ctx, &object, "size", stat.size, bigint)?;
    set_number_or_bigint(ctx, &object, "blksize", 0, bigint)?;
    set_number_or_bigint(ctx, &object, "blocks", 0, bigint)?;
    object.set("writable", stat.writable)?;
    object.set("device", stat.device)?;
    set_number_or_bigint(ctx, &object, "atimeMs", 0, bigint)?;
    set_number_or_bigint(ctx, &object, "mtimeMs", 0, bigint)?;
    set_number_or_bigint(ctx, &object, "ctimeMs", 0, bigint)?;
    set_number_or_bigint(ctx, &object, "birthtimeMs", 0, bigint)?;
    if bigint {
        object.set("atimeNs", BigInt::from_i64(ctx.clone(), 0)?)?;
        object.set("mtimeNs", BigInt::from_i64(ctx.clone(), 0)?)?;
        object.set("ctimeNs", BigInt::from_i64(ctx.clone(), 0)?)?;
        object.set("birthtimeNs", BigInt::from_i64(ctx.clone(), 0)?)?;
    }
    let date: Object = ctx.eval("new Date(0)")?;
    object.set("atime", date.clone())?;
    object.set("mtime", date.clone())?;
    object.set("ctime", date.clone())?;
    object.set("birthtime", date)?;
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

pub(super) fn name_value<'js>(
    ctx: &Ctx<'js>,
    name: &str,
    encoding: Option<&str>,
) -> rquickjs::Result<Value<'js>> {
    match encoding.map(str::to_ascii_lowercase).as_deref() {
        None | Some("utf8" | "utf-8") => name.to_owned().into_js(ctx),
        Some("buffer") => {
            Ok(TypedArray::<u8>::new_copy(ctx.clone(), name.as_bytes())?.into_value())
        }
        Some(_) => output(ctx.clone(), name.as_bytes(), encoding),
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
    _options: &FsOptions,
) -> rquickjs::Result<Vec<(String, DirectoryEntry)>> {
    let entries = vfs_call(ctx, |vfs| vfs.walk(cwd))?;
    let patterns = expand_braces(pattern);
    let mut result = Vec::new();
    for (path, entry) in entries {
        let candidate = if pattern.starts_with('/') {
            path.clone()
        } else {
            path.strip_prefix(cwd)
                .unwrap_or(&path)
                .trim_start_matches('/')
                .to_owned()
        };
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
    let is_file = kind == NodeType::File && !device;
    let is_directory = kind == NodeType::Directory;
    let is_symlink = kind == NodeType::Symlink;
    object.set("isFile", Function::new(ctx.clone(), move || is_file)?)?;
    object.set(
        "isDirectory",
        Function::new(ctx.clone(), move || is_directory)?,
    )?;
    object.set(
        "isSymbolicLink",
        Function::new(ctx.clone(), move || is_symlink)?,
    )?;
    object.set(
        "isCharacterDevice",
        Function::new(ctx.clone(), move || device)?,
    )?;
    object.set("isBlockDevice", Function::new(ctx.clone(), || false)?)?;
    object.set("isFIFO", Function::new(ctx.clone(), || false)?)?;
    object.set("isSocket", Function::new(ctx.clone(), || false)?)?;
    Ok(())
}
