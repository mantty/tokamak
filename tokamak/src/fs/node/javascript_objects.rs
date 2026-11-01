#![allow(clippy::wildcard_imports)]

use tokamak_macros::JsType;

use super::*;

/// The namespace of the runtime module `fs/node/{name}`.
pub(super) fn fs_module<'js>(ctx: &Ctx<'js>, name: &str) -> rquickjs::Result<Object<'js>> {
    cached(ctx, &format!("__tokamak_node_fs_{name}"), |ctx| {
        crate::runtime_modules::module_namespace(ctx.clone(), &format!("tokamak:fs/node/{name}"))
    })
}

/// `fs/node/objects.ts`'s export `name`.
pub(super) fn fs_object<'js>(ctx: &Ctx<'js>, name: &str) -> rquickjs::Result<Constructor<'js>> {
    fs_module(ctx, "objects.ts")?.get(name)
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

/// An entry a `Dir` reads: its name, libuv `UV_DIRENT_*` type and parent
/// path.
#[derive(IntoJs, JsType)]
#[qjs(rename_all = "camelCase")]
pub(crate) struct DirEntryRecord {
    name: String,
    #[qjs(rename = "type")]
    kind: u32,
    parent_path: String,
}

/// What a `Stats` reports of a file.
#[derive(IntoJs, JsType)]
pub(crate) struct StatRecord {
    /// The file type and permission bits.
    mode: u32,
    size: u64,
    /// Whether the file is a synthetic device.
    device: bool,
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
    let handles: Vec<DirEntryRecord> = entries
        .iter()
        .map(|item| DirEntryRecord {
            name: item.name.clone(),
            kind: dirent_type(&item.entry),
            parent_path: item.parent.clone(),
        })
        .collect();
    let options = Object::new(ctx.clone())?;
    if let Some(encoding) = encoding {
        options.set("encoding", encoding)?;
    }
    fs_object(ctx, "Dir")?.construct((handles, path, options))
}

pub(super) fn item_dirent<'js>(
    ctx: &Ctx<'js>,
    item: &DirItem,
    encoding: Option<&str>,
) -> rquickjs::Result<Object<'js>> {
    let name = text_value(ctx, &item.name, encoding)?;
    dirent(ctx, name, dirent_type(&item.entry), &item.parent)
}

/// The `Stats` workerd reports for `stat`.
pub(super) fn stat_object<'js>(
    ctx: &Ctx<'js>,
    stat: &Stat,
    bigint: bool,
) -> rquickjs::Result<Object<'js>> {
    // `S_IFCHR`, `S_IFREG`, `S_IFDIR` and `S_IFLNK`.
    let file_type = match stat.kind {
        _ if stat.device => 0o20_000,
        NodeType::File => 0o100_000,
        NodeType::Directory => 0o40_000,
        NodeType::Symlink => 0o120_000,
    };
    let record = StatRecord {
        mode: file_type | if stat.writable { 0o666 } else { 0o444 },
        size: stat.size,
        device: stat.device,
    };
    let create: Function = fs_module(ctx, "objects.ts")?.get("stats")?;
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

/// A `Dirent` named `name` of the libuv `UV_DIRENT_*` type `kind`.
pub(super) fn dirent<'js>(
    ctx: &Ctx<'js>,
    name: Value<'js>,
    kind: u32,
    parent_path: &str,
) -> rquickjs::Result<Object<'js>> {
    fs_object(ctx, "Dirent")?.construct((name, kind, parent_path))
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
