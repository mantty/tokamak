// workerd's node:fs ReadStream and WriteStream, over the callback fs API.
import { builtinNamespace, processCwd } from "tokamak:host";
import { Readable, Writable, finished } from "../../streams/node.mjs";
import { Buffer } from "../../node/buffer.mjs";
import { codedError } from "../../node/errors.mjs";
import { fileURLToPath } from "../../node/url.mjs";
import { URL } from "../../network/url.mjs";
import {
  invalidArgType, invalidArgValue, outOfRange, parseFileMode, validateAbortSignal, validateBoolean,
  validateFunction, validateInt32, validateObject, validateString, validateThisInternalField, validateUint32,
} from "../../node/validators.mjs";

export const kFileHandle = Symbol("kFileHandle");
const kFs = Symbol("kFs");
const kIsPerformingIO = Symbol("kIsPerformingIO");
const kIoDone = Symbol("kIoDone");
const kHandle = Symbol("kHandle");

// node:fs loads this module, so this module loads node:fs on first use.
let lazyFs;
async function getLazyFs() {
  lazyFs ??= builtinNamespace("node:fs");
  return lazyFs;
}

// Calls `callback` with `args`, reporting what it throws.
function report(callback, ...args) {
  try { callback?.(...args); } catch (error) { reportError(error); }
}

// Runs the callback fs operation `name` with `args` once fs is loaded, and
// passes `callback` its error, or null and its first `resultCount` results.
function deferred(name, args, callback, resultCount) {
  const done = (error, ...results) => {
    if (error) report(callback, error);
    else report(callback, null, ...results.slice(0, resultCount));
  };
  getLazyFs().then(fs => fs[name](...args, done), error => report(callback, error));
}

const kDefaultFsOperations = {
  open(path, flags = "r", mode = 0o666, cb) {
    let callback;
    if (typeof flags === "function") callback = flags;
    else if (typeof mode === "function") callback = mode;
    else if (typeof cb === "function") callback = cb;
    else throw codedError(TypeError, 'The "fs.open callback" argument must be specified', "ERR_MISSING_ARGS");
    validateFunction(callback, "fs.open callback");
    const openFlags = typeof flags === "function" ? "r" : flags;
    const openMode = typeof mode === "function" ? 0o666 : mode;
    const done = (error, fd) => {
      if (error) report(callback, error);
      else report(callback, null, fd);
    };
    getLazyFs().then(fs => {
      try { fs.open(path, openFlags, openMode, done); } catch (error) { done(error); }
    }, done);
  },
  close(fd, cb = () => {}) { deferred("close", [fd], cb, 0); },
  fsync(fd, cb = () => {}) { deferred("fsync", [fd], cb, 0); },
  read(fd, buffer, offset, length, position, cb) { deferred("read", [fd, buffer, offset, length, position], cb, 2); },
  write(fd, buffer, offset, length, position, cb) { deferred("write", [fd, buffer, offset, length, position], cb, 2); },
  writev(fd, buffers, position, cb) { deferred("writev", [fd, buffers, position], cb, 2); },
};

function isFileHandle(object) {
  return typeof object === "object" && object !== null && Reflect.has(object, kFileHandle);
}

function getValidatedFd(fd, propName = "fd") {
  if (Object.is(fd, -0)) return 0;
  validateInt32(fd, propName, 0);
  return fd;
}

// The file URL of `path`, resolved against the working directory.
function normalizePath(path, encoding = "utf8") {
  if (path instanceof URL) return path;
  if (Buffer.isBuffer(path)) path = path.toString(encoding);
  else if (typeof path !== "string") throw invalidArgType("path", ["string", "Buffer", "URL"], path);
  if (path.includes("\0")) throw invalidArgValue("path", path, "must not contain null bytes");
  path = path.replace(/[#?]/g, encodeURIComponent).replace(/\/\//g, "/");
  return new URL(path, path.startsWith("/") ? "file://" : `file://${processCwd()}/`);
}

// Node's errorOrDestroy, for a stream that is not destroyed.
function errorOrDestroy(stream, error) {
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

function construct(callback) {
  const stream = this;
  if (typeof stream.fd === "number") {
    callback(null);
    return;
  }
  if (typeof stream.open === "function") {
    // Backwards compat for monkey patching open().
    const orgEmit = stream.emit;
    stream.emit = function (...args) {
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
    stream.open();
    return;
  }
  stream[kFs].open(stream.path, stream.flags, stream.mode, (er, fd) => {
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
function fileHandleOperations(handle) {
  const settle = (pending, cb, result) => pending.then(value => cb(null, ...result(value)), error => cb(error));
  return {
    open() { throw codedError(Error, "The open() method is not implemented", "ERR_METHOD_NOT_IMPLEMENTED"); },
    close(...args) { settle(handle.close(), args.at(-1), () => []); },
    fsync(...args) { settle(handle.sync(), args.at(-1), () => []); },
    read(...args) { settle(handle.read(...args.slice(1, -1)), args.at(-1), result => [result.bytesRead, result.buffer]); },
    write(...args) { settle(handle.write(...args.slice(1, -1)), args.at(-1), result => [result.bytesWritten, result.buffer]); },
    writev(...args) { settle(handle.writev(...args.slice(1, -1)), args.at(-1), result => [result.bytesWritten, result.buffers]); },
  };
}

function getValidatedFsOptions(fs) {
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

function readImpl(n) {
  n = this.pos !== undefined ? Math.min(this.end - this.pos + 1, n) : Math.min(this.end - this.bytesRead + 1, n);
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
  this[kFs].read(this.fd, buf, 0, n, this.pos, (er, bytesRead, buf) => {
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
    if (buf == null) {
      errorOrDestroy(this, invalidArgValue("buf", "null"));
      return;
    }
    if (bytesRead == null || bytesRead <= 0) {
      this.push(null);
      return;
    }
    if (this.pos !== undefined) this.pos += bytesRead;
    this.bytesRead += bytesRead;
    if (bytesRead !== buf.byteLength) {
      // Copy the bytes read so the stream keeps no large backing buffer.
      const dst = Buffer.allocUnsafeSlow(bytesRead);
      Buffer.from(buf.buffer, buf.byteOffset, bytesRead).copy(dst, 0, 0, bytesRead);
      buf = dst;
    }
    this.push(buf);
  });
}

function actualCloseImpl(stream, err, cb) {
  if (stream.fd == null) return;
  stream[kFs].close(stream.fd, er => {
    cb(er || err);
  });
  stream.fd = null;
}

function closeImpl(stream, err, cb) {
  if (stream.fd == null) {
    cb(err);
  } else if (stream.flush) {
    stream[kFs].fsync(stream.fd, flushErr => {
      actualCloseImpl(stream, err || flushErr, cb);
    });
  } else {
    actualCloseImpl(stream, err, cb);
  }
}

function destroyImpl(err, cb) {
  // A descriptor is closed only once no read or write is using it.
  if (this[kIsPerformingIO]) {
    this.once(kIoDone, er => {
      closeImpl(this, err || er, cb);
    });
  } else {
    closeImpl(this, err, cb);
  }
}

// The options object `options` names.
function streamOptions(options) {
  if (options === null) options = {};
  else if (typeof options === "string") options = { encoding: options };
  validateObject(options, "options");
  return options;
}

function validateStreamEncoding(encoding) {
  if (encoding !== "buffer" && encoding !== null && !Buffer.isEncoding(encoding)) {
    throw invalidArgValue("options.encoding", encoding);
  }
}

// Sets the stream's descriptor and fs operations from `fd`, or its path,
// flags and mode when `fd` is null.
function openTarget(stream, path, options, fd, fs, defaultFlags) {
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

export function ReadStream(path, options = {}) {
  if (!(this instanceof ReadStream)) return new ReadStream(path, options);
  options = streamOptions(options);
  const { encoding = null, autoClose = true, emitClose = true, start = 0, end = Infinity, highWaterMark = 64 * 1024,
    signal = null, flags = "r", fd = null, mode = 0o666, fs = kDefaultFsOperations } = options;
  validateStreamEncoding(encoding);
  validateBoolean(autoClose, "options.autoClose");
  validateBoolean(emitClose, "options.emitClose");
  validateUint32(start, "options.start");
  if (end !== Infinity) {
    validateUint32(end, "options.end");
    if (start > end) throw outOfRange("start", `<= "end" (here: ${end})`, start);
  }
  validateUint32(highWaterMark, "options.highWaterMark");
  if (signal != null) validateAbortSignal(signal, "options.signal");
  validateString(flags, "options.flags");
  parseFileMode(mode, "options.mode", 0o666);
  this[kFs] = getValidatedFsOptions(fs);
  openTarget(this, path, options, fd, fs, "r");
  this.start = start;
  this.end = end;
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
}
Object.setPrototypeOf(ReadStream.prototype, Readable.prototype);
Object.setPrototypeOf(ReadStream, Readable);

Object.defineProperty(ReadStream.prototype, "autoClose", {
  get() {
    validateThisInternalField(this, kFs, "ReadStream");
    return this._readableState?.autoDestroy || false;
  },
  set(value) {
    validateThisInternalField(this, kFs, "ReadStream");
    if (this._readableState !== undefined) this._readableState.autoDestroy = value;
  },
});

ReadStream.prototype.close = function (cb = () => {}) {
  if (typeof cb === "function") finished(this, cb);
  this.destroy();
};

Object.defineProperty(ReadStream.prototype, "pending", {
  get() { return this.fd === null; },
  configurable: true,
});

function writeAll(data, size, pos, cb, retries = 0) {
  if (this.fd == null) return cb(invalidArgValue("fd", "null"));
  this[kFs].write(this.fd, data, 0, size, pos, (er, bytesWritten, buffer) => {
    // No data currently available and operation should be retried later.
    if (er?.code === "EAGAIN") {
      er = null;
      bytesWritten = 0;
    }
    if (this.destroyed || er) return cb(er || streamDestroyed("write"));
    bytesWritten ??= 0;
    this.bytesWritten += bytesWritten;
    retries = bytesWritten ? 0 : retries + 1;
    size -= bytesWritten;
    pos += bytesWritten;
    // Try writing non-zero number of bytes up to 5 times.
    if (retries > 5) {
      cb(codedError(Error, "write failed", "ERR_SYSTEM_ERROR"));
    } else if (size && buffer != null) {
      const buf = typeof buffer === "string" ? Buffer.from(buffer) : Buffer.from(buffer.buffer, buffer.byteOffset, buffer.byteLength);
      writeAll.call(this, buf.slice(bytesWritten), size, pos, cb, retries);
    } else {
      cb(null);
    }
  });
}

function writevAll(chunks, size, pos, cb, retries = 0) {
  if (this.fd == null) return cb(invalidArgValue("fd", "null"));
  this[kFs].writev(this.fd, chunks, this.pos, (er, bytesWritten, buffers) => {
    // No data currently available and operation should be retried later.
    if (er?.code === "EAGAIN") {
      er = null;
      bytesWritten = 0;
    }
    if (this.destroyed || er) return cb(er || streamDestroyed("writev"));
    bytesWritten ??= 0;
    this.bytesWritten += bytesWritten;
    retries = bytesWritten ? 0 : retries + 1;
    size -= bytesWritten;
    pos += bytesWritten;
    // Try writing non-zero number of bytes up to 5 times.
    buffers ??= [];
    if (retries > 5) {
      cb(codedError(Error, "writev failed", "ERR_SYSTEM_ERROR"));
    } else if (size && buffers.length > 0) {
      const bufs = buffers.map(b => Buffer.from(b.buffer, b.byteOffset, b.byteLength));
      writevAll.call(this, [Buffer.concat(bufs).slice(bytesWritten)], size, pos, cb, retries);
    } else {
      cb(null);
    }
  });
}

function streamDestroyed(name) {
  return codedError(Error, `Cannot call ${name} after a stream was destroyed`, "ERR_STREAM_DESTROYED");
}

// Runs `callback` with the result of a write, telling ._destroy() when the
// descriptor is free to close.
function afterWrite(stream, callback) {
  return er => {
    stream[kIsPerformingIO] = false;
    const destroyed = stream.destroyed;
    callback(er);
    if (destroyed) stream.emit(kIoDone, er);
  };
}

function writeImpl(data, encodingOrCallback, cb) {
  let callback;
  let encoding = null;
  if (typeof encodingOrCallback === "function") {
    callback = encodingOrCallback;
  } else if (encodingOrCallback != null) {
    encoding = encodingOrCallback;
    if (cb !== undefined) callback = cb;
  }
  validateFunction(callback, "write callback");
  if (typeof data === "string") data = Buffer.from(data, encoding);
  this[kIsPerformingIO] = true;
  writeAll.call(this, data, data.byteLength, this.pos, afterWrite(this, callback));
  if (this.pos !== undefined) this.pos += data.byteLength;
}

function writevImpl(data, callback) {
  let size = 0;
  const chunks = data.map(({ chunk }) => {
    size += chunk.length;
    return chunk;
  });
  this[kIsPerformingIO] = true;
  writevAll.call(this, chunks, size, this.pos, afterWrite(this, callback));
  if (this.pos !== undefined) this.pos += size;
}

export function WriteStream(path, options = {}) {
  if (!(this instanceof WriteStream)) return new WriteStream(path, options);
  options = streamOptions(options);
  const { encoding = null, autoClose = true, emitClose = true, start = 0, highWaterMark = 64 * 1024,
    signal = null, flags = "w", fd = null, mode = 0o666, fs = kDefaultFsOperations } = options;
  let { flush = false } = options;
  validateStreamEncoding(encoding);
  validateBoolean(autoClose, "options.autoClose");
  validateBoolean(emitClose, "options.emitClose");
  if (flush === null) flush = false;
  validateBoolean(flush, "options.flush");
  validateUint32(start, "options.start");
  validateUint32(highWaterMark, "options.highWaterMark");
  if (signal != null) validateAbortSignal(signal, "options.signal");
  validateString(flags, "options.flags");
  parseFileMode(mode, "options.mode", 0o666);
  this[kFs] = getValidatedFsOptions(fs);
  this.flush = flush;
  openTarget(this, path, options, fd, fs, "w");
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
}
Object.setPrototypeOf(WriteStream.prototype, Writable.prototype);
Object.setPrototypeOf(WriteStream, Writable);

Object.defineProperty(WriteStream.prototype, "autoClose", {
  get() {
    validateThisInternalField(this, kFs, "WriteStream");
    return this._writableState?.autoDestroy || false;
  },
  set(value) {
    validateThisInternalField(this, kFs, "WriteStream");
    if (this._writableState !== undefined) this._writableState.autoDestroy = value;
  },
});

WriteStream.prototype.close = function (cb = () => {}) {
  if (cb) {
    if (this.closed) {
      queueMicrotask(() => cb(null));
      return;
    }
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
  get() { return this.fd === null; },
  configurable: true,
});

export function createReadStream(path, options = {}) {
  return new ReadStream(path, options);
}

export function createWriteStream(path, options = {}) {
  return new WriteStream(path, options);
}

// FileHandle's stream methods.
export const fileHandleMethods = {
  createReadStream(options = {}) {
    return createReadStream("", { ...options, fd: this });
  },
  createWriteStream(options = {}) {
    return createWriteStream("", { ...options, fd: this });
  },
};
