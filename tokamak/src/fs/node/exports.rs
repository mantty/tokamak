#![deny(missing_docs)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::elidable_lifetime_names)]
#![allow(clippy::needless_pass_by_value)]

//! Native `node:fs` bindings for the tokamak `QuickJS` runtime.

#[allow(clippy::wildcard_imports)]
use super::*;

/// The native `node:fs` module name.
pub const MODULE_NAME: &str = "node:fs";
/// The native `node:fs/promises` module name.
pub const PROMISES_MODULE_NAME: &str = "node:fs/promises";

/// A request-owned VFS handle installed into a `QuickJS` context.
pub type VfsHandle = Arc<Mutex<VirtualFileSystem>>;

pub(super) struct VfsUserData(pub(super) VfsHandle);

// This userdata contains no JavaScript values, so changing the marker lifetime
// cannot change its representation or validity.
#[allow(clippy::elidable_lifetime_names)]
unsafe impl<'js> rquickjs::JsLifetime<'js> for VfsUserData {
    type Changed<'to> = VfsUserData;
}

/// Install the request VFS before any filesystem module is evaluated.
pub fn install(ctx: &Ctx<'_>, vfs: &VfsHandle) -> rquickjs::Result<()> {
    ctx.store_userdata(VfsUserData(Arc::clone(vfs)))?;
    Ok(())
}

#[allow(clippy::wildcard_imports)]
#[rquickjs::module]
pub(crate) mod node_fs_module {
    use super::*;

    #[qjs(declare)]
    pub fn declare(declare: &Declarations) -> rquickjs::Result<()> {
        declare_exports(declare, false)
    }

    #[qjs(evaluate)]
    pub fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> rquickjs::Result<()> {
        export_module(ctx, exports, false)
    }
}

#[allow(clippy::wildcard_imports)]
#[rquickjs::module]
pub(crate) mod node_fs_promises_module {
    use super::*;

    #[qjs(declare)]
    pub fn declare(declare: &Declarations) -> rquickjs::Result<()> {
        declare_exports(declare, true)
    }

    #[qjs(evaluate)]
    pub fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> rquickjs::Result<()> {
        export_module(ctx, exports, true)
    }
}

/// The native `node:fs` module definition.
pub struct NodeFsModule;

impl ModuleDef for NodeFsModule {
    fn declare(declare: &Declarations) -> rquickjs::Result<()> {
        <js_node_fs_module as ModuleDef>::declare(declare)
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> rquickjs::Result<()> {
        <js_node_fs_module as ModuleDef>::evaluate(ctx, exports)
    }
}

/// The native `node:fs/promises` module definition.
pub struct NodeFsPromisesModule;

impl ModuleDef for NodeFsPromisesModule {
    fn declare(declare: &Declarations) -> rquickjs::Result<()> {
        <js_node_fs_promises_module as ModuleDef>::declare(declare)
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> rquickjs::Result<()> {
        <js_node_fs_promises_module as ModuleDef>::evaluate(ctx, exports)
    }
}

#[allow(clippy::too_many_lines)]
fn declare_exports(declare: &Declarations, promises_only: bool) -> rquickjs::Result<()> {
    let names = if promises_only {
        [
            "constants",
            "readFile",
            "writeFile",
            "appendFile",
            "access",
            "chmod",
            "chown",
            "cp",
            "glob",
            "lchmod",
            "lchown",
            "link",
            "lutimes",
            "mkdir",
            "mkdtemp",
            "open",
            "opendir",
            "readdir",
            "lstat",
            "readlink",
            "realpath",
            "rename",
            "unlink",
            "rm",
            "rmdir",
            "copyFile",
            "symlink",
            "stat",
            "statfs",
            "truncate",
            "utimes",
            "watch",
            "FileHandle",
            "default",
        ]
        .as_slice()
    } else {
        [
            "constants",
            "F_OK",
            "R_OK",
            "W_OK",
            "X_OK",
            "readFileSync",
            "writeFileSync",
            "appendFileSync",
            "accessSync",
            "chmodSync",
            "chownSync",
            "mkdirSync",
            "readdirSync",
            "statSync",
            "lstatSync",
            "existsSync",
            "unlinkSync",
            "rmSync",
            "rmdirSync",
            "renameSync",
            "copyFileSync",
            "cpSync",
            "fchmodSync",
            "fchownSync",
            "fdatasyncSync",
            "fsyncSync",
            "futimesSync",
            "globSync",
            "lchmodSync",
            "lchownSync",
            "lutimesSync",
            "linkSync",
            "mkdtempSync",
            "opendirSync",
            "symlinkSync",
            "readlinkSync",
            "realpathSync",
            "openSync",
            "closeSync",
            "fstatSync",
            "readSync",
            "readvSync",
            "writeSync",
            "writevSync",
            "truncateSync",
            "ftruncateSync",
            "statfsSync",
            "utimesSync",
            "openAsBlob",
            "Dirent",
            "Dir",
            "Stats",
            "ReadStream",
            "WriteStream",
            "FileReadStream",
            "FileWriteStream",
            "createReadStream",
            "createWriteStream",
            "access",
            "exists",
            "appendFile",
            "chmod",
            "chown",
            "close",
            "copyFile",
            "cp",
            "fchmod",
            "fchown",
            "fdatasync",
            "fstat",
            "fsync",
            "ftruncate",
            "futimes",
            "glob",
            "lchmod",
            "lchown",
            "link",
            "lstat",
            "lutimes",
            "mkdir",
            "mkdtemp",
            "open",
            "opendir",
            "read",
            "readFile",
            "readlink",
            "readv",
            "readdir",
            "realpath",
            "rename",
            "rm",
            "rmdir",
            "stat",
            "statfs",
            "symlink",
            "truncate",
            "unlink",
            "utimes",
            "write",
            "writeFile",
            "writev",
            "unwatchFile",
            "watch",
            "watchFile",
            "promises",
            "default",
        ]
        .as_slice()
    };
    for name in names {
        declare.declare(*name)?;
    }
    Ok(())
}

fn export_module<'js>(
    ctx: &Ctx<'js>,
    exports: &Exports<'js>,
    promises_only: bool,
) -> rquickjs::Result<()> {
    let object = Object::new(ctx.clone())?;
    export_constants(ctx, exports, &object, promises_only)?;
    if promises_only {
        let file_handle = file_handle_constructor(ctx)?;
        exports.export("FileHandle", file_handle.clone())?;
        object.set("FileHandle", file_handle)?;
        export_promises(ctx, exports, &object)?;
    } else {
        let dirent = dirent_constructor(ctx)?;
        let dir = dir_constructor(ctx)?;
        let stats = stats_constructor(ctx)?;
        for (name, value) in [
            ("Dirent", dirent.clone()),
            ("Dir", dir.clone()),
            ("Stats", stats.clone()),
        ] {
            exports.export(name, value.clone())?;
            object.set(name, value)?;
        }

        export_sync(ctx, exports, &object)?;
        let promises: Object = rquickjs::Module::import(ctx, PROMISES_MODULE_NAME)?
            .finish::<Object>()?
            .get("default")?;
        exports.export("promises", promises.clone())?;
        object.set("promises", promises)?;
        export_streams(ctx, exports, &object)?;
        export_callback(ctx, exports, &object)?;
    }
    exports.export("default", object).map(|_| ())
}

fn export_constants<'js>(
    ctx: &Ctx<'js>,
    exports: &Exports<'js>,
    object: &Object<'js>,
    promises_only: bool,
) -> rquickjs::Result<()> {
    let constants = constants(ctx.clone())?;
    if !promises_only {
        for (name, value) in [
            ("F_OK", 0_i32),
            ("R_OK", 4_i32),
            ("W_OK", 2_i32),
            ("X_OK", 1_i32),
        ] {
            exports.export(name, value)?;
            object.set(name, value)?;
        }
    }
    exports.export("constants", constants.clone())?;
    object.set("constants", constants)
}

fn export_sync<'js>(
    ctx: &Ctx<'js>,
    exports: &Exports<'js>,
    object: &Object<'js>,
) -> rquickjs::Result<()> {
    export_function(ctx, exports, object, "readFileSync", read_file_sync)?;
    export_function(ctx, exports, object, "writeFileSync", write_file_sync)?;
    export_function(ctx, exports, object, "appendFileSync", append_file_sync)?;
    export_function(ctx, exports, object, "accessSync", access_sync)?;
    export_function(ctx, exports, object, "chmodSync", chmod_sync)?;
    export_function(ctx, exports, object, "chownSync", chown_sync)?;
    export_function(ctx, exports, object, "mkdirSync", mkdir_sync)?;
    export_function(ctx, exports, object, "readdirSync", readdir_sync)?;
    export_function(ctx, exports, object, "statSync", stat_sync)?;
    export_function(ctx, exports, object, "lstatSync", lstat_sync)?;
    export_function(ctx, exports, object, "existsSync", exists_sync)?;
    export_function(ctx, exports, object, "unlinkSync", unlink_sync)?;
    export_function(ctx, exports, object, "rmSync", rm_sync)?;
    export_function(ctx, exports, object, "rmdirSync", rmdir_sync)?;
    export_function(ctx, exports, object, "renameSync", rename_sync)?;
    export_function(ctx, exports, object, "copyFileSync", copy_file_sync)?;
    export_function(ctx, exports, object, "cpSync", cp_sync)?;
    export_function(ctx, exports, object, "fchmodSync", fchmod_sync)?;
    export_function(ctx, exports, object, "fchownSync", fchown_sync)?;
    export_function(ctx, exports, object, "fdatasyncSync", fdatasync_sync)?;
    export_function(ctx, exports, object, "fsyncSync", fsync_sync)?;
    export_function(ctx, exports, object, "futimesSync", futimes_sync)?;
    export_function(ctx, exports, object, "globSync", glob_sync)?;
    export_function(ctx, exports, object, "lchmodSync", lchmod_sync)?;
    export_function(ctx, exports, object, "lchownSync", lchown_sync)?;
    export_function(ctx, exports, object, "lutimesSync", lutimes_sync)?;
    export_function(ctx, exports, object, "linkSync", link_sync)?;
    export_function(ctx, exports, object, "mkdtempSync", mkdtemp_sync)?;
    export_function(ctx, exports, object, "opendirSync", opendir_sync)?;
    export_function(ctx, exports, object, "symlinkSync", symlink_sync)?;
    export_function(ctx, exports, object, "readlinkSync", read_link_sync)?;
    export_function(ctx, exports, object, "realpathSync", realpath_sync)?;
    export_function(ctx, exports, object, "openSync", open_sync)?;
    export_function(ctx, exports, object, "closeSync", close_sync)?;
    export_function(ctx, exports, object, "fstatSync", fstat_sync)?;
    export_function(ctx, exports, object, "readSync", read_sync_export)?;
    export_function(ctx, exports, object, "readvSync", readv_sync)?;
    export_function(ctx, exports, object, "writeSync", write_sync_export)?;
    export_function(ctx, exports, object, "writevSync", writev_sync)?;
    export_function(ctx, exports, object, "truncateSync", truncate_sync)?;
    export_function(ctx, exports, object, "ftruncateSync", ftruncate_sync)?;
    export_function(ctx, exports, object, "statfsSync", statfs_sync)?;
    export_function(ctx, exports, object, "utimesSync", utimes_sync)?;
    export_function(ctx, exports, object, "openAsBlob", open_as_blob)
}

fn export_callback<'js>(
    ctx: &Ctx<'js>,
    exports: &Exports<'js>,
    object: &Object<'js>,
) -> rquickjs::Result<()> {
    for (name, operation) in callback_operations() {
        let name = *name;
        let operation = *operation;
        let function = Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
            callback_call(ctx, operation, args)
        })?;
        if name == "realpath" {
            function.set("native", function.clone())?;
        }
        exports.export(name, function.clone())?;
        object.set(name, function)?;
    }
    Ok(())
}

fn export_streams<'js>(
    ctx: &Ctx<'js>,
    exports: &Exports<'js>,
    object: &Object<'js>,
) -> rquickjs::Result<()> {
    let read_stream = stream_type_constructor(ctx, "Readable")?;
    let write_stream = stream_type_constructor(ctx, "Writable")?;
    for (name, value) in [
        ("ReadStream", read_stream.clone()),
        ("FileReadStream", read_stream),
        ("WriteStream", write_stream.clone()),
        ("FileWriteStream", write_stream),
    ] {
        exports.export(name, value.clone())?;
        object.set(name, value)?;
    }
    export_function(
        ctx,
        exports,
        object,
        "createReadStream",
        create_read_stream_export,
    )?;
    export_function(
        ctx,
        exports,
        object,
        "createWriteStream",
        create_write_stream_export,
    )?;
    export_function(ctx, exports, object, "unwatchFile", unsupported_watch)?;
    export_function(ctx, exports, object, "watch", unsupported_watch)?;
    export_function(ctx, exports, object, "watchFile", unsupported_watch)
}

fn unsupported_watch(ctx: Ctx<'_>) -> rquickjs::Result<()> {
    Err(Exception::throw_internal(
        &ctx,
        "filesystem watching is not available in the Tokamak runtime",
    ))
}

fn stream_type_constructor<'js>(ctx: &Ctx<'js>, name: &str) -> rquickjs::Result<Constructor<'js>> {
    let readable = name == "Readable";
    let prototype_name = if readable {
        "__tokamak_node_fs_read_stream_proto"
    } else {
        "__tokamak_node_fs_write_stream_proto"
    };
    let prototype = Object::new(ctx.clone())?;
    ctx.globals().set(prototype_name, prototype.clone())?;
    let prototype_name = prototype_name.to_owned();
    Constructor::new_prototype(
        ctx,
        prototype,
        move |ctx: Ctx<'js>, input: Opt<Value<'js>>, options: Opt<Value<'js>>| {
            let stream = if readable {
                create_read_stream(&ctx, input.0, None, options.0)
            } else {
                create_write_stream(&ctx, input.0, None, options.0)
            }?;
            if let Some(base_prototype) = stream.get_prototype() {
                let prototype: Object = ctx.globals().get(&prototype_name)?;
                prototype.set_prototype(Some(&base_prototype))?;
            }
            Ok::<Object<'js>, rquickjs::Error>(stream)
        },
    )
}

fn export_promises<'js>(
    ctx: &Ctx<'js>,
    exports: &Exports<'js>,
    object: &Object<'js>,
) -> rquickjs::Result<()> {
    for (name, function) in [
        ("readFile", Function::new(ctx.clone(), read_file_promise)?),
        ("writeFile", Function::new(ctx.clone(), write_file_promise)?),
        (
            "appendFile",
            Function::new(ctx.clone(), append_file_promise)?,
        ),
        ("access", Function::new(ctx.clone(), access_promise)?),
        ("chmod", Function::new(ctx.clone(), chmod_promise)?),
        ("chown", Function::new(ctx.clone(), chown_promise)?),
        ("cp", Function::new(ctx.clone(), cp_promise)?),
        ("glob", Function::new(ctx.clone(), glob_promise)?),
        ("lchmod", Function::new(ctx.clone(), lchmod_promise)?),
        ("lchown", Function::new(ctx.clone(), lchown_promise)?),
        ("link", Function::new(ctx.clone(), link_promise)?),
        ("lutimes", Function::new(ctx.clone(), lutimes_promise)?),
        ("mkdir", Function::new(ctx.clone(), mkdir_promise)?),
        ("mkdtemp", Function::new(ctx.clone(), mkdtemp_promise)?),
        ("open", Function::new(ctx.clone(), open_promise)?),
        ("opendir", Function::new(ctx.clone(), opendir_promise)?),
        ("readdir", Function::new(ctx.clone(), readdir_promise)?),
        ("stat", Function::new(ctx.clone(), stat_promise)?),
        ("lstat", Function::new(ctx.clone(), lstat_promise)?),
        ("unlink", Function::new(ctx.clone(), unlink_promise)?),
        ("rm", Function::new(ctx.clone(), rm_promise)?),
        ("rmdir", Function::new(ctx.clone(), rmdir_promise)?),
        ("rename", Function::new(ctx.clone(), rename_promise)?),
        ("copyFile", Function::new(ctx.clone(), copy_file_promise)?),
        ("symlink", Function::new(ctx.clone(), symlink_promise)?),
        ("readlink", Function::new(ctx.clone(), read_link_promise)?),
        ("realpath", Function::new(ctx.clone(), realpath_promise)?),
        ("truncate", Function::new(ctx.clone(), truncate_promise)?),
        ("statfs", Function::new(ctx.clone(), statfs_promise)?),
        ("utimes", Function::new(ctx.clone(), utimes_promise)?),
        ("watch", Function::new(ctx.clone(), unsupported_watch)?),
    ] {
        exports.export(name, function.clone())?;
        object.set(name, function)?;
    }
    Ok(())
}

fn export_function<'js, F, P>(
    ctx: &Ctx<'js>,
    exports: &Exports<'js>,
    object: &Object<'js>,
    name: &str,
    function: F,
) -> rquickjs::Result<()>
where
    F: IntoJsFunc<'js, P> + Copy + 'js,
{
    let function = Function::new(ctx.clone(), function)?;
    exports.export(name, function.clone())?;
    object.set(name, function)
}

fn constants(ctx: Ctx<'_>) -> rquickjs::Result<Object<'_>> {
    let object = Object::new(ctx)?;
    for (name, value) in [
        ("UV_FS_SYMLINK_DIR", 1_i32),
        ("UV_FS_SYMLINK_JUNCTION", 2),
        ("O_RDONLY", 0),
        ("O_WRONLY", 1),
        ("O_RDWR", 2),
        ("UV_DIRENT_UNKNOWN", 0),
        ("UV_DIRENT_FILE", 1),
        ("UV_DIRENT_DIR", 2),
        ("UV_DIRENT_LINK", 3),
        ("UV_DIRENT_FIFO", 4),
        ("UV_DIRENT_SOCKET", 5),
        ("UV_DIRENT_CHAR", 6),
        ("UV_DIRENT_BLOCK", 7),
        ("EXTENSIONLESS_FORMAT_JAVASCRIPT", 0),
        ("EXTENSIONLESS_FORMAT_WASM", 1),
        ("S_IFMT", 61_440),
        ("S_IFREG", 32_768),
        ("S_IFDIR", 16_384),
        ("S_IFCHR", 8_192),
        ("S_IFBLK", 24_576),
        ("S_IFIFO", 4_096),
        ("S_IFLNK", 40_960),
        ("S_IFSOCK", 49_152),
        ("O_CREAT", 64),
        ("O_EXCL", 128),
        ("UV_FS_O_FILEMAP", 0),
        ("O_NOCTTY", 256),
        ("O_TRUNC", 512),
        ("O_APPEND", 1_024),
        ("O_DIRECTORY", 65_536),
        ("O_NOATIME", 262_144),
        ("O_NOFOLLOW", 131_072),
        ("O_SYNC", 1_052_672),
        ("O_DSYNC", 4_096),
        ("O_DIRECT", 16_384),
        ("O_NONBLOCK", 2_048),
        ("S_IRWXU", 448),
        ("S_IRUSR", 256),
        ("S_IWUSR", 128),
        ("S_IXUSR", 64),
        ("S_IRWXG", 56),
        ("S_IRGRP", 32),
        ("S_IWGRP", 16),
        ("S_IXGRP", 8),
        ("S_IRWXO", 7),
        ("S_IROTH", 4),
        ("S_IWOTH", 2),
        ("S_IXOTH", 1),
        ("F_OK", 0),
        ("R_OK", 4),
        ("W_OK", 2),
        ("X_OK", 1),
        ("UV_FS_COPYFILE_EXCL", 1),
        ("COPYFILE_EXCL", 1),
        ("UV_FS_COPYFILE_FICLONE", 2),
        ("COPYFILE_FICLONE", 2),
        ("UV_FS_COPYFILE_FICLONE_FORCE", 4),
        ("COPYFILE_FICLONE_FORCE", 4),
    ] {
        object.set(name, value)?;
    }
    Ok(object)
}
