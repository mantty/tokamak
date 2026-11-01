// workerd's node:fs Dirent, Dir and Stats classes, which the host makes from
// its records.
import type { DirEntryRecord, StatRecord } from "tokamak:host";
import { Buffer } from "../../node/buffer.ts";
import { codedError } from "../../node/errors.ts";
import { propertyOf } from "../../globals/objects.ts";

// workerd keeps an entry's libuv `UV_DIRENT_*` type under a private symbol.
const kType = Symbol("type");

export class Dirent {
  name: unknown;
  parentPath: unknown;
  [kType]: unknown;
  constructor(name: unknown, type: unknown, path: unknown) {
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
}

const closed = () => codedError(Error, "Directory is closed", "ERR_DIR_CLOSED");

// A directory's entries, which the host reads when the Dir is opened.
export class Dir {
  #handle: DirEntryRecord[] | undefined;
  #encoding: string;
  #path: unknown;
  constructor(handle: DirEntryRecord[], path: unknown, options: unknown) {
    if (handle == null) throw codedError(TypeError, 'The "handle" argument must be specified', "ERR_MISSING_ARGS");
    this.#handle = handle;
    this.#path = path;
    if (options === null || typeof options !== "object" || Array.isArray(options)) {
      throw codedError(TypeError, 'The "options" argument must be of type object', "ERR_INVALID_ARG_TYPE");
    }
    const given = propertyOf(options, "encoding");
    const encoding = given === undefined ? "utf8" : given;
    if (typeof encoding !== "string") {
      throw codedError(TypeError, 'The "options.encoding" property must be of type string', "ERR_INVALID_ARG_TYPE");
    }
    this.#encoding = encoding;
  }
  get path() { return this.#path; }
  read(callback?: unknown) {
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
  close(callback?: unknown) {
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
}

const kConstruct = Symbol("kConstruct");

// A file's status. The host keeps no times, so each time is 0, as workerd
// reports it.
export class Stats {
  dev: number | bigint;
  ino: number | bigint;
  mode: number | bigint;
  nlink: number | bigint;
  uid: number | bigint;
  gid: number | bigint;
  rdev: number | bigint;
  size: number | bigint;
  blksize: number | bigint;
  blocks: number | bigint;
  atimeMs: number | bigint;
  mtimeMs: number | bigint;
  ctimeMs: number | bigint;
  birthtimeMs: number | bigint;
  atimeNs: bigint | undefined;
  mtimeNs: bigint | undefined;
  ctimeNs: bigint | undefined;
  birthtimeNs: bigint | undefined;
  atime: Date;
  mtime: Date;
  ctime: Date;
  birthtime: Date;
  constructor(key: unknown, stat: StatRecord, options: { bigint: boolean }) {
    if (key !== kConstruct) throw new TypeError("Illegal constructor");
    const integer = options.bigint ? BigInt : Number;
    this.dev = integer(stat.device);
    this.ino = integer(0);
    this.mode = integer(stat.mode);
    this.nlink = integer(1);
    this.uid = integer(0);
    this.gid = integer(0);
    this.rdev = integer(0);
    this.size = integer(stat.size);
    this.blksize = integer(0);
    this.blocks = integer(0);
    this.atimeMs = integer(0);
    this.mtimeMs = integer(0);
    this.ctimeMs = integer(0);
    this.birthtimeMs = integer(0);
    if (options.bigint) {
      this.atimeNs = 0n;
      this.mtimeNs = 0n;
      this.ctimeNs = 0n;
      this.birthtimeNs = 0n;
    }
    this.atime = new Date(0);
    this.mtime = new Date(0);
    this.ctime = new Date(0);
    this.birthtime = new Date(0);
  }
  isBlockDevice() { return (Number(this.mode) & 0o170000) === 0o60000; }
  isCharacterDevice() { return (Number(this.mode) & 0o170000) === 0o20000; }
  isDirectory() { return (Number(this.mode) & 0o170000) === 0o40000; }
  isFIFO() { return (Number(this.mode) & 0o170000) === 0o10000; }
  isFile() { return (Number(this.mode) & 0o170000) === 0o100000; }
  isSocket() { return (Number(this.mode) & 0o170000) === 0o140000; }
  isSymbolicLink() { return (Number(this.mode) & 0o170000) === 0o120000; }
}

/** The `Stats` of `stat`, with bigint numbers when `bigint` is true. */
export function stats(stat: StatRecord, bigint: boolean) {
  return new Stats(kConstruct, stat, { bigint });
}
