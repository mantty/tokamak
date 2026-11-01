// APIs that workerd implements and the type packages do not declare, or
// declare more narrowly than, or differently from, how workerd implements them.
interface Performance {
  timerify<T extends (...args: never[]) => unknown>(fn: T): T;
}

declare module "perf_hooks" {
  function timerify<T extends (...args: never[]) => unknown>(fn: T): T;
  function eventLoopUtilization(utilization1?: EventLoopUtilization, utilization2?: EventLoopUtilization): EventLoopUtilization;
}

declare module "fs" {
  // With the buffer encoding, a directory names its entries in Buffers.
  function opendirSync(path: PathLike, options: Omit<OpenDirOptions, "encoding"> & { encoding: "buffer" }): Omit<Dir, "readSync"> & { readSync(): Dirent<Buffer> | null };
  // Deprecated aliases of the stream classes.
  const FileReadStream: typeof ReadStream;
  const FileWriteStream: typeof WriteStream;
  interface WriteStream {
    // An alias of end().
    destroySoon: WriteStream["end"];
  }
}

declare module "node:fs" {
  import type { OpenAsBlobOptions, PathLike } from "fs";
  // workerd returns the Blob itself, where Node returns a promise of it. This
  // module's own declaration replaces the one it re-exports from "fs".
  export function openAsBlob(path: PathLike, options?: OpenAsBlobOptions): Blob;
}

declare module "zlib" {
  // Zlib streams pass their options to the Transform constructor.
  interface ZlibOptions {
    highWaterMark?: number;
  }
  // Every zlib stream can be reset.
  interface BrotliCompress extends ZlibReset {}
  interface ZstdCompress extends ZlibReset {}
  // With the `info` option, the convenience methods return their engine too.
  // An overload added here comes after the declared ones, so it is taken only
  // where those fail, as they do for a callback that takes the engine.
  function brotliCompress(buf: InputType, options: BrotliOptions & { info: true }, callback: (error: Error | null, result: { buffer: Buffer; engine: BrotliCompress }) => void): void;
}

declare module "node:zlib" {
  import type { Gzip, InputType, ZlibOptions } from "zlib";
  // The declared gzipSync would match `info` first, so this module's own
  // declaration replaces the one it re-exports from "zlib".
  export function gzipSync(buf: InputType, options: ZlibOptions & { info: true }): { buffer: Buffer; engine: Gzip };
  export function gzipSync(buf: InputType, options?: ZlibOptions): Buffer;
}

declare module "net" {
  // A port reserved in the isolate's port table, which a Socket or Server
  // adopts as its handle.
  class BoundSocket implements Disposable {
    static isBoundSocket(value: unknown): value is BoundSocket;
    constructor(options?: { port?: number; host?: string | null; ipv6Only?: boolean; reusePort?: boolean });
    address(): AddressInfo;
    fd(): number;
    close(): void;
    [Symbol.dispose](): void;
  }
  interface SocketConstructorOpts {
    handle?: BoundSocket;
  }
  interface Socket {
    // The peer's address, or an empty object before the socket connects.
    _getpeername(): Partial<AddressInfo>;
  }
  const Stream: typeof Socket;
  // The options and listener that connect() or listen() arguments give.
  function _normalizeArgs(args: unknown[]): unknown[];
}

declare module "http" {
  // The bindings of the request that workerd dispatched to the server.
  interface IncomingMessage {
    readonly cloudflare: { readonly env?: unknown; readonly ctx?: unknown; readonly cf?: unknown };
  }
}

declare module "node:buffer" {
  // workers-types declares the global Buffer as any, and the buffer module
  // exports that global, so this module declares its own.
  export const Buffer: BufferConstructor;
  export type Buffer<T extends ArrayBufferLike = ArrayBufferLike> = globalThis.Buffer<T>;
}
