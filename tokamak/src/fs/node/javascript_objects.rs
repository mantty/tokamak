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

/// The namespace of the module of workerd's `ReadStream` and `WriteStream`.
pub(super) fn streams_module<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
    cached(ctx, "__tokamak_node_fs_streams", |ctx| {
        crate::runtime_modules::module_namespace(ctx.clone(), "tokamak:fs/node/streams.ts")
    })
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
    // workerd's `openAsBlob` makes its own `Blob`, whatever `globalThis.Blob` is.
    let exact_blob: Function = cached(ctx, "__tokamak_node_fs_blob", |ctx| {
        crate::runtime_modules::module_namespace(ctx.clone(), "tokamak:network/fetch.ts")?
            .get("exactBlob")
    })?;
    exact_blob.call((TypedArray::<u8>::new_copy(ctx.clone(), &bytes)?, mime_type))
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
