// workerd's node:fs ReadStream and WriteStream, over the callback fs API.
import { builtinNamespace, processCwd } from "tokamak:host";
import type { EventEmitter } from "../../events/events.ts";
import { callFunction, callMethod, isFunction, isObject, objectOf, propertyOf } from "../../globals/objects.ts";
import type { AnyFunction } from "../../globals/objects.ts";
import { Readable, Writable, finished } from "../../streams/node.js";
import { Buffer } from "../../node/buffer.ts";
import { codedError } from "../../node/errors.ts";
import { fileURLToPath } from "../../node/url.ts";
import { URL } from "../../network/url.ts";
import {
  invalidArgType, invalidArgValue, outOfRange, parseFileMode, validateAbortSignal, validateBoolean,
  validateFunction, validateInt32, validateObject, validateString, validateThisInternalField, validateUint32,
} from "../../node/validators.ts";

export const kFileHandle = Symbol("kFileHandle");
const kFs = Symbol("kFs");
const kIsPerformingIO = Symbol("kIsPerformingIO");
const kIoDone = Symbol("kIoDone");
const kHandle = Symbol("kHandle");

/** The callback fs operations a stream performs its I/O with, which `options.fs` may replace. */
type FsOperations = Record<"open" | "close" | "fsync" | "read" | "write" | "writev", AnyFunction>;

/** A node:fs FileHandle, which carries `kFileHandle`: the members its streams use. */
interface FileHandle extends EventEmitter {
  readonly fd: number | undefined;
  close(): Promise<void>;
  sync(): Promise<void>;
  read(...args: unknown[]): Promise<{ bytesRead: number; buffer: unknown }>;
  write(...args: unknown[]): Promise<{ bytesWritten: number; buffer: unknown }>;
  writev(...args: unknown[]): Promise<{ bytesWritten: number; buffers: unknown }>;
}

/** What a file stream keeps besides its stream state. */
interface FileStream {
  fd: unknown;
  path?: string;
  flags?: unknown;
  mode?: unknown;
  flush?: boolean;
  start: number;
  pos: number | undefined;
  bytesRead: number;
  [kFs]: FsOperations;
  [kHandle]?: FileHandle;
  [kIsPerformingIO]: boolean;
  open?: unknown;
  close(callback?: unknown): void;
  readonly pending: boolean;
  autoClose: boolean;
}

export interface ReadStream extends Readable, FileStream {
  end: number;
}

export interface WriteStream extends Writable, FileStream {
  flush: boolean;
  bytesWritten: number;
  destroySoon: Writable["end"];
}

interface ReadStreamConstructor {
  new (path: unknown, options?: unknown): ReadStream;
  (path: unknown, options?: unknown): ReadStream;
  readonly prototype: ReadStream;
}

interface WriteStreamConstructor {
  new (path: unknown, options?: unknown): WriteStream;
  (path: unknown, options?: unknown): WriteStream;
  readonly prototype: WriteStream;
}

/** A callback a file stream's I/O reports to. */
type Callback = (error?: unknown) => void;

// node:fs loads this module, so this module loads node:fs on first use.
let lazyFs: Record<string, unknown> | undefined;
async function getLazyFs() {
  lazyFs ??= builtinNamespace("node:fs");
  return lazyFs;
}

// Calls `callback` with `args`, reporting what it throws.
function report(callback: unknown, ...args: unknown[]) {
  try { if (callback != null) callFunction(callback, undefined, ...args); } catch (error) { reportError(error); }
}

// Runs the callback fs operation `name` with `args` once fs is loaded, and
// passes `callback` its error, or null and its first `resultCount` results.
function deferred(name: string, args: unknown[], callback: unknown, resultCount: number) {
  const done = (error: unknown, ...results: unknown[]) => {
    if (error) report(callback, error);
    else report(callback, null, ...results.slice(0, resultCount));
  };
  getLazyFs().then(fs => callMethod(fs, name, ...args, done), error => report(callback, error));
}

const kDefaultFsOperations: FsOperations = {
  open(path: unknown, flags: unknown = "r", mode: unknown = 0o666, cb?: unknown) {
    let callback;
    if (typeof flags === "function") callback = flags;
    else if (typeof mode === "function") callback = mode;
    else if (typeof cb === "function") callback = cb;
    else throw codedError(TypeError, 'The "fs.open callback" argument must be specified', "ERR_MISSING_ARGS");
    validateFunction(callback, "fs.open callback");
    const openFlags = typeof flags === "function" ? "r" : flags;
    const openMode = typeof mode === "function" ? 0o666 : mode;
    const done = (error: unknown, fd?: unknown) => {
      if (error) report(callback, error);
      else report(callback, null, fd);
    };
    getLazyFs().then(fs => {
      try { callMethod(fs, "open", path, openFlags, openMode, done); } catch (error) { done(error); }
    }, done);
  },
  close(fd: unknown, cb: unknown = () => {}) { deferred("close", [fd], cb, 0); },
  fsync(fd: unknown, cb: unknown = () => {}) { deferred("fsync", [fd], cb, 0); },
  read(fd: unknown, buffer: unknown, offset: unknown, length: unknown, position: unknown, cb: unknown) { deferred("read", [fd, buffer, offset, length, position], cb, 2); },
  write(fd: unknown, buffer: unknown, offset: unknown, length: unknown, position: unknown, cb: unknown) { deferred("write", [fd, buffer, offset, length, position], cb, 2); },
  writev(fd: unknown, buffers: unknown, position: unknown, cb: unknown) { deferred("writev", [fd, buffers, position], cb, 2); },
};

function isFileHandle(object: unknown): object is FileHandle {
  return typeof object === "object" && object !== null && Reflect.has(object, kFileHandle);
}

function getValidatedFd(fd: unknown, propName = "fd") {
  if (Object.is(fd, -0)) return 0;
  validateInt32(fd, propName, 0);
  return fd;
}

// The file URL of `path`, resolved against the working directory.
function normalizePath(path: unknown, encoding = "utf8") {
  if (path instanceof URL) return path;
  const text = Buffer.isBuffer(path) ? path.toString(encoding) : path;
  if (typeof text !== "string") throw invalidArgType("path", ["string", "Buffer", "URL"], path);
  if (text.includes("\0")) throw invalidArgValue("path", text, "must not contain null bytes");
  const escaped = text.replace(/[#?]/g, encodeURIComponent).replace(/\/\//g, "/");
  return new URL(escaped, escaped.startsWith("/") ? "file://" : `file://${processCwd()}/`);
}

// Node's errorOrDestroy, for a stream that is not destroyed.
function errorOrDestroy(stream: ReadStream, error: unknown) {
  const state = stream._readableState;
  if (state.autoDestroy) {
    stream.destroy(error);
    return;
  }
  state.errored ??= error;
  if (state.errorEmitted) return;
  state.errorEmitted = true;
  stream.emit("error", error);
}

function construct(this: ReadStream | WriteStream, callback: Callback) {
  const stream = this;
  if (typeof stream.fd === "number") {
    callback(null);
    return;
  }
  if (typeof stream.open === "function") {
    // Backwards compat for monkey patching open().
    const orgEmit = stream.emit;
    stream.emit = function (this: ReadStream | WriteStream, ...args: unknown[]) {
      if (args[0] === "open") {
        this.emit = orgEmit;
        callback(null);
        Reflect.apply(orgEmit, this, args);
      } else if (args[0] === "error") {
        this.emit = orgEmit;
        callback(args[1]);
      } else {
        Reflect.apply(orgEmit, this, args);
      }
      return true;
    };
    callMethod(stream, "open");
    return;
  }
  stream[kFs].open(stream.path, stream.flags, stream.mode, (er: unknown, fd: unknown) => {
    if (er) {
      callback(er);
      return;
    }
    if (fd === undefined) {
      callback(invalidArgValue("fd", "undefined"));
      return;
    }
    stream.fd = fd;
    callback(null);
    stream.emit("open", stream.fd);
    stream.emit("ready");
  });
}

// `handle`'s methods as callback fs operations.
function fileHandleOperations(handle: FileHandle): FsOperations {
  const settle = <T>(pending: Promise<T>, cb: unknown, result: (value: T) => unknown[]) => pending.then(value => callFunction(cb, undefined, null, ...result(value)), error => callFunction(cb, undefined, error));
  return {
    open() { throw codedError(Error, "The open() method is not implemented", "ERR_METHOD_NOT_IMPLEMENTED"); },
    close(...args: unknown[]) { settle(handle.close(), args.at(-1), () => []); },
    fsync(...args: unknown[]) { settle(handle.sync(), args.at(-1), () => []); },
    read(...args: unknown[]) { settle(handle.read(...args.slice(1, -1)), args.at(-1), result => [result.bytesRead, result.buffer]); },
    write(...args: unknown[]) { settle(handle.write(...args.slice(1, -1)), args.at(-1), result => [result.bytesWritten, result.buffer]); },
    writev(...args: unknown[]) { settle(handle.writev(...args.slice(1, -1)), args.at(-1), result => [result.bytesWritten, result.buffers]); },
  };
}

function getValidatedFsOptions(fs: unknown): FsOperations {
  validateObject(fs, "options.fs");
  if (isFileHandle(fs)) return fileHandleOperations(fs);
  const { open = kDefaultFsOperations.open, close = kDefaultFsOperations.close, fsync = kDefaultFsOperations.fsync,
    read = kDefaultFsOperations.read, write = kDefaultFsOperations.write, writev = kDefaultFsOperations.writev } = fs;
  validateFunction(open, "options.fs.open");
  validateFunction(read, "options.fs.read");
  validateFunction(close, "options.fs.close");
  validateFunction(fsync, "options.fs.fsync");
  validateFunction(write, "options.fs.write");
  validateFunction(writev, "options.fs.writev");
  return { open, close, fsync, read, write, writev };
}

function readImpl(this: ReadStream, size: number) {
  const n = this.pos !== undefined ? Math.min(this.end - this.pos + 1, size) : Math.min(this.end - this.bytesRead + 1, size);
  if (n <= 0) {
    this.push(null);
    return;
  }
  const buf = Buffer.allocUnsafeSlow(n);
  this[kIsPerformingIO] = true;
  if (this.fd == null) {
    this.push(null);
    return;
  }
  this[kFs].read(this.fd, buf, 0, n, this.pos, (er: unknown, bytesRead: unknown, buffer: unknown) => {
    this[kIsPerformingIO] = false;
    // Tell ._destroy() that it's safe to close the fd now.
    if (this.destroyed) {
      this.emit(kIoDone, er);
      return;
    }
    if (er) {
      errorOrDestroy(this, er);
      return;
    }
    if (buffer == null) {
      errorOrDestroy(this, invalidArgValue("buf", "null"));
      return;
    }
    const count = Number(bytesRead);
    if (bytesRead == null || count <= 0) {
      this.push(null);
      return;
    }
    if (this.pos !== undefined) this.pos += count;
    this.bytesRead += count;
    if (count === propertyOf(buffer, "byteLength")) {
      this.push(buffer);
      return;
    }
    // Copy the bytes read so the stream keeps no large backing buffer.
    const dst = Buffer.allocUnsafeSlow(count);
    Buffer.from(propertyOf(buffer, "buffer"), propertyOf(buffer, "byteOffset"), count).copy(dst, 0, 0, count);
    this.push(dst);
  });
}

function actualCloseImpl(stream: ReadStream | WriteStream, err: unknown, cb: Callback) {
  if (stream.fd == null) return;
  stream[kFs].close(stream.fd, (er: unknown) => {
    cb(er || err);
  });
  stream.fd = null;
}

function closeImpl(stream: ReadStream | WriteStream, err: unknown, cb: Callback) {
  if (stream.fd == null) {
    cb(err);
  } else if (stream.flush) {
    stream[kFs].fsync(stream.fd, (flushErr: unknown) => {
      actualCloseImpl(stream, err || flushErr, cb);
    });
  } else {
    actualCloseImpl(stream, err, cb);
  }
}

function destroyImpl(this: ReadStream | WriteStream, err: unknown, cb: Callback) {
  // A descriptor is closed only once no read or write is using it.
  if (this[kIsPerformingIO]) {
    this.once(kIoDone, (er: unknown) => {
      closeImpl(this, err || er, cb);
    });
  } else {
    closeImpl(this, err, cb);
  }
}

// The options object `options` names.
function streamOptions(options: unknown) {
  const object = options === null ? {} : typeof options === "string" ? { encoding: options } : options;
  validateObject(object, "options");
  return object;
}

function validateStreamEncoding(encoding: unknown) {
  if (encoding !== "buffer" && encoding !== null && !Buffer.isEncoding(encoding)) {
    throw invalidArgValue("options.encoding", encoding);
  }
}

// The `end` option: Infinity, or a uint32 no less than `start`.
function streamEnd(end: unknown, start: number) {
  if (end === Infinity) return Infinity;
  validateUint32(end, "options.end");
  if (start > end) throw outOfRange("start", `<= "end" (here: ${end})`, start);
  return end;
}

// Sets the stream's descriptor and fs operations from `fd`, or its path,
// flags and mode when `fd` is null.
function openTarget(stream: ReadStream | WriteStream, path: unknown, options: Record<PropertyKey, unknown>, fd: unknown, fs: unknown, defaultFlags: string) {
  if (fd == null) {
    stream.fd = null;
    // Path will be ignored when fd is specified, so it can be falsy
    stream.path = fileURLToPath(normalizePath(path));
    stream.flags = options.flags === undefined ? defaultFlags : options.flags;
    stream.mode = options.mode === undefined ? 0o666 : options.mode;
  } else if (isFileHandle(fd)) {
    if (fs !== kDefaultFsOperations) {
      throw codedError(Error, "The FileHandle with fs method is not implemented", "ERR_METHOD_NOT_IMPLEMENTED");
    }
    stream[kHandle] = fd;
    stream[kFs] = getValidatedFsOptions(fd);
    fd.on("close", () => stream.close());
    stream.fd = fd.fd || null;
  } else {
    stream.fd = getValidatedFd(fd);
  }
}

export const ReadStream = function (this: unknown, path: unknown, options: unknown = {}): ReadStream {
  if (!(this instanceof ReadStream)) return new ReadStream(path, options);
  const settings = streamOptions(options);
  const { encoding = null, autoClose = true, emitClose = true, start = 0, end = Infinity, highWaterMark = 64 * 1024,
    signal = null, flags = "r", fd = null, mode = 0o666, fs = kDefaultFsOperations } = settings;
  validateStreamEncoding(encoding);
  validateBoolean(autoClose, "options.autoClose");
  validateBoolean(emitClose, "options.emitClose");
  validateUint32(start, "options.start");
  const last = streamEnd(end, start);
  validateUint32(highWaterMark, "options.highWaterMark");
  if (signal != null) validateAbortSignal(signal, "options.signal");
  validateString(flags, "options.flags");
  parseFileMode(mode, "options.mode", 0o666);
  this[kFs] = getValidatedFsOptions(fs);
  openTarget(this, path, settings, fd, fs, "r");
  this.start = start;
  this.end = last;
  this.pos = start;
  this.bytesRead = 0;
  this[kIsPerformingIO] = false;
  Reflect.apply(Readable, this, [{
    highWaterMark,
    encoding,
    emitClose,
    autoDestroy: autoClose,
    signal,
    construct: construct.bind(this),
    read: readImpl.bind(this),
    destroy: destroyImpl.bind(this),
  }]);
  return this;
} as ReadStreamConstructor;
Object.setPrototypeOf(ReadStream.prototype, Readable.prototype);
Object.setPrototypeOf(ReadStream, Readable);

Object.defineProperty(ReadStream.prototype, "autoClose", {
  get(this: unknown) {
    validateThisInternalField(this, kFs, "ReadStream");
    return propertyOf(this._readableState, "autoDestroy") || false;
  },
  set(this: unknown, value: unknown) {
    validateThisInternalField(this, kFs, "ReadStream");
    const state = this._readableState;
    if (isObject(state)) state.autoDestroy = value;
  },
});

ReadStream.prototype.close = function (this: ReadStream, cb: unknown = () => {}) {
  if (isFunction(cb)) finished(this, cb);
  this.destroy();
};

Object.defineProperty(ReadStream.prototype, "pending", {
  get(this: FileStream) { return this.fd === null; },
  configurable: true,
});

function writeAll(this: WriteStream, data: Uint8Array, size: number, pos: number | undefined, cb: Callback, retries = 0) {
  if (this.fd == null) return cb(invalidArgValue("fd", "null"));
  this[kFs].write(this.fd, data, 0, size, pos, (er: unknown, bytesWritten: unknown, buffer: unknown) => {
    // No data currently available and operation should be retried later.
    const retry = propertyOf(er, "code") === "EAGAIN";
    const error = retry ? null : er;
    if (this.destroyed || error) return cb(error || streamDestroyed("write"));
    const written = retry ? 0 : Number(bytesWritten ?? 0);
    this.bytesWritten += written;
    const attempts = written ? 0 : retries + 1;
    const remaining = size - written;
    const next = pos === undefined ? undefined : pos + written;
    // Try writing non-zero number of bytes up to 5 times.
    if (attempts > 5) {
      cb(codedError(Error, "write failed", "ERR_SYSTEM_ERROR"));
    } else if (remaining && buffer != null) {
      const buf = typeof buffer === "string" ? Buffer.from(buffer) : Buffer.from(propertyOf(buffer, "buffer"), propertyOf(buffer, "byteOffset"), propertyOf(buffer, "byteLength"));
      writeAll.call(this, buf.slice(written), remaining, next, cb, attempts);
    } else {
      cb(null);
    }
  });
}

function writevAll(this: WriteStream, chunks: Uint8Array[], size: number, pos: number | undefined, cb: Callback, retries = 0) {
  if (this.fd == null) return cb(invalidArgValue("fd", "null"));
  this[kFs].writev(this.fd, chunks, this.pos, (er: unknown, bytesWritten: unknown, buffers: unknown) => {
    // No data currently available and operation should be retried later.
    const retry = propertyOf(er, "code") === "EAGAIN";
    const error = retry ? null : er;
    if (this.destroyed || error) return cb(error || streamDestroyed("writev"));
    const written = retry ? 0 : Number(bytesWritten ?? 0);
    this.bytesWritten += written;
    const attempts = written ? 0 : retries + 1;
    const remaining = size - written;
    const next = pos === undefined ? undefined : pos + written;
    const writtenChunks = Array.isArray(buffers) ? buffers : [];
    // Try writing non-zero number of bytes up to 5 times.
    if (attempts > 5) {
      cb(codedError(Error, "writev failed", "ERR_SYSTEM_ERROR"));
    } else if (remaining && writtenChunks.length > 0) {
      const bufs = writtenChunks.map(b => Buffer.from(propertyOf(b, "buffer"), propertyOf(b, "byteOffset"), propertyOf(b, "byteLength")));
      writevAll.call(this, [Buffer.concat(bufs).slice(written)], remaining, next, cb, attempts);
    } else {
      cb(null);
    }
  });
}

function streamDestroyed(name: string) {
  return codedError(Error, `Cannot call ${name} after a stream was destroyed`, "ERR_STREAM_DESTROYED");
}

// Runs `callback` with the result of a write, telling ._destroy() when the
// descriptor is free to close.
function afterWrite(stream: WriteStream, callback: AnyFunction): Callback {
  return er => {
    stream[kIsPerformingIO] = false;
    const destroyed = stream.destroyed;
    callback(er);
    if (destroyed) stream.emit(kIoDone, er);
  };
}

function writeImpl(this: WriteStream, data: Uint8Array | string, encodingOrCallback: unknown, cb?: unknown) {
  const [encoding, callback] = isFunction(encodingOrCallback) ? [null, encodingOrCallback] : encodingOrCallback != null ? [encodingOrCallback, cb] : [null, undefined];
  validateFunction(callback, "write callback");
  const bytes = typeof data === "string" ? Buffer.from(data, encoding) : data;
  this[kIsPerformingIO] = true;
  writeAll.call(this, bytes, bytes.byteLength, this.pos, afterWrite(this, callback));
  if (this.pos !== undefined) this.pos += bytes.byteLength;
}

function writevImpl(this: WriteStream, data: { chunk: Uint8Array }[], callback: AnyFunction) {
  let size = 0;
  const chunks = data.map(({ chunk }) => {
    size += chunk.length;
    return chunk;
  });
  this[kIsPerformingIO] = true;
  writevAll.call(this, chunks, size, this.pos, afterWrite(this, callback));
  if (this.pos !== undefined) this.pos += size;
}

export const WriteStream = function (this: unknown, path: unknown, options: unknown = {}): WriteStream {
  if (!(this instanceof WriteStream)) return new WriteStream(path, options);
  const settings = streamOptions(options);
  const { encoding = null, autoClose = true, emitClose = true, start = 0, highWaterMark = 64 * 1024,
    signal = null, flags = "w", fd = null, mode = 0o666, fs = kDefaultFsOperations, flush: flushOption = false } = settings;
  validateStreamEncoding(encoding);
  validateBoolean(autoClose, "options.autoClose");
  validateBoolean(emitClose, "options.emitClose");
  const flush = flushOption === null ? false : flushOption;
  validateBoolean(flush, "options.flush");
  validateUint32(start, "options.start");
  validateUint32(highWaterMark, "options.highWaterMark");
  if (signal != null) validateAbortSignal(signal, "options.signal");
  validateString(flags, "options.flags");
  parseFileMode(mode, "options.mode", 0o666);
  this[kFs] = getValidatedFsOptions(fs);
  this.flush = flush;
  openTarget(this, path, settings, fd, fs, "w");
  this.start = start;
  this.pos = start;
  this.bytesRead = 0;
  this.bytesWritten = 0;
  this[kIsPerformingIO] = false;
  Reflect.apply(Writable, this, [{
    highWaterMark,
    encoding,
    emitClose,
    autoDestroy: autoClose,
    signal,
    construct: construct.bind(this),
    write: writeImpl.bind(this),
    writev: writevImpl.bind(this),
    destroy: destroyImpl.bind(this),
  }]);
  if (encoding != null && encoding !== "buffer") this.setDefaultEncoding(encoding);
  return this;
} as WriteStreamConstructor;
Object.setPrototypeOf(WriteStream.prototype, Writable.prototype);
Object.setPrototypeOf(WriteStream, Writable);

Object.defineProperty(WriteStream.prototype, "autoClose", {
  get(this: unknown) {
    validateThisInternalField(this, kFs, "WriteStream");
    return propertyOf(this._writableState, "autoDestroy") || false;
  },
  set(this: unknown, value: unknown) {
    validateThisInternalField(this, kFs, "WriteStream");
    const state = this._writableState;
    if (isObject(state)) state.autoDestroy = value;
  },
});

WriteStream.prototype.close = function (this: WriteStream, cb: unknown = () => {}) {
  if (cb) {
    if (this.closed) {
      queueMicrotask(() => callFunction(cb, undefined, null));
      return;
    }
    validateFunction(cb, "listener");
    this.on("close", cb);
  }
  // Without autoClose, the stream is destroyed once it finishes.
  if (!this.autoClose) this.on("finish", () => this.destroy());
  // Files have no shutdown(), so end() closes them.
  this.end();
};

// There is no shutdown() for files.
WriteStream.prototype.destroySoon = WriteStream.prototype.end;

Object.defineProperty(WriteStream.prototype, "pending", {
  get(this: FileStream) { return this.fd === null; },
  configurable: true,
});

export function createReadStream(path: unknown, options: unknown = {}) {
  return new ReadStream(path, options);
}

export function createWriteStream(path: unknown, options: unknown = {}) {
  return new WriteStream(path, options);
}

// FileHandle's stream methods.
export const fileHandleMethods = {
  createReadStream(this: unknown, options: unknown = {}) {
    return createReadStream("", { ...objectOf(options ?? {}), fd: this });
  },
  createWriteStream(this: unknown, options: unknown = {}) {
    return createWriteStream("", { ...objectOf(options ?? {}), fd: this });
  },
};
