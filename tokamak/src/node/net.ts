import EventEmitter from "../events/events.ts";
import { connect as connectSocket } from "../builtins/cloudflare-sockets.ts";
import { type AnyFunction, callMethod, isFunction, isObject, objectOf, propertyOf } from "../globals/objects.ts";
import { clearTimeout, setTimeout } from "../globals/timers.ts";
import { Duplex } from "../streams/node.js";
import { Buffer } from "./buffer.ts";
import { ipVersion } from "tokamak:host";
import { abortError, codedError } from "./errors.ts";
import { type CloudflareSocket, GATEWAY_ADDRESS, HOST_ADDRESS, type PortHandler, bindPort, tcpPorts, validatePort } from "./tcp-ports.ts";
import { inspect } from "./util.ts";
import { type AbortSignalLike, invalidArgType, invalidArgValue, outOfRange, validateAbortSignal, validateBoolean, validateFunction, validateInt32, validateNumber, validateObject, validateString, validateUint32 } from "./validators.ts";

const kLastWriteQueueSize = Symbol("kLastWriteQueueSize");
const kTimeout = Symbol("kTimeout");
const kBuffer = Symbol("kBuffer");
const kBufferCb = Symbol("kBufferCb");
const kBufferGen = Symbol("kBufferGen");
const kBufferContinuations = Symbol("kBufferContinuations");
const kBytesRead = Symbol("kBytesRead");
const kBytesWritten = Symbol("kBytesWritten");
const normalizedArgsSymbol = Symbol("normalizedArgs");
// The addresses of an opened socket.
const kSocketInfo = Symbol("kSocketInfo");
// A socket's local address, adopted from a BoundSocket or autobound on connect;
// kBoundReserved marks a port reservation that destroy releases.
const kBoundSource = Symbol("kBoundSource");
const kBoundReserved = Symbol("kBoundReserved");
const kBoundSocketConsume = Symbol("kBoundSocketConsume");
const DEFAULT_IPV4_ADDR = "0.0.0.0";
const DEFAULT_IPV6_ADDR = "::";
const TIMEOUT_MAX = 2 ** 31 - 1;
// The families a lookup function may report.
const LOOKUP_FAMILIES = new Set<unknown>([4, 6, "IPv4", "IPv6"]);

type Family = "IPv4" | "IPv6";
type FamilyName = "ipv4" | "ipv6";
type Callback = (error?: Error | null) => void;

// An endpoint, as address() and the address getters report it.
interface Address {
  address: string;
  family?: Family | undefined;
  port: number;
}

// The peer a connection handle was opened to.
interface HandleTarget {
  host: string;
  port: number;
  addressType: number;
}

// A connection: the cloudflare:sockets socket, its reader and writer, and the
// read loop's state.
interface SocketHandle {
  socket: CloudflareSocket;
  writer: WritableStreamDefaultWriter;
  reader: ReadableStreamBYOBReader;
  bytesRead: number;
  bytesWritten: number;
  // Whether the socket wants to be reading.
  reading: boolean;
  options: HandleTarget;
  // Set while startRead's loop runs over this handle.
  readLoopActive?: boolean;
  writeQueueSize?: number;
}

// The chunks of a writev, as the stream hands them over.
type WriteChunks = { chunk: unknown; encoding: unknown }[] & { allBuffers?: boolean };
// A write made while connecting.
type PendingData = string | Uint8Array | WriteChunks;

// The options and callback of connect() and listen(), once normalized.
type NormalizedArgs = [Record<PropertyKey, unknown>, AnyFunction | null] & { [normalizedArgsSymbol]?: boolean };

// The local endpoint and peer of a connection being opened.
interface Connection {
  localAddress: unknown;
  localPort: unknown;
  port: number;
  addressType: number;
}

export interface Socket extends Duplex {
  _handle: SocketHandle | null;
  connecting: boolean;
  _hadError: boolean;
  _parent: Socket | null;
  // The socket wrapping this one, which shares its local endpoint and timers.
  _parentWrap: Socket | null;
  _host: string | null;
  [kLastWriteQueueSize]: number | undefined;
  [kTimeout]: number | null;
  [kBuffer]: boolean | null;
  [kBufferCb]: AnyFunction | null | undefined;
  [kBufferGen]: (() => unknown) | null;
  [kBufferContinuations]: WeakMap<Uint8Array, Uint8Array> | null;
  [kSocketInfo]: { remoteAddress: Address } | null;
  [kBytesRead]: number;
  [kBytesWritten]: number;
  [kBoundSource]: Address | null;
  [kBoundReserved]: boolean;
  server: Server | null;
  _server: Server | null;
  _closeAfterHandlingError: boolean;
  autoSelectFamilyAttemptedAddresses: string[];
  _pendingData: PendingData | null;
  _pendingEncoding: unknown;
  timeout: unknown;
  resetAndClosing: boolean;
  _aborted: boolean;
  // Set by TLSSocket.
  encrypted?: boolean;
  // readable-stream members that Node's types leave out.
  _undestroy(): void;
  readonly writableBuffer: unknown;
  destroy(error?: unknown): this;
  _unrefTimer(): void;
  setTimeout(msecs: unknown, callback?: unknown): this;
  _onTimeout(): void;
  _getpeername(): Partial<Address>;
  _getsockname(): Partial<Address>;
  address(): Partial<Address>;
  _writeGeneric(writev: boolean, data: PendingData, encoding: unknown, cb: Callback): false | undefined;
  _reset(): this;
  resetAndDestroy(): this;
  destroySoon(): void;
  connect(...args: unknown[]): this | undefined;
  setNoDelay(enable?: unknown): this;
  setKeepAlive(enable?: unknown, initialDelay?: unknown): this;
  ref(): void;
  unref(): void;
  readonly _connecting: boolean;
  readonly pending: boolean;
  readonly readyState: string;
  readonly bufferSize: number | undefined;
  readonly bytesRead: number;
  readonly _bytesDispatched: number;
  readonly bytesWritten: number | undefined;
  readonly remoteAddress: string | undefined;
  readonly remoteFamily: Family | undefined;
  readonly remotePort: number | undefined;
  readonly localAddress: string | undefined;
  readonly localPort: number | undefined;
  readonly localFamily: Family | undefined;
}

interface SocketConstructor {
  new (options?: unknown): Socket;
  (options?: unknown): Socket;
  readonly prototype: Socket;
}

function validateFamily(family: unknown, name = "family"): asserts family is FamilyName {
  if (family !== "ipv4" && family !== "ipv6") throw invalidArgValue(name, family, "must be one of: 'ipv4', 'ipv6'");
}

function invalidIPAddress(address: unknown) {
  return codedError(TypeError, `Invalid IP address: ${address}`, "ERR_INVALID_IP_ADDRESS");
}

function invalidAddress(family: FamilyName) {
  return invalidIPAddress(`Invalid ${family === "ipv4" ? "IPv4" : "IPv6"} address`);
}

function socketClosed() {
  return codedError(Error, "Socket is closed", "ERR_SOCKET_CLOSED");
}

function handleAdopted() {
  return codedError(Error, "The bound socket has already been adopted by a server or socket", "ERR_SOCKET_HANDLE_ADOPTED");
}

// The family an IP version number names.
function versionFamily(version: unknown): Family | undefined {
  return version === 4 ? "IPv4" : version === 6 ? "IPv6" : undefined;
}

function familyOf(address: string): Family | undefined {
  return versionFamily(isIP(address));
}

function isWildcard(address: string): boolean {
  return address === DEFAULT_IPV4_ADDR || address === DEFAULT_IPV6_ADDR;
}

// A synthetic IPv4 address in the form `family` reports it.
function forFamily(address: string, family: Family | undefined): string {
  return family === "IPv6" ? `::ffff:${address}` : address;
}

// The address a socket bound to the wildcard reports once connected or
// accepted: the isolate's host address.
function concreteLocal(bound: Address): Address {
  return isWildcard(bound.address) ? { ...bound, address: forFamily(HOST_ADDRESS, bound.family) } : bound;
}

function releaseBoundSource(socket: Socket): void {
  const bound = socket[kBoundSource];
  if (socket[kBoundReserved] && bound !== null) {
    tcpPorts.unregister(socket);
    tcpPorts.release(bound.port);
    socket[kBoundReserved] = false;
  }
  socket[kBoundSource] = null;
}

// The address of a "host:port" or "[v6]:port" authority; "*" is the wildcard.
function parseAuthority(authority: unknown): Address | null {
  const match = typeof authority === "string" ? /^\[?([^\]]*?)\]?:(\d+)$/.exec(authority) : null;
  if (match === null) return null;
  const address = match[1] === "*" ? DEFAULT_IPV4_ADDR : match[1];
  const info: Address = { address, port: Number(match[2]) };
  const family = familyOf(address);
  if (family !== undefined) info.family = family;
  return info;
}

// The `_handle` of a socket over the cloudflare:sockets socket `socket`.
function socketHandle(socket: CloudflareSocket, options: HandleTarget): SocketHandle {
  return {
    socket, writer: socket.writable.getWriter(), reader: socket.readable.getReader({ mode: "byob" }),
    bytesRead: 0, bytesWritten: 0, reading: false, options,
  };
}

// A reserved port in the isolate's port table; the Socket or Server adopting it
// takes over the reservation and reports the bound address as its own.
export class BoundSocket {
  #address: Address | null;
  // Bound with port 0.
  #ephemeral: boolean;

  static isBoundSocket(value: unknown): value is BoundSocket {
    return typeof value === "object" && value !== null && #address in value;
  }

  constructor(options: unknown = {}) {
    validateObject(options, "options");
    if (options.path !== undefined) throw invalidArgValue("options.path", options.path, "is not supported");
    const port = validatePort(options.port ?? 0, "options.port");
    this.#ephemeral = port === 0;
    const ipv6Only = options.ipv6Only ?? false;
    validateBoolean(ipv6Only, "options.ipv6Only");
    const reusePort = options.reusePort ?? false;
    validateBoolean(reusePort, "options.reusePort");
    const { host: requestedHost } = options;
    let host = ipv6Only ? DEFAULT_IPV6_ADDR : DEFAULT_IPV4_ADDR;
    let addressType = ipv6Only ? 6 : 4;
    if (requestedHost !== undefined && requestedHost !== null) {
      validateString(requestedHost, "options.host");
      host = requestedHost;
      addressType = isIP(host);
      if (addressType === 0) throw invalidArgValue("options.host", host, "must be a numeric IP address; net.BoundSocket does not perform DNS resolution");
    }
    const address: Address = { address: host, family: addressType === 6 ? "IPv6" : "IPv4", port: bindPort(host, port, reusePort) };
    this.#address = address;
    tcpPorts.register(this, address.port);
  }

  address(): Address {
    if (this.#address === null) throw handleAdopted();
    return this.#address;
  }

  // Workers have no file descriptors.
  fd(): number {
    if (this.#address === null) throw handleAdopted();
    return -1;
  }

  close(): void {
    if (this.#address === null) throw handleAdopted();
    tcpPorts.unregister(this);
    tcpPorts.release(this.#address.port);
    this.#address = null;
  }

  [Symbol.dispose](): void {
    if (this.#address !== null) this.close();
  }

  // Transfers the reservation to the adopter.
  [kBoundSocketConsume](): { address: Address; ephemeral: boolean } {
    if (this.#address === null) throw handleAdopted();
    tcpPorts.unregister(this);
    const address = this.#address;
    this.#address = null;
    return { address, ephemeral: this.#ephemeral };
  }
}

// The host, port and reusePort of a server's listen() options.
function listenOptions(options: Record<PropertyKey, unknown>): { host: string; port: number; reusePort: boolean } {
  if (options.path != null) throw invalidArgValue("options.path", options.path, "is not supported");
  if (options.handle != null || options.fd != null) throw invalidArgValue("options", options, "only a net.BoundSocket handle is supported");
  const port = options.port == null ? 0 : validatePort(options.port, "options.port");
  const reusePort = options.reusePort ?? false;
  validateBoolean(reusePort, "options.reusePort");
  const { host } = options;
  if (host === undefined) return { host: options.ipv6Only ? DEFAULT_IPV6_ADDR : DEFAULT_IPV4_ADDR, port, reusePort };
  validateString(host, "options.host");
  return { host: host === "localhost" ? "127.0.0.1" : host, port, reusePort };
}

// A server's keep-alive delay option in whole seconds, where a negative delay is 0.
function keepAliveSeconds(delay: unknown): number {
  if (delay === undefined) return 0;
  validateNumber(delay, "options.keepAliveInitialDelay");
  return delay < 0 ? 0 : ~~(delay / 1000);
}

// A listening server reserves its port in the isolate's port table and
// installs a connect handler there; cloudflare:node routes inbound sockets to
// it by local port and wraps them as net.Sockets.
export class Server extends EventEmitter {
  #address: Address | null = null;
  #connections = 0;
  #closing = false;
  #handler: PortHandler = { connect: socket => this.#onConnection(socket) };
  allowHalfOpen: unknown;
  pauseOnConnect: boolean;
  noDelay: boolean;
  keepAlive: boolean;
  keepAliveInitialDelay: number;
  highWaterMark: unknown;
  maxConnections: number | undefined = undefined;

  constructor(options?: unknown, connectionListener?: unknown) {
    super();
    let settings: Record<PropertyKey, unknown> = {};
    if (typeof options === "function") connectionListener = options;
    else if (options != null) {
      validateObject(options, "options");
      settings = options;
    }
    if (connectionListener !== undefined) {
      validateFunction(connectionListener, "connectionListener");
      this.on("connection", connectionListener);
    }
    const keepAliveInitialDelay = keepAliveSeconds(settings.keepAliveInitialDelay);
    this.allowHalfOpen = settings.allowHalfOpen ?? false;
    this.pauseOnConnect = Boolean(settings.pauseOnConnect);
    this.noDelay = Boolean(settings.noDelay);
    this.keepAlive = Boolean(settings.keepAlive);
    this.keepAliveInitialDelay = keepAliveInitialDelay;
    this.highWaterMark = settings.highWaterMark;
  }

  listen(...args: unknown[]): this {
    const [options, callback] = _normalizeArgs(args);
    if (this.#address !== null) throw codedError(Error, "Listen method has been called more than once without closing.", "ERR_SERVER_ALREADY_LISTEN");
    if (callback !== null) this.once("listening", callback);
    let address: Address;
    if (BoundSocket.isBoundSocket(options)) ({ address } = options[kBoundSocketConsume]());
    else if (BoundSocket.isBoundSocket(options.handle)) ({ address } = options.handle[kBoundSocketConsume]());
    else {
      const { host, port, reusePort } = listenOptions(options);
      try {
        address = { address: host, port: bindPort(host, port, reusePort) };
      } catch (error) {
        queueMicrotask(() => this.emit("error", error));
        return this;
      }
      const family = familyOf(host);
      if (family !== undefined) address.family = family;
    }
    this.#address = address;
    this.#closing = false;
    tcpPorts.setHandler(address.port, this.#handler);
    queueMicrotask(() => this.emit("listening"));
    return this;
  }

  address(): Address | null {
    return this.#address === null ? null : { ...this.#address };
  }

  get listening(): boolean {
    return this.#address !== null;
  }

  close(callback?: unknown): this {
    if (isFunction(callback)) {
      if (this.#address === null) this.once("close", () => callback(codedError(Error, "Server is not running.", "ERR_SERVER_NOT_RUNNING")));
      else this.once("close", callback);
    }
    if (this.#address !== null) {
      tcpPorts.clearHandler(this.#address.port, this.#handler);
      tcpPorts.release(this.#address.port);
      this.#address = null;
    }
    this.#closing = true;
    this.#maybeEmitClose();
    return this;
  }

  // 'close' fires once the server is not listening and no connection remains.
  #maybeEmitClose(): void {
    if (!this.#closing || this.#address !== null || this.#connections !== 0) return;
    this.#closing = false;
    queueMicrotask(() => this.emit("close"));
  }

  getConnections(callback: unknown): this {
    validateFunction(callback, "cb");
    const count = this.#connections;
    queueMicrotask(() => callback(null, count));
    return this;
  }

  ref(): this { return this; }
  unref(): this { return this; }

  async [Symbol.asyncDispose](): Promise<void> {
    if (this.#address === null) return;
    const { promise, resolve } = Promise.withResolvers<void>();
    this.close(() => resolve());
    await promise;
  }

  // Accepts an inbound socket: its local endpoint is the server's address, and
  // a peer the platform does not report appears behind the gateway address.
  async #onConnection(handle: CloudflareSocket): Promise<void> {
    const info = await handle.opened;
    if (this.#address === null) {
      await handle.close();
      return;
    }
    const localAddress = concreteLocal(this.#address);
    const remoteAddress = parseAuthority(info.remoteAddress) ?? {
      address: forFamily(GATEWAY_ADDRESS, localAddress.family),
      family: localAddress.family ?? "IPv4",
      port: tcpPorts.ephemeral(),
    };
    if (this.maxConnections !== undefined && this.#connections >= this.maxConnections) {
      this.emit("drop", {
        localAddress: localAddress.address, localPort: localAddress.port, localFamily: localAddress.family,
        remoteAddress: remoteAddress.address, remotePort: remoteAddress.port, remoteFamily: remoteAddress.family,
      });
      await handle.close();
      return;
    }
    const socket = new Socket({ allowHalfOpen: this.allowHalfOpen, highWaterMark: this.highWaterMark });
    const connection = socketHandle(handle, { host: remoteAddress.address, port: remoteAddress.port, addressType: isIP(remoteAddress.address) });
    socket._handle = connection;
    socket[kSocketInfo] = { remoteAddress: { ...remoteAddress } };
    socket[kBoundSource] = localAddress;
    socket.server = this;
    socket._server = this;
    socket.connecting = false;
    socket._undestroy();
    this.#connections++;
    const closed = new Promise<void>(resolve => socket.once("close", () => resolve()));
    handle.closed.then(onConnectionClosed.bind(socket), error => socket.destroy(error));
    if (this.pauseOnConnect) socket.pause();
    this.emit("connection", socket);
    if (!socket.isPaused() && !connection.reading) tryReadStart(socket);
    await closed;
    this.#connections--;
    this.#maybeEmitClose();
  }
}

// Sets how `socket` reads: into the buffers `onread` hands out, delivered to its
// callback, or into fresh buffers pushed to the stream.
function setReadBuffer(socket: Socket, onread: unknown): void {
  const custom = typeof onread === "object" && onread !== null ? objectOf(onread) : undefined;
  const buffer = custom?.buffer;
  const callback = custom?.callback;
  if (!(buffer instanceof Uint8Array || isFunction(buffer)) || !isFunction(callback)) {
    socket[kBuffer] = true;
    socket[kBufferGen] = () => new Uint8Array(4096);
    socket[kBufferCb] = undefined;
    return;
  }
  socket[kBufferGen] = isFunction(buffer) ? buffer : () => buffer;
  socket[kBuffer] = true;
  // BYOB reads transfer the buffers they are handed, so a reused buffer is
  // continued over the store it moved to.
  socket[kBufferContinuations] = new WeakMap();
  socket[kBufferCb] = callback;
}

function adoptHandle(socket: Socket, handle: unknown): void {
  validateObject(handle, "options.handle");
  if (!BoundSocket.isBoundSocket(handle)) {
    // Any other handle is taken as a connection handle, as Node takes a native one.
    socket._handle = handle as unknown as SocketHandle;
    return;
  }
  const { address } = handle[kBoundSocketConsume]();
  socket[kBoundSource] = address;
  socket[kBoundReserved] = true;
  tcpPorts.register(socket, address.port);
}

// Throws for an object mode, which sockets do not support.
function rejectObjectModes(options: unknown): void {
  for (const name of ["objectMode", "readableObjectMode", "writableObjectMode"]) {
    const value = propertyOf(options, name);
    if (value) throw invalidArgValue(`options.${name}`, value, "is not supported");
  }
}

// A function rather than a class, as in Node, so that it can be called without
// `new`. TypeScript gives a function expression no construct signature, so its
// type is asserted.
export const Socket = function (this: unknown, options?: unknown): Socket {
  if (!(this instanceof Socket)) return new Socket(options);
  rejectObjectModes(options);
  const keepAliveInitialDelay = propertyOf(options, "keepAliveInitialDelay");
  if (keepAliveInitialDelay !== undefined) {
    validateNumber(keepAliveInitialDelay, "options.keepAliveInitialDelay");
    if (keepAliveInitialDelay < 0 && isObject(options)) options.keepAliveInitialDelay = 0;
  }
  const settings: Record<PropertyKey, unknown> = typeof options === "number" ? { fd: options } : { ...objectOf(options ?? {}) };
  if (settings.fd !== undefined) throw codedError(Error, "The options.fd option is not implemented", "ERR_OPTION_NOT_IMPLEMENTED");
  settings.allowHalfOpen = Boolean(settings.allowHalfOpen);
  // The socket emits 'close' itself, with hadError.
  settings.emitClose = false;
  settings.autoDestroy = true;
  settings.decodeStrings = false;
  settings.readable = true;
  settings.writable = true;
  this._handle = null;
  this.connecting = false;
  this._hadError = false;
  this._parent = null;
  this._parentWrap = null;
  this._host = null;
  this[kLastWriteQueueSize] = 0;
  this[kTimeout] = null;
  this[kBuffer] = null;
  this[kBufferCb] = null;
  this[kBufferGen] = null;
  this[kBufferContinuations] = null;
  this[kSocketInfo] = null;
  this[kBytesRead] = 0;
  this[kBytesWritten] = 0;
  this[kBoundSource] = null;
  this[kBoundReserved] = false;
  this.server = null;
  this._server = null;
  this._closeAfterHandlingError = false;
  this.autoSelectFamilyAttemptedAddresses = [];
  this._undestroy();
  this._pendingData = null;
  this._pendingEncoding = "";
  Duplex.call(this, settings);
  if (settings.handle) adoptHandle(this, settings.handle);
  // Every 'end', since a socket can reconnect.
  this.on("end", onReadableStreamEnd);
  setReadBuffer(this, settings.onread);
  if (settings.signal) addClientAbortSignalOption(this, settings.signal);
  this[kBytesRead] = 0;
  this[kBytesWritten] = 0;
  return this;
} as SocketConstructor;
Object.setPrototypeOf(Socket.prototype, Duplex.prototype);
Object.setPrototypeOf(Socket, Duplex);

// Restarts the idle timer of this socket and of any socket wrapping it.
Socket.prototype._unrefTimer = function _unrefTimer() {
  for (let socket: Socket | null = this; socket != null; socket = socket._parentWrap) {
    if (socket[kTimeout] == null) continue;
    const current = socket;
    clearTimeout(current[kTimeout]);
    current[kTimeout] = setTimeout(() => current._onTimeout(), current.timeout);
  }
};

// Disables the idle timer of `socket` and of any socket wrapping it.
function clearSocketTimers(socket: Socket): void {
  for (let current: Socket | null = socket; current != null; current = current._parentWrap) {
    clearTimeout(current[kTimeout]);
    current[kTimeout] = null;
  }
}

Socket.prototype.setTimeout = function (msecs, callback) {
  if (this.destroyed) return this;
  this.timeout = msecs;
  const duration = getTimerDuration(msecs, "msecs");
  clearTimeout(this[kTimeout]);
  this[kTimeout] = null;
  if (duration !== 0) this[kTimeout] = setTimeout(() => this._onTimeout(), duration);
  if (callback === undefined) return this;
  validateFunction(callback, "callback");
  if (duration === 0) this.removeListener("timeout", callback);
  else this.once("timeout", callback);
  return this;
};

Socket.prototype._onTimeout = function () {
  const handle = this._handle;
  const lastWriteQueueSize = this[kLastWriteQueueSize] ?? 0;
  // A changed write queue means a write is in progress.
  if (lastWriteQueueSize > 0 && handle && lastWriteQueueSize !== handle.writeQueueSize) {
    this[kLastWriteQueueSize] = handle.writeQueueSize;
    this._unrefTimer();
    return;
  }
  this.emit("timeout");
};

Socket.prototype._getpeername = function () {
  if (this._handle == null || this[kSocketInfo] == null) return {};
  return { ...this[kSocketInfo].remoteAddress };
};

Socket.prototype._getsockname = function () {
  if (this[kBoundSource] != null) return this[kBoundSource];
  // A wrapping socket shares the local endpoint of the socket it wraps.
  if (this._parentWrap != null) return this._parentWrap._getsockname();
  return {};
};

Socket.prototype.address = function () {
  return this._getsockname();
};

// The bytes of `chunks`, the chunks of a writev, as one buffer.
function concatenatedChunks(chunks: WriteChunks): Buffer {
  return Buffer.concat(chunks.map(({ chunk, encoding }) => {
    if (typeof chunk === "string") return Buffer.from(chunk, encoding);
    if (ArrayBuffer.isView(chunk)) return new Uint8Array(chunk.buffer, chunk.byteOffset, chunk.byteLength);
    throw invalidArgType("chunk", ["string", "ArrayBufferView"], chunk);
  }));
}

// The bytes of a write: a chunk, or the chunks of a writev.
function writeBytes(data: PendingData, encoding: unknown): Uint8Array {
  if (typeof data === "string") return Buffer.from(data, encoding);
  return Array.isArray(data) ? concatenatedChunks(data) : data;
}

Socket.prototype._writeGeneric = function (writev, data, encoding, cb) {
  try {
    // Writes made while connecting wait for the connection.
    if (this.connecting) {
      this._pendingData = data;
      this._pendingEncoding = encoding;
      const onClose = () => cb(codedError(Error, "Socket closed before connection established", "ERR_SOCKET_CLOSED_BEFORE_CONNECTION"));
      this.once("connect", () => {
        this.off("close", onClose);
        this._writeGeneric(writev, data, encoding, cb);
      });
      this.once("close", onClose);
      return;
    }
    this._pendingData = null;
    this._pendingEncoding = "";
    if (this._handle?.writer === undefined) {
      cb(socketClosed());
      return false;
    }
    this._unrefTimer();
    const bytes = writeBytes(data, encoding);
    this._handle.writer.write(bytes).then(() => {
      if (this._handle != null) this._handle.bytesWritten += this[kLastWriteQueueSize] ?? 0;
      else this[kBytesWritten] += this[kLastWriteQueueSize] ?? 0;
      this[kLastWriteQueueSize] = 0;
      this._unrefTimer();
      cb();
    }, error => {
      this[kLastWriteQueueSize] = 0;
      this._unrefTimer();
      cb(error);
    });
    this[kLastWriteQueueSize] = bytes.byteLength;
  } catch (error) {
    this.destroy(error);
  }
  return undefined;
};

Socket.prototype._writev = function (chunks: WriteChunks, cb: Callback) {
  this._writeGeneric(true, chunks, "", cb);
};

Socket.prototype._write = function (data: string | Uint8Array, encoding: unknown, cb: Callback) {
  this._writeGeneric(false, data, encoding, cb);
};

Socket.prototype._final = function (cb: Callback) {
  if (this.connecting) {
    this.once("connect", () => this._final(cb));
    return;
  }
  if (this._handle == null) {
    cb();
    return;
  }
  this._handle.writer.close().then(() => cb(), error => cb(error));
};

Socket.prototype.end = function (this: Socket, data?: unknown, encoding?: unknown, cb?: unknown) {
  Reflect.apply(Duplex.prototype.end, this, [data, encoding, cb]);
  return this;
};

Socket.prototype.pause = function (this: Socket) {
  // Stops the read loop after its current read.
  if (this[kBuffer] && !this.connecting && this._handle?.reading && !this.destroyed) this._handle.reading = false;
  Duplex.prototype.pause.call(this);
  return this;
};

Socket.prototype.resume = function (this: Socket) {
  if (this[kBuffer] && !this.connecting && this._handle && !this._handle.reading) tryReadStart(this);
  Duplex.prototype.resume.call(this);
  return this;
};

Socket.prototype.read = function (this: Socket, n?: number) {
  if (this[kBuffer] && !this.connecting && this._handle && !this._handle.reading) tryReadStart(this);
  return Duplex.prototype.read.call(this, n);
};

Socket.prototype._read = function (this: Socket, n: number) {
  if (this.connecting || !this._handle) this.once("connect", () => this._read(n));
  else if (!this._handle.reading) tryReadStart(this);
};

Socket.prototype._reset = function () {
  this.resetAndClosing = true;
  return this.destroy();
};

// Destroys the socket, once connected if it is connecting.
Socket.prototype.resetAndDestroy = function () {
  if (!this._handle) this.destroy(socketClosed());
  else if (this.connecting) this.once("connect", () => this._reset());
  else this._reset();
  return this;
};

Socket.prototype.destroySoon = function () {
  if (this.writable) this.end();
  if (this.writableFinished) this.destroy();
  else this.once("finish", () => this.destroy());
};

Socket.prototype._destroy = function (exception: Error | null, cb: Callback) {
  this.connecting = false;
  clearSocketTimers(this);
  releaseBoundSource(this);
  if (this._handle == null) {
    cleanupAfterDestroy(this, cb, exception);
    return;
  }
  this._handle.socket.close().then(() => cleanupAfterDestroy(this, cb, exception), error => cleanupAfterDestroy(this, cb, error || exception));
};

// Replaces the socket's connection: the previous handle is detached first so
// that its EOF and close do not end the socket, and its local endpoint dropped.
function reconnect(socket: Socket, previous: SocketHandle, options: Record<PropertyKey, unknown>): void {
  releaseBoundSource(socket);
  socket._handle = null;
  previous.reading = false;
  socket.connecting = true;
  const connectAgain = () => initializeConnection(socket, options);
  previous.socket.close().then(connectAgain, connectAgain);
}

function isNormalizedArgs(value: unknown): value is NormalizedArgs {
  return Array.isArray(value) && Boolean(propertyOf(value, normalizedArgsSymbol));
}

Socket.prototype.connect = function (...args) {
  const [options, cb] = isNormalizedArgs(args[0]) ? args[0] : _normalizeArgs(args);
  if (this.connecting) throw codedError(Error, "Socket is already connecting", "ERR_SOCKET_CONNECTING");
  if (this._aborted) {
    if (!cb) throw abortError();
    cb(abortError());
    return undefined;
  }
  if (cb !== null) this.once("connect", cb);
  if (this._parentWrap?.connecting) return this;
  if (options.port === undefined && options.path == null) {
    throw codedError(TypeError, "The \"options\" or \"port\" or \"path\" argument must be specified", "ERR_MISSING_ARGS");
  }
  if (this.write !== Socket.prototype.write) this.write = Socket.prototype.write;
  if (options.path != null) throw invalidArgValue("path", options.path, "is not supported");
  this[kBytesRead] = 0;
  this[kBytesWritten] = 0;
  if (this._handle) reconnect(this, this._handle, options);
  else initializeConnection(this, options);
  return this;
};

Socket.prototype.setNoDelay = function (_enable) {
  return this;
};

Socket.prototype.setKeepAlive = function (_enable, _initialDelay) {
  return this;
};

Socket.prototype.ref = function () {};

Socket.prototype.unref = function () {};

// The bytes written and those still queued, counting a write made while connecting.
function bytesWritten(socket: Socket): number | undefined {
  const data = socket._pendingData;
  if (!socket.writableBuffer) return undefined;
  if (!data) return socket.writableLength + (socket._handle != null ? socket._handle.bytesWritten : socket[kBytesWritten]);
  if (!Array.isArray(data)) return socket._bytesDispatched + (typeof data === "string" ? Buffer.byteLength(data, socket._pendingEncoding) : data.length);
  let bytes = socket._bytesDispatched;
  for (const entry of data) {
    // As in Node and workerd, a writev of buffers adds each entry's `length`,
    // which entries do not have, so the count is NaN.
    bytes += data.allBuffers ? Number(propertyOf(entry, "length")) : Buffer.byteLength(entry.chunk, entry.encoding);
  }
  return bytes;
}

// A getter's descriptor, with no attributes inherited from Object.prototype.
function getter(get: (this: Socket) => unknown, attributes: { configurable?: boolean; enumerable?: boolean } = {}) {
  return { __proto__: null, ...attributes, get };
}

function enumerableGetter(get: (this: Socket) => unknown) {
  return getter(get, { configurable: false, enumerable: true });
}

Object.defineProperties(Socket.prototype, {
  _connecting: getter(function () { return this.connecting; }),
  pending: getter(function () { return !this._handle || this.connecting; }, { configurable: true }),
  readyState: getter(function () {
    if (this.connecting) return "opening";
    if (this.readable && this.writable) return "open";
    if (this.readable) return "readOnly";
    return this.writable ? "writeOnly" : "closed";
  }),
  bufferSize: getter(function () { return this._handle ? this.writableLength : undefined; }),
  bytesRead: enumerableGetter(function () { return this._handle ? this._handle.bytesRead : this[kBytesRead]; }),
  _bytesDispatched: enumerableGetter(function () { return this._handle ? this._handle.bytesWritten : this[kBytesWritten]; }),
  bytesWritten: enumerableGetter(function () { return bytesWritten(this); }),
  remoteAddress: enumerableGetter(function () { return this._getpeername().address; }),
  remoteFamily: enumerableGetter(function () { return this._getpeername().family; }),
  remotePort: enumerableGetter(function () { return this._getpeername().port; }),
  localAddress: enumerableGetter(function () { return this._getsockname().address; }),
  localPort: enumerableGetter(function () { return this._getsockname().port; }),
  localFamily: enumerableGetter(function () { return this._getsockname().family; }),
});

// Ends a destroyed socket: the destroy callback reports the error, and 'close'
// follows it; without one, both are emitted directly.
function cleanupAfterDestroy(socket: Socket, cb: Callback | null | undefined, error: Error | null | undefined): void {
  const isException = error != null;
  if (socket._handle != null) {
    socket[kBytesRead] = socket._handle.bytesRead;
    socket[kBytesWritten] = socket._handle.bytesWritten;
    socket.resetAndClosing = false;
  }
  socket[kLastWriteQueueSize] = 0;
  socket[kSocketInfo] = null;
  if (cb != null) {
    cb(error);
    queueMicrotask(() => socket.emit("close", isException));
    return;
  }
  if (error != null) socket.emit("error", error);
  socket.emit("close", isException);
}

function initializeConnection(socket: Socket, options: Record<PropertyKey, unknown>): void {
  const { host = "localhost", family, hints, autoSelectFamily, autoSelectFamilyAttemptTimeout, lookup, localAddress, localPort, port } = options;
  if (socket[kBoundSource] != null && (localAddress !== undefined || localPort !== undefined)) {
    throw invalidArgValue("options", options, "localAddress and localPort cannot be used with an adopted bound socket");
  }
  if (localAddress && !isIP(localAddress)) throw invalidIPAddress(localAddress);
  if (localPort) validateNumber(localPort, "options.localPort");
  if (typeof port !== "number" && typeof port !== "string") throw invalidArgType("options.port", ["number", "string"], port);
  const portNumber = validatePort(port);
  if (autoSelectFamily != null) validateBoolean(autoSelectFamily, "options.autoSelectFamily");
  if (autoSelectFamilyAttemptTimeout != null) validateInt32(autoSelectFamilyAttemptTimeout, "options.autoSelectFamilyAttemptTimeout", 1);
  socket._unrefTimer();
  socket.connecting = true;
  if (lookup != null) validateFunction(lookup, "options.lookup");
  const addressType = isIP(host);
  const connection = { localAddress, localPort, port: portNumber, addressType };
  if (addressType !== 0 || lookup == null) {
    openConnection(socket, connection, String(host), addressType);
    return;
  }
  // The lookup function resolves a host that is not an IP address.
  lookup(host, { family: family || addressType, hints }, (error: unknown, address: unknown, resolvedFamily: unknown) => {
    socket.emit("lookup", error, address, resolvedFamily, host);
    if (error) {
      socket.destroy(error);
      return;
    }
    if (isIP(address) === 0) throw invalidIPAddress(address);
    if (!LOOKUP_FAMILIES.has(resolvedFamily)) throw invalidArgValue("family", resolvedFamily, "must be 4 or 6");
    openConnection(socket, connection, String(address), resolvedFamily);
  });
}

// Autobinds the local endpoint, as connect(2) does; only a nonzero localPort
// reserves a port, and localAddress alone is a label.
function autobind(socket: Socket, localAddress: unknown, localPort: unknown, family: unknown): Address {
  const address = localAddress == null ? (family === 6 ? DEFAULT_IPV6_ADDR : DEFAULT_IPV4_ADDR) : String(localAddress);
  const reserved = typeof localPort === "number" && localPort !== 0;
  const bound: Address = { address, family: isIP(address) === 6 ? "IPv6" : "IPv4", port: reserved ? bindPort(address, localPort) : tcpPorts.ephemeral() };
  if (!reserved) return bound;
  socket[kBoundReserved] = true;
  tcpPorts.register(socket, bound.port);
  return bound;
}

function openConnection(socket: Socket, { localAddress, localPort, port, addressType }: Connection, host: string, family: unknown): void {
  socket[kSocketInfo] = { remoteAddress: { address: host, port, family: versionFamily(family) } };
  const authority = family === 6 ? `[${host}]` : host;
  socket.autoSelectFamilyAttemptedAddresses = [`${authority}:${port}`];
  socket.emit("connectionAttempt", authority, port, addressType);
  socket._host = authority;
  try {
    // The platform does not report an outbound socket's source address.
    socket[kBoundSource] = concreteLocal(socket[kBoundSource] ?? autobind(socket, localAddress, localPort, family));
    // The Duplex owns the half-open policy, and a Node socket can always be upgraded to TLS.
    const handle = connectSocket(`${authority}:${port}`, { allowHalfOpen: true, secureTransport: socket.encrypted ? "on" : "starttls" });
    socket._handle = socketHandle(handle, { host: authority, port, addressType });
    socket._undestroy();
    handle.opened.then(onConnectionOpened.bind(socket), error => {
      socket.emit("connectionAttemptFailed", authority, port, addressType, error);
      socket.destroy(error);
    });
    // A reconnect may have replaced the handle by the time it reports closed.
    handle.closed.then(() => {
      if (socket._handle?.socket === handle) onConnectionClosed.call(socket);
    }, error => socket.destroy(error));
  } catch (error) {
    socket.destroy(error);
  }
}

function onConnectionOpened(this: Socket): void {
  if (this.destroyed) return;
  this.connecting = false;
  this._unrefTimer();
  this.emit("connect");
  this.emit("ready");
  if (!this.isPaused()) tryReadStart(this);
}

function onConnectionClosed(this: Socket): void {
  const handle = this._handle;
  if (handle?.socket.upgraded) return;
  clearSocketTimers(this);
  // A pending read reports the connection's end or failure itself.
  if (this.destroyed || handle == null || handle.readLoopActive) return;
  // Nobody is reading, so the readable ends here.
  this.push(null);
  this.read(0);
}

// The buffer the socket's onread option hands out for its next read.
function nextBuffer(socket: Socket): Uint8Array {
  const buffer = socket[kBufferGen]?.();
  if (!(buffer instanceof Uint8Array)) throw invalidArgType("onread.buffer", ["Buffer", "Uint8Array"], buffer);
  return buffer;
}

// The view a read loop reads into: `buffer`, or its continuation over the
// store that a previous read transferred it to.
function readView(socket: Socket, buffer: Uint8Array): Uint8Array {
  const view = socket[kBufferContinuations]?.get(buffer) ?? buffer;
  if (view.byteLength === 0) throw Object.assign(codedError(Error, "read ENOBUFS", "ENOBUFS"), { syscall: "read" });
  return view;
}

// The range a view covered, over the store its read transferred it to; empty
// when that store is too small for it.
function continuedView(store: ArrayBufferLike, byteOffset: number, byteLength: number): Uint8Array {
  return byteOffset + byteLength <= store.byteLength ? new Uint8Array(store, byteOffset, byteLength) : new Uint8Array(0);
}

// Hands `buffer` to the onread callback, or pushes a copy; false when reading stops.
function deliverChunk(socket: Socket, buffer: Buffer): boolean {
  const callback = socket[kBufferCb];
  if (isFunction(callback)) return Reflect.apply(callback, socket, [buffer.byteLength, buffer]) !== false;
  return socket.push(Buffer.from(buffer));
}

// Each read is handled in the turn it completes, so that a stop leaves the
// loop before the stream asks for more.
async function startRead(socket: Socket): Promise<void> {
  const handle = socket._handle;
  if (!handle || handle.readLoopActive) return;
  handle.readLoopActive = true;
  try {
    while (handle.reading && socket._handle === handle) {
      const buffer = nextBuffer(socket);
      const view = readView(socket, buffer);
      const { byteOffset, byteLength } = view;
      const { value, done } = await handle.reader.read(view);
      // Data read after a destroy or reconnect is dropped.
      if (socket.destroyed || socket._handle !== handle) break;
      socket._unrefTimer();
      if (done) {
        socket.push(null);
        // EOF surfaces as 'end' at once, even when nobody is consuming the socket.
        Duplex.prototype.read.call(socket, 0);
        break;
      }
      socket[kBufferContinuations]?.set(buffer, continuedView(value.buffer, byteOffset, byteLength));
      if (value.byteLength === 0) continue;
      handle.bytesRead += value.byteLength;
      if (!deliverChunk(socket, Buffer.from(value.buffer, value.byteOffset, value.byteLength))) break;
    }
  } catch (error) {
    // A read rejected by the socket's own destroy is not news.
    if (socket._handle === handle && !socket.destroyed) socket.destroy(error);
  } finally {
    handle.readLoopActive = false;
    handle.reading = false;
  }
}

function tryReadStart(socket: Socket): void {
  if (socket._handle != null) socket._handle.reading = true;
  startRead(socket).catch(error => socket.destroy(error));
}

function writeAfterFIN(this: Socket, chunk: unknown, encoding?: unknown, cb?: unknown): boolean {
  if (!this.writableEnded) return Reflect.apply(Duplex.prototype.write, this, [chunk, encoding, cb]);
  const callback = typeof encoding === "function" ? encoding : cb;
  const error = codedError(Error, "This socket has been ended by the other party", "EPIPE");
  if (isFunction(callback)) queueMicrotask(() => callback(error));
  this.destroy(error);
  return false;
}

function onReadableStreamEnd(this: Socket): void {
  if (!this.allowHalfOpen) this.write = writeAfterFIN;
}

function getTimerDuration(msecs: unknown, name: string): number {
  validateNumber(msecs, name);
  if (msecs < 0 || !Number.isFinite(msecs)) throw outOfRange(name, "a non-negative finite number", msecs);
  return Math.min(msecs, TIMEOUT_MAX);
}

function toNumber(value: unknown): number | false {
  const number = Number(value);
  return number >= 0 ? number : false;
}

function isPipeName(value: unknown): boolean {
  return typeof value === "string" && toNumber(value) === false;
}

function normalized(options: Record<PropertyKey, unknown>, cb: AnyFunction | null): NormalizedArgs {
  const args: NormalizedArgs = [options, cb];
  args[normalizedArgsSymbol] = true;
  return args;
}

export function _normalizeArgs(args: unknown[]): NormalizedArgs {
  if (args.length === 0) return normalized({}, null);
  const [first] = args;
  let options: Record<PropertyKey, unknown> = {};
  if (typeof first === "object" && first !== null) options = objectOf(first);
  else if (isPipeName(first)) options.path = first;
  else {
    options.port = first;
    if (args.length > 1 && typeof args[1] === "string") options.host = args[1];
  }
  const cb = args.at(-1);
  return normalized(options, isFunction(cb) ? cb : null);
}

// Marks `socket` aborted once `signal` aborts.
function addClientAbortSignalOption(socket: Socket, signal: {}): void {
  validateAbortSignal(signal, "options.signal");
  const onAbort = () => { socket._aborted = true; };
  if (signal.aborted) queueMicrotask(onAbort);
  else queueMicrotask(() => addAbortListener(signal, onAbort));
}

function addAbortListener(signal: AbortSignalLike, listener: () => void): void {
  if (signal.aborted) queueMicrotask(listener);
  else signal.addEventListener("abort", listener, { once: true });
}

function isFamilyAddress(address: string, family: FamilyName): boolean {
  return ipVersion(address) === (family === "ipv4" ? 4 : 6);
}

function requireAddress(address: string, family: FamilyName): void {
  if (!isFamilyAddress(address, family)) throw invalidAddress(family);
}

function ipv4Parts(address: string): number[] | undefined {
  const parts = address.split(".").map(part => parseInt(part, 10));
  return parts.length === 4 && parts.every(part => part >= 0 && part <= 255) ? parts : undefined;
}

function ipv6Parts(address: string): number[] | undefined {
  const mapped = /^::ffff:(\d+\.\d+\.\d+\.\d+)$/i.exec(address);
  const ipv4 = mapped ? ipv4Parts(mapped[1]) : undefined;
  if (ipv4) return [0, 0, 0, 0, 0, 0xffff, (ipv4[0] << 8) | ipv4[1], (ipv4[2] << 8) | ipv4[3]];
  const parts = ipv6Groups(address).map(group => parseInt(group || "0", 16));
  return parts.length === 8 && parts.every(part => part >= 0 && part <= 0xffff) ? parts : undefined;
}

// The colon-separated groups of `address`, with "::" expanded to zero groups.
function ipv6Groups(address: string): string[] {
  const halves = address.split("::");
  if (halves.length !== 2) return address.split(":");
  const [left, right] = halves.map(half => half.split(":"));
  return [...left, ...Array<string>(Math.max(0, 8 - left.length - right.length)).fill("0"), ...right];
}

// A signed 32-bit number, the order in which BlockList compares IPv4 addresses.
function ipv4Number(address: string): number {
  const parts = ipv4Parts(address);
  if (parts === undefined) throw invalidAddress("ipv4");
  return (parts[0] << 24) | (parts[1] << 16) | (parts[2] << 8) | parts[3];
}

function ipv6Number(address: string): bigint {
  const parts = ipv6Parts(address);
  if (parts === undefined) throw invalidAddress("ipv6");
  return parts.reduce((value, part) => (value << 16n) | BigInt(part), 0n);
}

const ipv6Mask = (1n << 128n) - 1n;

type BlockRule =
  | { kind: "address"; family: FamilyName; address: string }
  | { kind: "range"; family: FamilyName; start: string; end: string }
  | { kind: "subnet"; family: FamilyName; network: string; prefix: number };
type SubnetRule = Extract<BlockRule, { kind: "subnet" }>;

function inSubnet(address: string, rule: SubnetRule): boolean {
  if (rule.prefix === 0) return true;
  if (rule.family === "ipv4") {
    const mask = (0xffffffff << (32 - rule.prefix)) >>> 0;
    return (ipv4Number(address) & mask) === (ipv4Number(rule.network) & mask);
  }
  const mask = (ipv6Mask << BigInt(128 - rule.prefix)) & ipv6Mask;
  return (ipv6Number(address) & mask) === (ipv6Number(rule.network) & mask);
}

// Whether `address` lies between `start` and `end` in `family`'s order.
function inRange(address: string, start: string, end: string, family: FamilyName): boolean {
  if (family === "ipv4") return ipv4Number(address) >= ipv4Number(start) && ipv4Number(address) <= ipv4Number(end);
  return ipv6Number(address) >= ipv6Number(start) && ipv6Number(address) <= ipv6Number(end);
}

// An IPv4 rule also matches an IPv6 address written "::ffff:" and an IPv4 address.
function ruleMatches(rule: BlockRule, address: string, family: FamilyName): boolean {
  if (rule.family !== family) return rule.family === "ipv4" && address.startsWith("::ffff:") && ruleMatches(rule, address.slice(7), "ipv4");
  if (rule.kind === "address") return rule.address === address;
  if (rule.kind === "subnet") return inSubnet(address, rule);
  return inRange(address, rule.start, rule.end, family);
}

function ruleText(rule: BlockRule): string {
  const family = rule.family === "ipv4" ? "IPv4" : "IPv6";
  if (rule.kind === "address") return `Address: ${family} ${rule.address}`;
  if (rule.kind === "range") return `Range: ${family} ${rule.start}-${rule.end}`;
  return `Subnet: ${family} ${rule.network}/${rule.prefix}`;
}

// The address and family of `value`: a SocketAddress, or an address string of `family`.
function addressOf(value: unknown, family: unknown, name: string): { address: string; family: FamilyName } {
  if (value instanceof SocketAddress) return { address: value.address, family: value.family };
  validateString(value, name);
  validateFamily(family);
  return { address: value, family };
}

// The inspection options of a custom inspection.
interface InspectDepth {
  depth?: number | null;
}

// `options` one level deeper, for the value a custom inspection formats.
function nestedInspectOptions(options: InspectDepth): InspectDepth {
  return { ...options, depth: options.depth == null ? null : options.depth - 1 };
}

// A BlockList JSON rule's family marker, pattern and the call that adds it.
const jsonRules: [Family, RegExp, (list: BlockList, match: string[]) => void][] = [
  ["IPv4", /Subnet: IPv4 (\d{1,3}(?:\.\d{1,3}){3})\/(\d{1,2})/, (list, [, network, prefix]) => list.addSubnet(network, parseInt(prefix, 10))],
  ["IPv4", /Address: IPv4 (\d{1,3}(?:\.\d{1,3}){3})/, (list, [, address]) => list.addAddress(address)],
  ["IPv4", /Range: IPv4 (\d{1,3}(?:\.\d{1,3}){3})-(\d{1,3}(?:\.\d{1,3}){3})/, (list, [, start, end]) => list.addRange(start, end)],
  ["IPv6", /Subnet: IPv6 ([0-9a-fA-F:]{1,39})\/(\d{1,3})/i, (list, [, network, prefix]) => list.addSubnet(network, parseInt(prefix, 10), "ipv6")],
  ["IPv6", /Address: IPv6 ([0-9a-fA-F:]{1,39})/i, (list, [, address]) => list.addAddress(address, "ipv6")],
  ["IPv6", /Range: IPv6 ([0-9a-fA-F:]{1,39})-([0-9a-fA-F:]{1,39})/i, (list, [, start, end]) => list.addRange(start, end, "ipv6")],
];

// Adds the first rule `item` describes to `list`; an item describing none is skipped.
function addJSONRule(list: BlockList, item: unknown): void {
  for (const [family, pattern, add] of jsonRules) {
    if (!callMethod(item, "includes", family)) continue;
    const match = callMethod(item, "match", pattern);
    if (Array.isArray(match)) {
      add(list, match);
      return;
    }
  }
}

function parseRulesJSON(data: string): unknown {
  try { return JSON.parse(data); }
  catch { throw invalidArgValue("data", data, "must be valid JSON"); }
}

export class BlockList {
  #rules: BlockRule[] = [];

  static isBlockList(value: unknown): value is BlockList { return value instanceof BlockList; }

  addAddress(address: unknown, family: unknown = "ipv4"): void {
    const rule = addressOf(address, family, "address");
    requireAddress(rule.address, rule.family);
    this.#rules.push({ kind: "address", ...rule });
  }

  addRange(start: unknown, end: unknown, family: unknown = "ipv4"): void {
    const first = addressOf(start, family, "start");
    const endIsAddress = end instanceof SocketAddress;
    if (!endIsAddress) validateString(end, "end");
    else if (start instanceof SocketAddress && end.family !== first.family) throw invalidArgValue("end", end, "must be same family as start");
    const last = endIsAddress ? end.address : end;
    requireAddress(first.address, first.family);
    requireAddress(last, first.family);
    const number = first.family === "ipv4" ? ipv4Number : ipv6Number;
    if (number(first.address) > number(last)) throw invalidArgValue("start", first.address, "must come before end");
    this.#rules.push({ kind: "range", family: first.family, start: first.address, end: last });
  }

  addSubnet(network: unknown, prefix: unknown, family: unknown = "ipv4"): void {
    const subnet = addressOf(network, family, "network");
    validateNumber(prefix, "prefix");
    if (!Number.isInteger(prefix)) throw outOfRange("prefix", "an integer", prefix);
    if (prefix !== (prefix | 0)) throw outOfRange("prefix", ">= -2147483648 && <= 2147483647", prefix);
    const bits = subnet.family === "ipv4" ? 32 : 128;
    if (prefix < 0 || prefix > bits) throw outOfRange("prefix", `between 0 and ${bits}`, prefix);
    requireAddress(subnet.address, subnet.family);
    this.#rules.push({ kind: "subnet", family: subnet.family, network: subnet.address, prefix });
  }

  check(address: unknown, family: unknown = "ipv4"): boolean {
    const candidate = addressOf(address, family, "address");
    if (!isFamilyAddress(candidate.address, candidate.family)) return false;
    return this.#rules.some(rule => ruleMatches(rule, candidate.address, candidate.family));
  }

  get rules(): string[] { return this.#rules.map(ruleText); }

  [inspect.custom](depth: number, options: InspectDepth): string {
    if (depth < 0) return "[BlockList]";
    return `BlockList {\n  rules: ${inspect(this.rules, nestedInspectOptions(options))}\n}`;
  }

  toJSON(): string[] { return this.rules; }

  fromJSON(data: unknown): void {
    const rules = typeof data === "string" ? parseRulesJSON(data) : data;
    if (!Array.isArray(rules)) throw invalidArgType("data", "Array", rules);
    // As in workerd, JSON text adds no rules: it iterates `data`, the text's characters.
    if (typeof data === "string") return;
    for (const item of rules) addJSONRule(this, item);
  }
}

export class SocketAddress {
  #address: string;
  #port: number;
  #family: FamilyName;
  #flowlabel: number;

  static isSocketAddress(value: unknown): value is SocketAddress { return value instanceof SocketAddress; }

  constructor(options: unknown = {}) {
    if (options === null || typeof options !== "object" || Array.isArray(options)) throw invalidArgType("options", "object", options);
    const settings = objectOf(options);
    const requestedFamily = settings.family || "ipv4";
    const toLowerCase = propertyOf(requestedFamily, "toLowerCase");
    const family = isFunction(toLowerCase) ? Reflect.apply(toLowerCase, requestedFamily, []) : requestedFamily;
    validateFamily(family, "options.family");
    const { address = family === "ipv4" ? "127.0.0.1" : "::", port = 0, flowlabel = 0 } = settings;
    validateString(address, "options.address");
    const portNumber = validatePort(port, "options.port");
    validateUint32(flowlabel, "options.flowlabel");
    if (!isFamilyAddress(address, family)) throw codedError(TypeError, "Invalid socket address", "ERR_INVALID_ADDRESS");
    this.#address = address;
    this.#port = portNumber;
    this.#family = family;
    this.#flowlabel = flowlabel;
  }

  get address(): string { return this.#address; }
  get port(): number { return this.#port; }
  get family(): FamilyName { return this.#family; }
  get flowlabel(): number { return this.#flowlabel; }

  [inspect.custom](depth: number, options: InspectDepth): string | this {
    if (depth < 0) return this;
    return `SocketAddress ${inspect(this.toJSON(), nestedInspectOptions(options))}`;
  }

  toJSON(): { address: string; port: number; family: FamilyName; flowlabel: number } {
    return { address: this.address, port: this.port, family: this.family, flowlabel: this.flowlabel };
  }

  // The address `input` gives as "host[:port]", or undefined when it is not an IP socket address.
  static parse(input: unknown): SocketAddress | undefined {
    validateString(input, "input");
    try {
      const url = URL.parse(`http://${input}`);
      if (url === null) return undefined;
      const port = Number(url.port) | 0;
      if (url.hostname.startsWith("[") && url.hostname.endsWith("]")) return new SocketAddress({ address: url.hostname.slice(1, -1), port, family: "ipv6" });
      return new SocketAddress({ address: url.hostname, port });
    } catch {
      return undefined;
    }
  }
}

export function connect(...args: unknown[]): Socket | undefined {
  const normalizedArgs = _normalizeArgs(args);
  const [options] = normalizedArgs;
  const socket = new Socket(options);
  if (options.timeout) socket.setTimeout(options.timeout);
  if (socket.destroyed) return socket;
  return socket.connect(normalizedArgs);
}

export const createConnection = connect;

export function createServer(options?: unknown, connectionListener?: unknown): Server {
  return new Server(options, connectionListener);
}

export function getDefaultAutoSelectFamily(): boolean {
  return false;
}

export function setDefaultAutoSelectFamily(value: unknown): void {
  if (value) throw invalidArgValue("val", value, "is invalid");
}

// Read back only; no connection uses it.
let autoSelectFamilyAttemptTimeout = 10;

export function getDefaultAutoSelectFamilyAttemptTimeout(): number {
  return autoSelectFamilyAttemptTimeout;
}

export function setDefaultAutoSelectFamilyAttemptTimeout(value: unknown): void {
  validateInt32(value, "val", 1);
  autoSelectFamilyAttemptTimeout = Math.max(value, 10);
}

export function isIP(value: unknown): number {
  return ipVersion(`${value}`);
}
export const isIPv4 = (value: unknown): boolean => isIP(value) === 4;
export const isIPv6 = (value: unknown): boolean => isIP(value) === 6;
export const Stream = Socket;

export default {
  BlockList, BoundSocket, SocketAddress, Stream, Server, Socket, connect, createConnection, createServer,
  getDefaultAutoSelectFamily, setDefaultAutoSelectFamily, getDefaultAutoSelectFamilyAttemptTimeout,
  setDefaultAutoSelectFamilyAttemptTimeout, isIP, isIPv4, isIPv6, _normalizeArgs,
};
