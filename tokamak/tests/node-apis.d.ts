// APIs that workerd implements and the type packages do not declare, or
// declare more narrowly than, or differently from, how workerd implements them.
interface Performance {
  timerify<T extends (...args: never[]) => unknown>(fn: T): T;
}

interface Buffer<TArrayBuffer extends ArrayBufferLike = ArrayBufferLike> {
  // What util.inspect() shows of the buffer.
  inspect(): string;
}

declare module "perf_hooks" {
  function timerify<T extends (...args: never[]) => unknown>(fn: T): T;
  function eventLoopUtilization(utilization1?: EventLoopUtilization, utilization2?: EventLoopUtilization): EventLoopUtilization;
}

declare module "crypto" {
  // Node compares ArrayBuffers as well as views.
  function timingSafeEqual(a: NodeJS.ArrayBufferView | ArrayBuffer, b: NodeJS.ArrayBufferView | ArrayBuffer): boolean;
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

declare module "async_hooks" {
  interface AsyncLocalStorage<T> {
    // The name given in the constructor's options.
    readonly name: string;
  }
  interface AsyncResource {
    // The bound function runs with `thisArg` as its receiver.
    bind<Func extends (...args: any[]) => any>(fn: Func, thisArg: unknown): Func;
  }
}

declare module "url" {
  // workerd exports this helper, which Node keeps internal.
  function toPathIfFileURL(fileURLOrPath: URL | string): string;
}

declare module "querystring" {
  // Undocumented exports, and the option to decode `+` as a space.
  function unescape(str: string, decodeSpaces?: boolean): string;
  function unescapeBuffer(str: string, decodeSpaces?: boolean): Buffer;
}

declare module "util" {
  // Undocumented exports, which workerd declares and does not implement.
  function _errnoException(err: number, syscall: string, original?: string): NodeJS.ErrnoException;
  function _exceptionWithHostPort(err: number, syscall: string, address: string, port?: number, additional?: string): NodeJS.ErrnoException;
}

declare module "node:test" {
  namespace test {
    // workerd exports the mock classes, which Node does not.
    const MockTracker: { new (): MockTracker; readonly prototype: MockTracker };
    const MockFunctionContext: { new (): MockFunctionContext<Function>; readonly prototype: MockFunctionContext<Function> };
  }
}

declare module "zlib" {
  // Zlib streams pass their options to the Transform constructor.
  interface ZlibOptions {
    highWaterMark?: number;
  }
  // The result codes, by name and by number.
  const codes: { readonly [name: string]: number | string };
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
    // Sockets pass their options to the Duplex constructor.
    highWaterMark?: number;
    readableHighWaterMark?: number;
    writableHighWaterMark?: number;
  }
  interface Socket {
    // The peer's address, or an empty object before the socket connects.
    _getpeername(): Partial<AddressInfo>;
    // The socket's own address, or an empty object before the socket connects.
    _getsockname(): Partial<AddressInfo>;
  }
  interface BlockList {
    // Each address may be a string or a SocketAddress.
    addAddress(address: string | SocketAddress, type?: IPVersion): void;
    addRange(start: string | SocketAddress, end: string | SocketAddress, type?: IPVersion): void;
    addSubnet(net: string | SocketAddress, prefix: number, type?: IPVersion): void;
    check(address: string | SocketAddress, type?: IPVersion): boolean;
  }
  interface SocketAddress {
    toJSON(): { address: string; port: number; family: IPVersion; flowlabel: number };
  }
  namespace SocketAddress {
    function isSocketAddress(value: unknown): value is SocketAddress;
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
