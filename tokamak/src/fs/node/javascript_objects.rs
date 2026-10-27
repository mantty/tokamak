#![allow(clippy::wildcard_imports)]

use super::*;

/// workerd's `Dirent` class, which holds an entry's libuv `UV_DIRENT_*` type
/// under a private symbol.
const DIRENT_CLASS: &str = r#"(() => {
  const kType = Symbol("type");
  return class Dirent {
    name;
    parentPath;
    [kType];
    constructor(name, type, path) {
      this.name = name;
      this.parentPath = path;
      this[kType] = type;
    }
    isDirectory() { return this[kType] === 2; }
    isFile() { return this[kType] === 1; }
    isBlockDevice() { return this[kType] === 7; }
    isCharacterDevice() { return this[kType] === 6; }
    isSymbolicLink() { return this[kType] === 3; }
    isFIFO() { return this[kType] === 4; }
    isSocket() { return this[kType] === 5; }
  };
})()"#;

pub(super) fn dirent_constructor<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Constructor<'js>> {
    cached(ctx, "__tokamak_node_fs_dirent", |ctx| {
        ctx.eval(DIRENT_CLASS)
    })
}

/// workerd's `Dir` class, made from `Buffer` and `Dirent`, which reads the
/// `{ name, type, parentPath }` handles it is given.
const DIR_CLASS: &str = r#"(Buffer, Dirent) => {
  const coded = (Type, code, message) => Object.assign(new Type(message), { code });
  const closed = () => coded(Error, "ERR_DIR_CLOSED", "Directory is closed");
  return class Dir {
    #handle;
    #encoding;
    #path;
    constructor(handle, path, options) {
      if (handle == null) throw coded(TypeError, "ERR_MISSING_ARGS", 'The "handle" argument must be specified');
      this.#handle = handle;
      this.#path = path;
      if (options === null || typeof options !== "object" || Array.isArray(options)) {
        throw coded(TypeError, "ERR_INVALID_ARG_TYPE", 'The "options" argument must be of type object');
      }
      const { encoding = "utf8" } = options;
      if (typeof encoding !== "string") {
        throw coded(TypeError, "ERR_INVALID_ARG_TYPE", 'The "options.encoding" property must be of type string');
      }
      this.#encoding = encoding;
    }
    get path() { return this.#path; }
    read(callback) {
      if (typeof callback === "function") {
        try {
          const entry = this.readSync();
          queueMicrotask(() => callback(null, entry));
        } catch (error) {
          queueMicrotask(() => callback(error, null));
        }
        return;
      }
      try { return Promise.resolve(this.readSync()); }
      catch (error) { return Promise.reject(error); }
    }
    readSync() {
      if (this.#handle === undefined) throw closed();
      const entry = this.#handle.shift();
      if (entry == null) return null;
      const name = Buffer.from(entry.name);
      return new Dirent(this.#encoding === "buffer" ? name : name.toString(this.#encoding), entry.type, entry.parentPath);
    }
    close(callback) {
      this.closeSync();
      if (typeof callback === "function") {
        queueMicrotask(() => callback(null));
        return undefined;
      }
      return Promise.resolve();
    }
    closeSync() {
      if (this.#handle === undefined) throw closed();
      this.#handle = undefined;
    }
    async *entries() {
      for (;;) {
        const entry = await this.read();
        if (entry == null) return;
        yield entry;
      }
    }
    [Symbol.asyncIterator]() { return this.entries(); }
    async [Symbol.asyncDispose]() { await this.close(); }
    [Symbol.dispose]() { this.closeSync(); }
  };
}"#;

pub(super) fn dir_constructor<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Constructor<'js>> {
    cached(ctx, "__tokamak_node_fs_dir", |ctx| {
        let class: Function = ctx.eval(DIR_CLASS)?;
        class.call((buffer_class(ctx)?, dirent_constructor(ctx)?))
    })
}

/// The value `create` makes, made once per context and kept in the hidden
/// global `name`.
pub(super) fn cached<'js, T: FromJs<'js> + IntoJs<'js> + Clone>(
    ctx: &Ctx<'js>,
    name: &str,
    create: impl FnOnce(&Ctx<'js>) -> rquickjs::Result<T>,
) -> rquickjs::Result<T> {
    if let Some(value) = ctx.globals().get::<_, Option<T>>(name)? {
        return Ok(value);
    }
    let value = create(ctx)?;
    ctx.globals().set(name, value.clone())?;
    Ok(value)
}

#[derive(Clone)]
pub(super) struct DirItem {
    pub(super) name: String,
    pub(super) parent: String,
    pub(super) entry: DirectoryEntry,
}

/// A `Dir` reading `entries`, whose `path` is the `path` argument it was
/// opened with.
pub(super) fn dir_object<'js>(
    ctx: &Ctx<'js>,
    path: Value<'js>,
    entries: Vec<DirItem>,
    encoding: Option<&str>,
) -> rquickjs::Result<Object<'js>> {
    let handles = Array::new(ctx.clone())?;
    for (index, item) in entries.iter().enumerate() {
        let handle = Object::new(ctx.clone())?;
        handle.set("name", item.name.as_str())?;
        handle.set("type", dirent_type(&item.entry))?;
        handle.set("parentPath", item.parent.as_str())?;
        handles.set(index, handle)?;
    }
    let options = Object::new(ctx.clone())?;
    if let Some(encoding) = encoding {
        options.set("encoding", encoding)?;
    }
    dir_constructor(ctx)?.construct((handles, path, options))
}

pub(super) fn item_dirent<'js>(
    ctx: &Ctx<'js>,
    item: &DirItem,
    encoding: Option<&str>,
) -> rquickjs::Result<Object<'js>> {
    let name = text_value(ctx, &item.name, encoding)?;
    dirent(ctx, name, dirent_type(&item.entry), &item.parent)
}

#[derive(Clone)]
pub(super) struct FileHandleState {
    pub(super) descriptor: Arc<Mutex<Option<u32>>>,
}

/// `FileHandle`, which extends `EventEmitter`.
pub(super) fn file_handle_constructor<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Function<'js>> {
    let event_emitter: Object =
        crate::runtime_modules::module_namespace(ctx.clone(), "tokamak:events/events.mjs")?
            .get("EventEmitter")?;
    let function = Function::new(ctx.clone(), |ctx: Ctx<'_>| -> rquickjs::Result<()> {
        Err(Exception::throw_type(&ctx, "Illegal constructor"))
    })?;
    function.set_name("FileHandle")?;
    function.set_prototype(Some(&event_emitter))?;
    let prototype = Object::new_proto(ctx.clone(), Some(&event_emitter.get("prototype")?))?;
    function.set("prototype", prototype.clone())?;
    prototype.set("constructor", function.clone())?;
    ctx.globals()
        .set("__tokamak_node_fs_file_handle_proto", prototype)?;
    Ok(function)
}

#[allow(clippy::too_many_lines)]
pub(super) fn file_handle_object<'js>(
    ctx: &Ctx<'js>,
    descriptor: u32,
) -> rquickjs::Result<Object<'js>> {
    let prototype: Option<Object> = ctx.globals().get("__tokamak_node_fs_file_handle_proto")?;
    let object = Object::new_proto(ctx.clone(), prototype.as_ref())?;
    object.set("fd", descriptor)?;
    let state = FileHandleState {
        descriptor: Arc::new(Mutex::new(Some(descriptor))),
    };

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
            move |ctx: Ctx<'js>, buffers: Value<'js>, position: Opt<Value<'js>>| {
                let read = open_descriptor(&ctx, &vector_read_state).and_then(|descriptor| {
                    let count =
                        read_views(&ctx, descriptor, &vector_read(&ctx, &buffers, position.0)?)?;
                    transfer_value(&ctx, "bytesRead", count as usize, "buffers", buffers)
                });
                promise(ctx.clone(), read)
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
                Err::<(), rquickjs::Error>(crate::errors::throw_message(
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
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
            let mut args = args.0.into_iter();
            let buffers = args
                .next()
                .unwrap_or_else(|| Value::new_undefined(ctx.clone()));
            handle_write_promise(&ctx, &vector_write_state, "buffers", || {
                vector_write(&ctx, buffers, defined(args.next()))
            })
        })?,
    )?;
    let write_file_state = state.clone();
    object.set(
        "writeFile",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, data: Value<'js>, options: Opt<Value<'js>>| {
                let written = open_descriptor(&ctx, &write_file_state).and_then(|descriptor| {
                    let count = write_all(
                        &ctx,
                        descriptor.into_js(&ctx)?,
                        data.clone(),
                        options.0.clone(),
                    )?;
                    // workerd reports a string as `Buffer.from(data, options)`
                    // encodes it.
                    let buffer = if is_array_buffer_view(&data) {
                        data
                    } else {
                        let options = options
                            .0
                            .unwrap_or_else(|| Value::new_undefined(ctx.clone()));
                        encoded(&ctx, data, options)?
                    };
                    transfer_value(&ctx, "bytesWritten", count, "buffer", buffer)
                });
                promise(ctx.clone(), written)
            },
        )?,
    )?;
    let append_state = state.clone();
    object.set(
        "appendFile",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, data: Value<'js>, options: Opt<Value<'js>>| {
                let appended = open_descriptor(&ctx, &append_state).and_then(|descriptor| {
                    append_file_sync(ctx.clone(), descriptor.into_js(&ctx)?, data, options)
                });
                promise_unit(ctx, appended.map(drop))
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
                let stat = vfs_call(&ctx, "fstat", |vfs| vfs.fstat(descriptor))?;
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
            vfs_call(&ctx, "ftruncate", |vfs| {
                vfs.ftruncate(descriptor, length.0.unwrap_or(0))
            })?;
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
    let streams = streams_module(ctx)?;
    let stream_methods: Object = streams.get("fileHandleMethods")?;
    for name in ["createReadStream", "createWriteStream"] {
        object.set(name, stream_methods.get::<_, Function>(name)?)?;
    }
    object.set(streams.get::<_, Value>("kFileHandle")?, true)?;
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

/// The namespace of the module of workerd's `ReadStream` and `WriteStream`.
pub(super) fn streams_module<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
    cached(ctx, "__tokamak_node_fs_streams", |ctx| {
        crate::runtime_modules::module_namespace(ctx.clone(), "tokamak:fs/node/streams.mjs")
    })
}

pub(super) fn handle_descriptor(state: &FileHandleState) -> Option<u32> {
    *lock(&state.descriptor)
}

/// The handle's descriptor, or an `EBADF` error once it is closed.
fn open_descriptor(ctx: &Ctx<'_>, state: &FileHandleState) -> rquickjs::Result<u32> {
    handle_descriptor(state)
        .ok_or_else(|| crate::errors::throw_message(ctx, "EBADF: file descriptor closed"))
}

/// `{ [count_name]: count, [buffer_name]: buffer }`.
fn transfer_value<'js>(
    ctx: &Ctx<'js>,
    count_name: &str,
    count: usize,
    buffer_name: &str,
    buffer: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    let result = Object::new(ctx.clone())?;
    result.set(count_name, count)?;
    result.set(buffer_name, buffer)?;
    Ok(result.into_value())
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
        close_descriptor(ctx, descriptor)?;
        object.set("fd", Value::new_undefined(ctx.clone()))?;
        if let Ok(Some(emit)) = object.get::<_, Option<Function>>("emit") {
            emit.call::<_, bool>((This(object.clone()), "close"))?;
        }
    }
    Ok(())
}

pub(super) fn file_handle_sync<'js>(
    ctx: Ctx<'js>,
    state: FileHandleState,
) -> rquickjs::Result<Function<'js>> {
    Function::new(ctx, move |ctx: Ctx<'js>| {
        require_descriptor(&ctx, open_descriptor(&ctx, &state)?, "fstat")?;
        promise_unit(ctx, Ok(()))
    })
}

pub(super) fn handle_read<'js>(
    ctx: &Ctx<'js>,
    state: &FileHandleState,
    args: Rest<Value<'js>>,
) -> rquickjs::Result<Promise<'js>> {
    let read = open_descriptor(ctx, state).and_then(|descriptor| {
        let read = file_handle_read(ctx, &args.0)?;
        let count = read_views(ctx, descriptor, &read.request(ctx)?)?;
        transfer_value(ctx, "bytesRead", count as usize, "buffer", read.buffer)
    });
    promise(ctx.clone(), read)
}

pub(super) fn handle_write<'js>(
    ctx: &Ctx<'js>,
    state: &FileHandleState,
    args: Rest<Value<'js>>,
) -> rquickjs::Result<Promise<'js>> {
    let mut args = args.0.into_iter();
    let value = args
        .next()
        .unwrap_or_else(|| Value::new_undefined(ctx.clone()));
    handle_write_promise(ctx, state, "buffer", || {
        file_handle_write(ctx, value, args.collect())
    })
}

/// A promise of `{ bytesWritten, [data_name]: data }` for the write `request`
/// returns, rejected with any error.
fn handle_write_promise<'js>(
    ctx: &Ctx<'js>,
    state: &FileHandleState,
    data_name: &str,
    request: impl FnOnce() -> rquickjs::Result<WriteRequest<'js>>,
) -> rquickjs::Result<Promise<'js>> {
    let written = open_descriptor(ctx, state).and_then(|descriptor| {
        let write = request()?;
        let count = write_views(ctx, descriptor, &write)?;
        transfer_value(ctx, "bytesWritten", count as usize, data_name, write.data)
    });
    promise(ctx.clone(), written)
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

/// workerd's `Stats` class, with a function that makes one from a stat
/// record and whether its numbers are bigints.
const STATS_CLASS: &str = r#"(() => {
  const badge = Symbol("badge");
  class Stats {
    dev;
    ino;
    mode;
    nlink;
    uid;
    gid;
    rdev;
    size;
    blksize;
    blocks;
    atimeMs;
    mtimeMs;
    ctimeMs;
    birthtimeMs;
    atimeNs;
    mtimeNs;
    ctimeNs;
    birthtimeNs;
    atime;
    mtime;
    ctime;
    birthtime;
    constructor(key, stat, options) {
      if (key !== badge) throw new TypeError("Illegal constructor");
      this.mode = 0o444;
      if (stat.writable) this.mode |= 0o222;
      if (stat.device) this.mode |= 0o20000;
      else this.mode |= { file: 0o100000, directory: 0o40000, symlink: 0o120000 }[stat.type];
      if (options.bigint) {
        this.dev = BigInt(stat.device);
        this.size = BigInt(stat.size);
        this.mode = BigInt(this.mode);
        this.atimeNs = 0n;
        this.mtimeNs = stat.lastModified;
        this.ctimeNs = stat.lastModified;
        this.birthtimeNs = stat.created;
        this.atimeMs = this.atimeNs / 1000000n;
        this.mtimeMs = this.mtimeNs / 1000000n;
        this.ctimeMs = this.ctimeNs / 1000000n;
        this.birthtimeMs = this.birthtimeNs / 1000000n;
        this.atime = new Date(Number(this.atimeMs));
        this.mtime = new Date(Number(this.mtimeMs));
        this.ctime = new Date(Number(this.ctimeMs));
        this.birthtime = new Date(Number(this.birthtimeMs));
        this.ino = 0n;
        this.nlink = 1n;
        this.uid = 0n;
        this.gid = 0n;
        this.rdev = 0n;
        this.blksize = 0n;
        this.blocks = 0n;
      } else {
        this.dev = Number(stat.device);
        this.size = stat.size;
        this.atimeMs = 0;
        this.mtimeMs = Number(stat.lastModified) / 1000000;
        this.ctimeMs = Number(stat.lastModified) / 1000000;
        this.birthtimeMs = Number(stat.created) / 1000000;
        this.atime = new Date(this.atimeMs);
        this.mtime = new Date(this.mtimeMs);
        this.ctime = new Date(this.ctimeMs);
        this.birthtime = new Date(this.birthtimeMs);
        this.ino = 0;
        this.nlink = 1;
        this.uid = 0;
        this.gid = 0;
        this.rdev = 0;
        this.blksize = 0;
        this.blocks = 0;
      }
    }
    isBlockDevice() { return (Number(this.mode) & 0o170000) === 0o60000; }
    isCharacterDevice() { return (Number(this.mode) & 0o170000) === 0o20000; }
    isDirectory() { return (Number(this.mode) & 0o170000) === 0o40000; }
    isFIFO() { return (Number(this.mode) & 0o170000) === 0o10000; }
    isFile() { return (Number(this.mode) & 0o170000) === 0o100000; }
    isSocket() { return (Number(this.mode) & 0o170000) === 0o140000; }
    isSymbolicLink() { return (Number(this.mode) & 0o170000) === 0o120000; }
  }
  return [Stats, (stat, bigint) => new Stats(badge, stat, { bigint })];
})()"#;

/// `Stats` and the function that makes one.
fn stats_class<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Array<'js>> {
    cached(ctx, "__tokamak_node_fs_stats", |ctx| ctx.eval(STATS_CLASS))
}

pub(super) fn stats_constructor<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Constructor<'js>> {
    stats_class(ctx)?.get(0)
}

/// The `Stats` workerd reports for `stat`, which has no times.
pub(super) fn stat_object<'js>(
    ctx: &Ctx<'js>,
    stat: &Stat,
    bigint: bool,
) -> rquickjs::Result<Object<'js>> {
    let record = Object::new(ctx.clone())?;
    record.set("type", node_type_name(stat.kind))?;
    record.set("size", stat.size)?;
    record.set("writable", stat.writable)?;
    record.set("device", stat.device)?;
    record.set("lastModified", BigInt::from_i64(ctx.clone(), 0)?)?;
    record.set("created", BigInt::from_i64(ctx.clone(), 0)?)?;
    let create: Function = stats_class(ctx)?.get(1)?;
    create.call((record, bigint))
}

pub(super) fn text_value<'js>(
    ctx: &Ctx<'js>,
    value: &str,
    encoding: Option<&str>,
) -> rquickjs::Result<Value<'js>> {
    match encoding.map(str::to_ascii_lowercase).as_deref() {
        None | Some("utf8" | "utf-8") => value.to_owned().into_js(ctx),
        Some("buffer") => output(ctx.clone(), value.as_bytes(), None),
        Some(_) => output(ctx.clone(), value.as_bytes(), encoding),
    }
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
    let zero = if bigint {
        BigInt::from_u64(ctx.clone(), 0)?.into_value()
    } else {
        0.into_js(ctx)?
    };
    for name in [
        "type", "bsize", "blocks", "bfree", "bavail", "files", "ffree",
    ] {
        object.set(name, zero.clone())?;
    }
    Ok(object)
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

/// A `Dirent` named `name` of the libuv `UV_DIRENT_*` type `kind`.
pub(super) fn dirent<'js>(
    ctx: &Ctx<'js>,
    name: Value<'js>,
    kind: u32,
    parent_path: &str,
) -> rquickjs::Result<Object<'js>> {
    dirent_constructor(ctx)?.construct((name, kind, parent_path))
}

/// The libuv `UV_DIRENT_*` type of `entry`.
pub(super) const fn dirent_type(entry: &DirectoryEntry) -> u32 {
    match entry.kind {
        _ if entry.device => 6,
        NodeType::File => 1,
        NodeType::Directory => 2,
        NodeType::Symlink => 3,
    }
}
