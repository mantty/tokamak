// workerd's node:fs/promises FileHandle, over the host's state for its open
// descriptor. The parameters give each method workerd's length.
import type { FileHandleState } from "tokamak:host";
import { EventEmitter } from "../../events/events.ts";
import { objectOf } from "../../globals/objects.ts";
import { createReadStream, createWriteStream, kFileHandle } from "./streams.ts";

const kConstruct = Symbol("kConstruct");

export class FileHandle extends EventEmitter {
  readonly #state: FileHandleState;
  [kFileHandle] = true;

  constructor(token: unknown, state: FileHandleState) {
    if (token !== kConstruct) throw new TypeError("Illegal constructor");
    super();
    this.#state = state;
  }

  get fd() { return this.#state.fd; }
  async appendFile(data: unknown, ...options: [options?: unknown]) { return this.#state.appendFile(data, ...options); }
  async chmod(mode: unknown) { return this.#state.chmod(mode); }
  async chown(uid: number, gid: number) { return this.#state.chown(uid, gid); }
  async datasync() { return this.#state.sync(); }
  async sync() { return this.#state.sync(); }
  async read(...args: unknown[]) { return this.#state.read(...args); }
  async readv(buffers: unknown, ...position: [position?: unknown]) { return this.#state.readv(buffers, ...position); }
  async readFile(...options: [options?: unknown]) { return this.#state.readFile(...options); }
  readLines(): never { throw new Error("readLines is not implemented"); }
  async stat(...options: [options?: unknown]) { return this.#state.stat(...options); }
  async truncate(...length: [length?: number]) { return this.#state.truncate(...length); }
  async utimes(atime: unknown, mtime: unknown) { return this.#state.utimes(atime, mtime); }
  async write(data: unknown, ...args: unknown[]) { return this.#state.write(data, ...args); }
  async writev(buffers: unknown, ...args: unknown[]) { return this.#state.writev(buffers, ...args); }
  async writeFile(data: unknown, ...options: [options?: unknown]) { return this.#state.writeFile(data, ...options); }

  // workerd emits `close` for each call, also once the handle is closed.
  async close() {
    this.#state.close();
    this.emit("close");
  }

  readableWebStream() { return this.#state.readableWebStream(); }
  createReadStream(options: unknown = {}) { return createReadStream("", { ...objectOf(options ?? {}), fd: this }); }
  createWriteStream(options: unknown = {}) { return createWriteStream("", { ...objectOf(options ?? {}), fd: this }); }
  async [Symbol.asyncDispose]() { await this.close(); }
}

/** A `FileHandle` over `state`. */
export function fileHandle(state: FileHandleState) {
  return new FileHandle(kConstruct, state);
}
