use super::{MODULE_NAME, PROMISES_MODULE_NAME, install};
use crate::fs::vfs::{Bundle, VirtualFileSystem};
use rquickjs::{Context, Ctx, Module, Object, Promise, Runtime};
use std::path::Path;
use std::sync::{Arc, Mutex};

fn node_fs_runtime(root: &Path) -> rquickjs::Result<Runtime> {
    let runtime = Runtime::new()?;
    crate::dispatcher::configure_worker_loader(
        &runtime,
        &crate::quickjs::WorkerBundle::from_modules("entry.js", root, root),
    );
    Ok(runtime)
}

fn install_node_fs(ctx: &Ctx<'_>, root: &Path) -> rquickjs::Result<()> {
    let vfs = Arc::new(Mutex::new(VirtualFileSystem::new(Bundle::new(root))));
    install(ctx, &vfs)?;
    crate::compat::initialize(ctx)
}

#[test]
#[allow(clippy::too_many_lines)]
fn exposes_the_native_node_fs_surface() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    std::fs::create_dir_all(directory.path().join("config"))?;
    std::fs::write(
        directory.path().join("config/app.json"),
        br#"{"enabled":true}"#,
    )?;
    let runtime = node_fs_runtime(directory.path())?;
    let context = Context::full(&runtime)?;
    context.with(|ctx| -> Result<(), Box<dyn std::error::Error>> {
            install_node_fs(&ctx, directory.path())?;
            let namespace: Object = Module::import(&ctx, MODULE_NAME)?.finish()?;
            let default: Object = namespace.get("default")?;
            assert!(default.get::<_, rquickjs::Function>("readFileSync").is_ok());
            assert!(namespace.get::<_, Object>("promises").is_ok());
            let promises_namespace: Object =
                Module::import(&ctx, PROMISES_MODULE_NAME)?.finish()?;
            assert!(promises_namespace
                .get::<_, rquickjs::Function>("readFile")
                .is_ok());
            assert!(promises_namespace.get::<_, Object>("default").is_ok());
            let result: String = ctx.eval(
                r#"(() => {
                  const fs = process.getBuiltinModule("node:fs");
                  fs.mkdirSync("/tmp/data", { recursive: true });
                  fs.writeFileSync("/tmp/data/value.txt", "hello");
                  fs.appendFileSync("/tmp/data/value.txt", " world");
                  const fd = fs.openSync("/tmp/data/value.bin", "w+");
                  fs.writeSync(fd, Uint8Array.from([1, 2, 3]));
                  const bytes = new Uint8Array(3);
                  const count = fs.readSync(fd, bytes, 0, bytes.length, 0);
                  fs.closeSync(fd);
                  const entries = fs.readdirSync("/tmp/data", { withFileTypes: true });
                  fs.symlinkSync("/tmp/data/value.txt", "/tmp/data/link.txt");
                  const linkTarget = fs.readlinkSync("/tmp/data/link.txt");
                  const linkPath = fs.realpathSync("/tmp/data/link.txt");
                  fs.copyFileSync("/tmp/data/value.txt", "/tmp/data/copy.txt");
                  fs.renameSync("/tmp/data/copy.txt", "/tmp/data/renamed.txt");
                  const renamed = fs.readFileSync("/tmp/data/renamed.txt", "utf8");
                  fs.unlinkSync("/tmp/data/renamed.txt");
                  const append = fs.openSync("/tmp/data/value.txt", "a");
                  let appendReadCode;
                  try { fs.readSync(append, new Uint8Array(1), 0, 1, 0); }
                  catch (error) { appendReadCode = error.code; }
                  fs.closeSync(append);
                  const zero = fs.openSync("/dev/zero", "r");
                  const zeros = new Uint8Array(4);
                  fs.readSync(zero, zeros, 0, zeros.length, 0);
                  fs.closeSync(zero);
                  let readonlyCode;
                  const bundle = fs.openSync("/bundle/config/app.json", "r");
                  try { fs.ftruncateSync(bundle, 1); } catch (error) { readonlyCode = error.code; }
                  fs.closeSync(bundle);
                  return JSON.stringify({
                    text: fs.readFileSync("/tmp/data/value.txt", "utf8"),
                    count,
                    bytes: [...bytes],
                    file: fs.statSync("/tmp/data/value.txt").isFile(),
                    entry: entries[0].isFile(),
                    linkTarget,
                    linkPath,
                    renamed,
                    appendReadCode,
                    zeros: [...zeros],
                    readonlyCode,
                    bundle: fs.readFileSync("/bundle/config/app.json", "utf8"),
                  });
                })()"#,
            )?;
            assert_eq!(
                result,
                r#"{"text":"hello world","count":3,"bytes":[1,2,3],"file":true,"entry":true,"linkTarget":"/tmp/data/value.txt","linkPath":"/tmp/data/value.txt","renamed":"hello world","appendReadCode":"EPERM","zeros":[0,0,0,0],"readonlyCode":"EPERM","bundle":"{\"enabled\":true}"}"#
            );

            let promise: Promise = ctx.eval(
                r#"(async () => {
                  const fs = process.getBuiltinModule("node:fs");
                  let callbackText;
                  await new Promise((resolve, reject) => fs.writeFile(
                    "/tmp/callback.txt",
                    "callback",
                    error => error ? reject(error) : resolve(),
                  ));
                  await new Promise((resolve, reject) => fs.readFile(
                    "/tmp/callback.txt",
                    "utf8",
                    (error, value) => error ? reject(error) : (callbackText = value, resolve()),
                  ));
                  await fs.promises.writeFile("/tmp/promise.txt", "done");
                  const promiseText = await fs.promises.readFile("/tmp/promise.txt", "utf8");
                  const imported = await import("node:fs/promises");
                  await imported.writeFile("/tmp/imported.txt", "imported");
                  const importedText = await imported.readFile("/tmp/imported.txt", "utf8");
                  return JSON.stringify({ callbackText, promiseText, importedText });
                })()"#,
            )?;
            assert_eq!(
                promise.finish::<String>()?,
                r#"{"callbackText":"callback","promiseText":"done","importedText":"imported"}"#
            );
            let realpath_shape: String = ctx.eval(
                r#"(() => { const fs = process.getBuiltinModule("node:fs"); return `${typeof fs.realpath}:${typeof fs.realpath?.native}`; })()"#,
            )?;
            assert_eq!(realpath_shape, "function:function");

            let advanced: Promise = ctx.eval(
                r#"(async () => {
                  const fs = process.getBuiltinModule("node:fs");
                  fs.accessSync("/bundle/config/app.json", fs.R_OK);
                  let accessCode;
                  try { fs.accessSync("/bundle/config/app.json", fs.W_OK); }
                  catch (error) { accessCode = error.code; }
                  const temp = fs.mkdtempSync("/tmp/prefix-");
                  fs.writeFileSync(`${temp}/nested.txt`, "nested");
                  fs.linkSync(`${temp}/nested.txt`, `${temp}/hard.txt`);
                  fs.writeFileSync("/tmp/data/fd-append.txt", "a");
                  const appendFd = fs.openSync("/tmp/data/fd-append.txt", "r+");
                  fs.appendFileSync(appendFd, "b");
                  fs.fsync(appendFd);
                  fs.close(appendFd);
                  fs.cpSync(temp, "/tmp/copied", { recursive: true });
                  const matches = fs.globSync("**/*.txt", { cwd: "/tmp/copied" });
                  const fd = fs.openSync("/tmp/data/vectors.bin", "w+");
                  const written = fs.writevSync(fd, [Uint8Array.from([1, 2]), Uint8Array.from([3, 4])], 0);
                  fs.closeSync(fd);
                  const readFd = fs.openSync("/tmp/data/vectors.bin", "r");
                  const first = new Uint8Array(2);
                  const second = new Uint8Array(2);
                  const read = fs.readvSync(readFd, [first, second], 0);
                  fs.closeSync(readFd);
                  const dir = fs.opendirSync(temp);
                  const dirEntry = dir.readSync();
                  dir.closeSync();
                  const recursiveDir = fs.opendirSync(temp, { recursive: true });
                  const recursiveNames = [];
                  for (let entry; (entry = recursiveDir.readSync()) !== null;) recursiveNames.push(entry.name);
                  recursiveDir.closeSync();
                  const callbackDir = fs.opendirSync(temp);
                  const callbackDirEntry = await new Promise((resolve, reject) => callbackDir.read(
                    (error, entry) => error ? reject(error) : resolve(entry.name),
                  ));
                  const callbackDirClose = await new Promise((resolve, reject) => callbackDir.close(
                    error => error ? reject(error) : resolve("closed"),
                  ));
                  const callbackDirClosedRead = await new Promise(resolve => callbackDir.read(
                    error => resolve(error && error.message),
                  ));
                  const handle = await fs.promises.open("/tmp/data/value.txt", "r");
                  const handleRead = await handle.read({ length: 5 });
                  await handle.close();
                  const writeHandle = await fs.promises.open("/tmp/data/handle-write.txt", "w+");
                  const writeResult = await writeHandle.writeFile("write");
                  await writeHandle.appendFile("!");
                  const handleWriteText = await writeHandle.readFile("utf8");
                  await writeHandle.close();
                  const blob = fs.openAsBlob("/bundle/config/app.json");
                  const blobText = await blob.text();
                  const nativeRealpath = await new Promise((resolve, reject) => fs.realpath.native(
                    "/tmp/data/value.txt",
                    (error, value) => error ? reject(error) : resolve(value),
                  ));
                  const callbackRead = await new Promise((resolve, reject) => {
                    const callbackFd = fs.openSync("/tmp/data/value.bin", "r");
                    const callbackBuffer = new Uint8Array(3);
                    fs.read(callbackFd, callbackBuffer, 0, 3, 0, (error, count, output) => {
                      fs.closeSync(callbackFd);
                      error ? reject(error) : resolve({ count, output: [...output] });
                    });
                  });
                  const callbackReadOptions = await new Promise((resolve, reject) => {
                    const overloadFd = fs.openSync("/tmp/data/value.bin", "r");
                    fs.read(overloadFd, { length: 3, position: 0 }, (error, count, output) => {
                      fs.closeSync(overloadFd);
                      error ? reject(error) : resolve({ count, output: [...output] });
                    });
                  });
                  const callbackReadBufferOptions = await new Promise((resolve, reject) => {
                    const overloadFd = fs.openSync("/tmp/data/value.bin", "r");
                    const overloadBuffer = new Uint8Array(3);
                    fs.read(overloadFd, overloadBuffer, { length: 3, position: 0 }, (error, count, output) => {
                      fs.closeSync(overloadFd);
                      error ? reject(error) : resolve({ count, output: [...output] });
                    });
                  });
                  for await (const match of fs.promises.glob("**/*.txt", { cwd: temp })) {
                    if (match !== "nested.txt" && match !== "hard.txt") throw new Error("bad glob");
                  }
                  return JSON.stringify({
                    accessCode,
                    hardLink: fs.readFileSync(`${temp}/hard.txt`, "utf8"),
                    fdAppend: fs.readFileSync("/tmp/data/fd-append.txt", "utf8"),
                    matches,
                    written,
                    read,
                    first: [...first],
                    second: [...second],
                    dirEntry: dirEntry.name,
                    recursiveNames,
                    callbackDirEntry,
                    callbackDirClose,
                    callbackDirClosedRead,
                    handleText: String.fromCharCode(...handleRead.buffer),
                    writeResult: writeResult.bytesWritten,
                    handleWriteText,
                    blobText,
                    callbackRead,
                    callbackReadOptions,
                    callbackReadBufferOptions,
                    nativeRealpath,
                    bigint: String(fs.statSync(`${temp}/nested.txt`, { bigint: true }).size),
                  });
                })()"#,
            )?;
            let advanced_result = advanced.finish::<String>().map_err(|error| {
                format!("advanced filesystem test failed: {error}; {:?}", ctx.catch())
            })?;
            assert_eq!(
                advanced_result,
                r#"{"accessCode":"ENOENT","hardLink":"nested","fdAppend":"ab","matches":["hard.txt","nested.txt"],"written":4,"read":4,"first":[1,2],"second":[3,4],"dirEntry":"hard.txt","recursiveNames":["hard.txt","nested.txt"],"callbackDirEntry":"hard.txt","callbackDirClose":"closed","callbackDirClosedRead":"ERR_DIR_CLOSED: directory is closed","handleText":"hello","writeResult":5,"handleWriteText":"write!","blobText":"{\"enabled\":true}","callbackRead":{"count":3,"output":[1,2,3]},"callbackReadOptions":{"count":3,"output":[1,2,3]},"callbackReadBufferOptions":{"count":3,"output":[1,2,3]},"nativeRealpath":"/tmp/data/value.txt","bigint":"6"}"#
            );
            Ok(())
        })
}

#[test]
fn gives_each_context_a_fresh_tmp_directory() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let runtime = node_fs_runtime(directory.path())?;

    let first = Context::full(&runtime)?;
    first.with(|ctx| -> Result<(), Box<dyn std::error::Error>> {
        install_node_fs(&ctx, directory.path())?;
        ctx.eval::<(), _>(
            r#"process.getBuiltinModule("node:fs").writeFileSync("/tmp/only-here", "value")"#,
        )?;
        Ok(())
    })?;
    drop(first);

    let second = Context::full(&runtime)?;
    second.with(|ctx| -> Result<(), Box<dyn std::error::Error>> {
        install_node_fs(&ctx, directory.path())?;
        let exists: bool =
            ctx.eval(r#"process.getBuiltinModule("node:fs").existsSync("/tmp/only-here")"#)?;
        assert!(!exists);
        Ok(())
    })?;
    Ok(())
}

#[test]
#[allow(clippy::too_many_lines)]
fn exposes_native_stream_bindings() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let runtime = node_fs_runtime(directory.path())?;
    let context = Context::full(&runtime)?;
    context.with(|ctx| -> Result<(), Box<dyn std::error::Error>> {
            install_node_fs(&ctx, directory.path())?;
            let result: String = ctx.eval(
                r#"(() => {
                  const nativeGetBuiltinModule = process.getBuiltinModule;
                  class Readable {
                    constructor() { this.chunks = []; this.ended = false; }
                    push(value) { if (value === null) this.ended = true; else this.chunks.push(value); return true; }
                    on(name, listener) {
                      if (name === "data") for (const chunk of this.chunks) listener(chunk);
                      if (name === "end" && this.ended) listener();
                      return this;
                    }
                  }
                  class Writable {}
                  process.getBuiltinModule = name => name === "node:stream"
                    ? { Readable, Writable }
                    : nativeGetBuiltinModule(name);
                  const fs = process.getBuiltinModule("node:fs");
                  fs.writeFileSync("/tmp/stream-source.txt", "source");
                  const read = new fs.ReadStream("/tmp/stream-source.txt");
                  let text = "";
                  read.on("data", chunk => text += String.fromCharCode(...chunk));
                  read.on("end", () => text += "!");
                  const fd = fs.openSync("/tmp/stream-source.txt", "r");
                  const readWithFd = fs.createReadStream("/tmp/stream-source.txt", { fd });
                  let fdText = "";
                  readWithFd.on("data", chunk => fdText += String.fromCharCode(...chunk));
                  const fdOpen = (() => { try { fs.fstatSync(fd); return true; } catch { return false; } })();
                  fs.closeSync(fd);
                  const write = new fs.WriteStream("/tmp/stream-destination.txt");
                  const callbacks = [];
                  write.write("one", error => callbacks.push(["write", error]));
                  write.end("two", error => callbacks.push(["end", error]));
                  const writeFd = fs.openSync("/tmp/stream-with-fd.txt", "w+");
                  const writeWithFd = fs.createWriteStream("/tmp/stream-with-fd.txt", { fd: writeFd });
                  writeWithFd.write("fd");
                  writeWithFd.end("stream");
                  const writeFdOpen = (() => { try { fs.fstatSync(writeFd); return true; } catch { return false; } })();
                  fs.closeSync(writeFd);
                  return JSON.stringify({
                    text,
                    fdText,
                    fdOpen,
                    writeFdOpen,
                    callbacks,
                    written: fs.readFileSync("/tmp/stream-destination.txt", "utf8"),
                    writtenWithFd: fs.readFileSync("/tmp/stream-with-fd.txt", "utf8"),
                  });
                })()"#,
            )
            .map_err(|error| format!("stream test failed: {error}; {:?}", ctx.catch()))?;
            assert_eq!(
                result,
                r#"{"text":"source!","fdText":"source","fdOpen":true,"writeFdOpen":true,"callbacks":[["write",null],["end",null]],"written":"onetwo","writtenWithFd":"fdstream"}"#
            );
            Ok(())
        })
}

#[test]
fn dirent_prototype_treats_a_missing_device_flag_as_a_plain_file()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let runtime = node_fs_runtime(directory.path())?;
    let context = Context::full(&runtime)?;
    context.with(|ctx| -> Result<(), Box<dyn std::error::Error>> {
        install_node_fs(&ctx, directory.path())?;
        let result: String = ctx.eval(
            r#"(() => {
              const fs = process.getBuiltinModule("node:fs");
              const entry = properties => Object.assign(Object.create(fs.Dirent.prototype), properties);
              const file = entry({ name: "value.txt", type: "file" });
              const device = entry({ name: "zero", type: "file", device: true });
              const directory = entry({ name: "data", type: "directory" });
              return JSON.stringify({
                file: [file.isFile(), file.isCharacterDevice(), file.isDirectory()],
                device: [device.isFile(), device.isCharacterDevice(), device.isDirectory()],
                directory: [directory.isFile(), directory.isCharacterDevice(), directory.isDirectory()],
              });
            })()"#,
        )?;
        assert_eq!(
            result,
            r#"{"file":[true,false,false],"device":[false,true,false],"directory":[false,false,true]}"#
        );
        Ok(())
    })
}
