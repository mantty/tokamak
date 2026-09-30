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
    for &name in names {
        declare.declare(name)?;
    }
    Ok(())
}

/// A module's named exports and its default export object, set together.
struct ModuleExports<'a, 'js> {
    ctx: &'a Ctx<'js>,
    exports: &'a Exports<'js>,
    default: Object<'js>,
}

impl<'js> ModuleExports<'_, 'js> {
    fn value(&self, name: &str, value: Value<'js>) -> rquickjs::Result<()> {
        self.exports.export(name, value.clone())?;
        self.default.set(name, value)
    }

    fn function<F, P>(&self, name: &str, function: F) -> rquickjs::Result<()>
    where
        F: IntoJsFunc<'js, P> + 'js,
    {
        let function = Function::new(self.ctx.clone(), function)?;
        self.value(name, function.into_value())
    }
}

fn export_module<'js>(
    ctx: &Ctx<'js>,
    exports: &Exports<'js>,
    promises_only: bool,
) -> rquickjs::Result<()> {
    let module = ModuleExports {
        ctx,
        exports,
        default: Object::new(ctx.clone())?,
    };
    export_constants(&module, promises_only)?;
    if promises_only {
        let file_handle = file_handle_constructor(ctx)?;
        module.value("FileHandle", file_handle.into_value())?;
        export_promises(&module)?;
    } else {
        let dirent = dirent_constructor(ctx)?;
        let dir = dir_constructor(ctx)?;
        let stats = stats_constructor(ctx)?;
        for (name, value) in [("Dirent", dirent), ("Dir", dir), ("Stats", stats)] {
            module.value(name, value.into_value())?;
        }

        export_sync(&module)?;
        let promises: Object = rquickjs::Module::import(ctx, PROMISES_MODULE_NAME)?
            .finish::<Object>()?
            .get("default")?;
        module.value("promises", promises.into_value())?;
        export_streams(&module)?;
        export_callback(&module)?;
    }
    exports.export("default", module.default).map(|_| ())
}

fn export_constants(module: &ModuleExports<'_, '_>, promises_only: bool) -> rquickjs::Result<()> {
    let constants = constants(module.ctx.clone())?;
    if !promises_only {
        for (name, value) in [("F_OK", 0), ("R_OK", 4), ("W_OK", 2), ("X_OK", 1)] {
            module.value(name, Value::new_int(module.ctx.clone(), value))?;
        }
    }
    module.value("constants", constants.into_value())
}

/// Casting to a shared signature compiles one host wrapper per signature.
fn export_sync(module: &ModuleExports<'_, '_>) -> rquickjs::Result<()> {
    module.function("readFileSync", read_file_sync as ValueOperation)?;
    module.function("writeFileSync", write_file_sync as PairOptionsOperation)?;
    module.function("appendFileSync", append_file_sync as PairOptionsOperation)?;
    module.function("accessSync", access_sync)?;
    module.function("chmodSync", chmod_sync as PairOperation)?;
    module.function("chownSync", chown_sync as OwnerOperation)?;
    module.function("mkdirSync", mkdir_sync as OptionsOperation)?;
    module.function("readdirSync", readdir_sync as ValueOperation)?;
    module.function("statSync", stat_sync as ValueOperation)?;
    module.function("lstatSync", lstat_sync as ValueOperation)?;
    module.function("existsSync", exists_sync)?;
    module.function("unlinkSync", unlink_sync)?;
    module.function("rmSync", rm_sync as OptionsOperation)?;
    module.function("rmdirSync", rmdir_sync as OptionsOperation)?;
    module.function("renameSync", rename_sync as PairOperation)?;
    module.function("copyFileSync", copy_file_sync)?;
    module.function("cpSync", cp_sync as PairOptionsOperation)?;
    module.function("fchmodSync", fchmod_sync)?;
    module.function("fchownSync", fchown_sync)?;
    module.function("fdatasyncSync", fsync_sync)?;
    module.function("fsyncSync", fsync_sync)?;
    module.function("futimesSync", futimes_sync)?;
    module.function("globSync", glob_sync as ValueOperation)?;
    module.function("lchmodSync", lchmod_sync as PairOperation)?;
    module.function("lchownSync", lchown_sync as OwnerOperation)?;
    module.function("lutimesSync", lutimes_sync as TimesOperation)?;
    module.function("linkSync", link_sync as PairOperation)?;
    module.function("mkdtempSync", mkdtemp_sync as ValueOperation)?;
    module.function("opendirSync", opendir_sync as ValueOperation)?;
    module.function("symlinkSync", symlink_sync as PairOperation)?;
    module.function("readlinkSync", read_link_sync as ValueOperation)?;
    module.function("realpathSync", realpath_sync as ValueOperation)?;
    module.function("openSync", open_sync)?;
    module.function("closeSync", close_sync)?;
    module.function("fstatSync", fstat_sync)?;
    module.function("readSync", read_sync_export)?;
    module.function("readvSync", readv_sync)?;
    module.function("writeSync", write_sync_export)?;
    module.function("writevSync", writev_sync)?;
    module.function("truncateSync", truncate_sync)?;
    module.function("ftruncateSync", ftruncate_sync)?;
    module.function("statfsSync", statfs_sync as ValueOperation)?;
    module.function("utimesSync", utimes_sync as TimesOperation)?;
    module.function("openAsBlob", open_as_blob as ValueOperation)
}

fn export_callback<'js>(module: &ModuleExports<'_, 'js>) -> rquickjs::Result<()> {
    for &(name, operation) in callback_operations() {
        let function = Function::new(
            module.ctx.clone(),
            move |ctx: Ctx<'js>, args: Rest<Value<'js>>| callback_call(ctx, operation, args),
        )?;
        if name == "realpath" {
            function.set("native", function.clone())?;
        }
        module.value(name, function.into_value())?;
    }
    Ok(())
}

fn export_streams(module: &ModuleExports<'_, '_>) -> rquickjs::Result<()> {
    let read_stream = stream_type_constructor(module.ctx, "Readable")?;
    let write_stream = stream_type_constructor(module.ctx, "Writable")?;
    for (name, value) in [
        ("ReadStream", read_stream.clone()),
        ("FileReadStream", read_stream),
        ("WriteStream", write_stream.clone()),
        ("FileWriteStream", write_stream),
    ] {
        module.value(name, value.into_value())?;
    }
    for (name, readable) in [("createReadStream", true), ("createWriteStream", false)] {
        module.value(name, stream_function(module.ctx, readable)?.into_value())?;
    }
    module.function("unwatchFile", unsupported_watch)?;
    module.function("watch", unsupported_watch)?;
    module.function("watchFile", unsupported_watch)
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
            let stream = create_stream(&ctx, readable, input.0, None, options.0)?;
            if let Some(base_prototype) = stream.get_prototype() {
                let prototype: Object = ctx.globals().get(&prototype_name)?;
                prototype.set_prototype(Some(&base_prototype))?;
            }
            Ok::<Object<'js>, rquickjs::Error>(stream)
        },
    )
}

fn export_promises(module: &ModuleExports<'_, '_>) -> rquickjs::Result<()> {
    let ctx = module.ctx;
    for (name, function) in [
        ("readFile", value_promise(ctx, read_file_sync)?),
        ("writeFile", pair_options_promise(ctx, write_file_sync)?),
        ("appendFile", pair_options_promise(ctx, append_file_sync)?),
        ("access", Function::new(ctx.clone(), access_promise)?),
        ("chmod", pair_promise(ctx, chmod_sync)?),
        ("chown", owner_promise(ctx, chown_sync)?),
        ("cp", pair_options_promise(ctx, cp_sync)?),
        ("glob", Function::new(ctx.clone(), glob_promise)?),
        ("lchmod", pair_promise(ctx, lchmod_sync)?),
        ("lchown", owner_promise(ctx, lchown_sync)?),
        ("link", pair_promise(ctx, link_sync)?),
        ("lutimes", times_promise(ctx, lutimes_sync)?),
        ("mkdir", options_promise(ctx, mkdir_sync)?),
        ("mkdtemp", value_promise(ctx, mkdtemp_sync)?),
        ("open", Function::new(ctx.clone(), open_promise)?),
        ("opendir", value_promise(ctx, opendir_sync)?),
        ("readdir", value_promise(ctx, readdir_sync)?),
        ("stat", value_promise(ctx, stat_sync)?),
        ("lstat", value_promise(ctx, lstat_sync)?),
        ("unlink", Function::new(ctx.clone(), unlink_promise)?),
        ("rm", options_promise(ctx, rm_sync)?),
        ("rmdir", options_promise(ctx, rmdir_sync)?),
        ("rename", pair_promise(ctx, rename_sync)?),
        ("copyFile", Function::new(ctx.clone(), copy_file_promise)?),
        ("symlink", pair_promise(ctx, symlink_sync)?),
        ("readlink", value_promise(ctx, read_link_sync)?),
        ("realpath", value_promise(ctx, realpath_sync)?),
        ("truncate", Function::new(ctx.clone(), truncate_promise)?),
        ("statfs", value_promise(ctx, statfs_sync)?),
        ("utimes", times_promise(ctx, utimes_sync)?),
        ("watch", Function::new(ctx.clone(), unsupported_watch)?),
    ] {
        module.value(name, function.into_value())?;
    }
    Ok(())
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
