import EventEmitter from "../events/events.mjs";
import { connect as connectSocket } from "../builtins/cloudflare-sockets.mjs";
import { clearTimeout, setTimeout } from "../globals/timers.mjs";
import { Duplex } from "../streams/node.mjs";
import { Buffer } from "./buffer.mjs";
import { ipVersion } from "tokamak:host";
import { abortError, codedError } from "./errors.mjs";
import { GATEWAY_ADDRESS, HOST_ADDRESS, bindPort, tcpPorts, validatePort } from "./tcp-ports.mjs";
import { inspect, types } from "./util.mjs";
import { invalidArgument, invalidType, invalidValue, outOfRange, validateAbortSignal, validateFunction, validateObject, validateType } from "./validators.mjs";

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

function validateInt32(value, name, min) {
  validateType(value, name, "number");
  if (!Number.isInteger(value)) throw outOfRange(name, "an integer", value);
  if (value < min || value > 2147483647) throw outOfRange(name, `>= ${min} && <= 2147483647`, value);
}

function validateFamily(family, name = "family") {
  if (family !== "ipv4" && family !== "ipv6") throw invalidValue(name, family, "must be one of: 'ipv4', 'ipv6'");
}

function validateUint32(value, name) {
  validateType(value, name, "number");
  if (!Number.isInteger(value)) throw outOfRange(name, "an integer", value);
  if (value < 0 || value > 0xffffffff) throw outOfRange(name, ">= 0 && < 4294967296", value);
}

function invalidIPAddress(address) {
  return invalidArgument(`Invalid IP address: ${address}`, "ERR_INVALID_IP_ADDRESS");
}

function invalidAddress(family) {
  return invalidIPAddress(`Invalid ${family === "ipv4" ? "IPv4" : "IPv6"} address`);
}

function socketClosed() {
  return codedError(Error, "Socket is closed", "ERR_SOCKET_CLOSED");
}

function handleAdopted() {
  return codedError(Error, "The bound socket has already been adopted by a server or socket", "ERR_SOCKET_HANDLE_ADOPTED");
}

function familyOf(address) {
  return { 4: "IPv4", 6: "IPv6" }[isIP(address)];
}

function isWildcard(address) {
  return address === DEFAULT_IPV4_ADDR || address === DEFAULT_IPV6_ADDR;
}

// A synthetic IPv4 address in the form `family` reports it.
function forFamily(address, family) {
  return family === "IPv6" ? `::ffff:${address}` : address;
}

// The address a socket bound to the wildcard reports once connected or
// accepted: the isolate's host address.
function concreteLocal(bound) {
  return isWildcard(bound.address) ? { ...bound, address: forFamily(HOST_ADDRESS, bound.family) } : bound;
}

function releaseBoundSource(socket) {
  if (socket[kBoundReserved]) {
    tcpPorts.unregister(socket);
    tcpPorts.release(socket[kBoundSource].port);
    socket[kBoundReserved] = false;
  }
  socket[kBoundSource] = null;
}

// The address of a "host:port" or "[v6]:port" authority; "*" is the wildcard.
function parseAuthority(authority) {
  const match = typeof authority === "string" ? /^\[?([^\]]*?)\]?:(\d+)$/.exec(authority) : null;
  if (match === null) return null;
  const address = match[1] === "*" ? DEFAULT_IPV4_ADDR : match[1];
  const info = { address, port: Number(match[2]) };
  const family = familyOf(address);
  if (family !== undefined) info.family = family;
  return info;
}

// The `_handle` of a socket over the cloudflare:sockets socket `socket`.
function socketHandle(socket, options) {
  return {
    socket, writer: socket.writable.getWriter(), reader: socket.readable.getReader({ mode: "byob" }),
    bytesRead: 0, bytesWritten: 0, reading: false, options,
  };
}

// A reserved port in the isolate's port table; the Socket or Server adopting it
// takes over the reservation and reports the bound address as its own.
export class BoundSocket {
  #address;
  // Bound with port 0.
  #ephemeral;

  static isBoundSocket(value) {
    return typeof value === "object" && value !== null && #address in value;
  }

  constructor(options = {}) {
    validateObject(options, "options");
    if (options.path !== undefined) throw invalidValue("options.path", options.path, "is not supported");
    const port = validatePort(options.port ?? 0, "options.port");
    this.#ephemeral = port === 0;
    const ipv6Only = options.ipv6Only ?? false;
    validateType(ipv6Only, "options.ipv6Only", "boolean");
    const reusePort = options.reusePort ?? false;
    validateType(reusePort, "options.reusePort", "boolean");
    let { host } = options;
    let addressType = ipv6Only ? 6 : 4;
    if (host === undefined || host === null) host = ipv6Only ? DEFAULT_IPV6_ADDR : DEFAULT_IPV4_ADDR;
    else {
      validateType(host, "options.host", "string");
      addressType = isIP(host);
      if (addressType === 0) throw invalidValue("options.host", host, "must be a numeric IP address; net.BoundSocket does not perform DNS resolution");
    }
    this.#address = { address: host, family: addressType === 6 ? "IPv6" : "IPv4", port: bindPort(host, port, reusePort) };
    tcpPorts.register(this, this.#address.port);
  }

  address() {
    if (this.#address === null) throw handleAdopted();
    return this.#address;
  }

  // Workers have no file descriptors.
  fd() {
    if (this.#address === null) throw handleAdopted();
    return -1;
  }

  close() {
    if (this.#address === null) throw handleAdopted();
    tcpPorts.unregister(this);
    tcpPorts.release(this.#address.port);
    this.#address = null;
  }

  [Symbol.dispose]() {
    if (this.#address !== null) this.close();
  }

  // Transfers the reservation to the adopter.
  [kBoundSocketConsume]() {
    if (this.#address === null) throw handleAdopted();
    tcpPorts.unregister(this);
    const address = this.#address;
    this.#address = null;
    return { address, ephemeral: this.#ephemeral };
  }
}

// The host, port and reusePort of a server's listen() options.
function listenOptions(options) {
  if (options.path != null) throw invalidValue("options.path", options.path, "is not supported");
  if (options.handle != null || options.fd != null) throw invalidValue("options", options, "only a net.BoundSocket handle is supported");
  const port = options.port == null ? 0 : validatePort(options.port, "options.port");
  const reusePort = options.reusePort ?? false;
  validateType(reusePort, "options.reusePort", "boolean");
  let { host } = options;
  if (host === undefined) host = options.ipv6Only ? DEFAULT_IPV6_ADDR : DEFAULT_IPV4_ADDR;
  else {
    validateType(host, "options.host", "string");
    if (host === "localhost") host = "127.0.0.1";
  }
  return { host, port, reusePort };
}

// A listening server reserves its port in the isolate's port table and
// installs a connect handler there; cloudflare:node routes inbound sockets to
// it by local port and wraps them as net.Sockets.
export class Server extends EventEmitter {
  #address = null;
  #connections = 0;
  #closing = false;
  #handler = { connect: socket => this.#onConnection(socket) };
  allowHalfOpen;
  pauseOnConnect;
  noDelay;
  keepAlive;
  keepAliveInitialDelay;
  highWaterMark;
  maxConnections = undefined;

  constructor(options, connectionListener) {
    super();
    if (typeof options === "function") {
      connectionListener = options;
      options = {};
    } else if (options == null) options = {};
    else validateObject(options, "options");
    if (connectionListener !== undefined) {
      validateFunction(connectionListener, "connectionListener");
      this.on("connection", connectionListener);
    }
    let { keepAliveInitialDelay } = options;
    if (keepAliveInitialDelay !== undefined) {
      validateType(keepAliveInitialDelay, "options.keepAliveInitialDelay", "number");
      if (keepAliveInitialDelay < 0) keepAliveInitialDelay = 0;
    }
    this.allowHalfOpen = options.allowHalfOpen ?? false;
    this.pauseOnConnect = Boolean(options.pauseOnConnect);
    this.noDelay = Boolean(options.noDelay);
    this.keepAlive = Boolean(options.keepAlive);
    this.keepAliveInitialDelay = ~~((keepAliveInitialDelay ?? 0) / 1000);
    this.highWaterMark = options.highWaterMark;
  }

  listen(...args) {
    const [options, callback] = _normalizeArgs(args);
    if (this.#address !== null) throw codedError(Error, "Listen method has been called more than once without closing.", "ERR_SERVER_ALREADY_LISTEN");
    if (callback !== null) this.once("listening", callback);
    let address;
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

  address() {
    return this.#address === null ? null : { ...this.#address };
  }

  get listening() {
    return this.#address !== null;
  }

  close(callback) {
    if (typeof callback === "function") {
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
  #maybeEmitClose() {
    if (!this.#closing || this.#address !== null || this.#connections !== 0) return;
    this.#closing = false;
    queueMicrotask(() => this.emit("close"));
  }

  getConnections(callback) {
    validateFunction(callback, "cb");
    const count = this.#connections;
    queueMicrotask(() => callback(null, count));
    return this;
  }

  ref() { return this; }
  unref() { return this; }

  async [Symbol.asyncDispose]() {
    if (this.#address === null) return;
    const { promise, resolve } = Promise.withResolvers();
    this.close(() => resolve());
    await promise;
  }

  // Accepts an inbound socket: its local endpoint is the server's address, and
  // a peer the platform does not report appears behind the gateway address.
  async #onConnection(handle) {
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
    socket._handle = socketHandle(handle, { host: remoteAddress.address, port: remoteAddress.port, addressType: isIP(remoteAddress.address) });
    socket[kSocketInfo] = { remoteAddress: { ...remoteAddress } };
    socket[kBoundSource] = localAddress;
    socket.server = this;
    socket._server = this;
    socket.connecting = false;
    socket._undestroy();
    this.#connections++;
    const closed = new Promise(resolve => socket.once("close", () => resolve()));
    handle.closed.then(onConnectionClosed.bind(socket), error => socket.destroy(error));
    if (this.pauseOnConnect) socket.pause();
    this.emit("connection", socket);
    if (!socket.isPaused() && !socket._handle.reading) tryReadStart(socket);
    await closed;
    this.#connections--;
    this.#maybeEmitClose();
  }
}

// Sets how `socket` reads: into the buffers `onread` hands out, delivered to its
// callback, or into fresh buffers pushed to the stream.
function setReadBuffer(socket, onread) {
  const custom = onread != null && typeof onread === "object"
    && (types.isUint8Array(onread.buffer) || typeof onread.buffer === "function") && typeof onread.callback === "function";
  if (!custom) {
    socket[kBuffer] = true;
    socket[kBufferGen] = () => new Uint8Array(4096);
    socket[kBufferCb] = undefined;
    return;
  }
  if (typeof onread.buffer === "function") socket[kBufferGen] = onread.buffer;
  else {
    const { buffer } = onread;
    socket[kBufferGen] = () => buffer;
  }
  socket[kBuffer] = true;
  // BYOB reads transfer the buffers they are handed, so a reused buffer is
  // continued over the store it moved to.
  socket[kBufferContinuations] = new WeakMap();
  socket[kBufferCb] = onread.callback;
}

function adoptHandle(socket, handle) {
  validateObject(handle, "options.handle");
  if (!BoundSocket.isBoundSocket(handle)) {
    socket._handle = handle;
    return;
  }
  const { address } = handle[kBoundSocketConsume]();
  socket[kBoundSource] = address;
  socket[kBoundReserved] = true;
  tcpPorts.register(socket, address.port);
}

export function Socket(options) {
  if (!(this instanceof Socket)) return new Socket(options);
  if (options?.objectMode) throw invalidValue("options.objectMode", options.objectMode, "is not supported");
  if (options?.readableObjectMode || options?.writableObjectMode) {
    const name = options.readableObjectMode ? "readableObjectMode" : "writableObjectMode";
    throw invalidValue(`options.${name}`, options.readableObjectMode || options.writableObjectMode, "is not supported");
  }
  if (options?.keepAliveInitialDelay !== undefined) {
    validateType(options.keepAliveInitialDelay, "options.keepAliveInitialDelay", "number");
    if (options.keepAliveInitialDelay < 0) options.keepAliveInitialDelay = 0;
  }
  options = typeof options === "number" ? { fd: options } : { ...options };
  if (options.fd !== undefined) throw codedError(Error, "The options.fd option is not implemented", "ERR_OPTION_NOT_IMPLEMENTED");
  options.allowHalfOpen = Boolean(options.allowHalfOpen);
  // The socket emits 'close' itself, with hadError.
  options.emitClose = false;
  options.autoDestroy = true;
  options.decodeStrings = false;
  options.readable = true;
  options.writable = true;
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
  Duplex.call(this, options);
  if (options.handle) adoptHandle(this, options.handle);
  // Every 'end', since a socket can reconnect.
  this.on("end", onReadableStreamEnd);
  setReadBuffer(this, options.onread);
  if (options.signal) addClientAbortSignalOption(this, options.signal);
  this[kBytesRead] = 0;
  this[kBytesWritten] = 0;
  return this;
}
Object.setPrototypeOf(Socket.prototype, Duplex.prototype);
Object.setPrototypeOf(Socket, Duplex);

// Restarts the idle timer of this socket and of any socket wrapping it.
Socket.prototype._unrefTimer = function _unrefTimer() {
  for (let socket = this; socket != null; socket = socket._parentWrap) {
    if (socket[kTimeout] == null) continue;
    clearTimeout(socket[kTimeout]);
    socket[kTimeout] = setTimeout(() => socket._onTimeout(), socket.timeout);
  }
};

// Disables the idle timer of `socket` and of any socket wrapping it.
function clearSocketTimers(socket) {
  for (let current = socket; current != null; current = current._parentWrap) {
    clearTimeout(current[kTimeout]);
    current[kTimeout] = null;
  }
}

Socket.prototype.setTimeout = function (msecs, callback) {
  if (this.destroyed) return this;
  this.timeout = msecs;
  msecs = getTimerDuration(msecs, "msecs");
  clearTimeout(this[kTimeout]);
  this[kTimeout] = null;
  if (msecs !== 0) this[kTimeout] = setTimeout(() => this._onTimeout(), msecs);
  if (callback === undefined) return this;
  validateFunction(callback, "callback");
  if (msecs === 0) this.removeListener("timeout", callback);
  else this.once("timeout", callback);
  return this;
};

Socket.prototype._onTimeout = function () {
  const handle = this._handle;
  const lastWriteQueueSize = this[kLastWriteQueueSize];
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
function concatenatedChunks(chunks) {
  return Buffer.concat(chunks.map(({ chunk, encoding }) => {
    if (typeof chunk === "string") return Buffer.from(chunk, encoding);
    if (types.isArrayBufferView(chunk)) return new Uint8Array(chunk.buffer, chunk.byteOffset, chunk.byteLength);
    throw invalidType("chunk", "of type string or an instance of ArrayBufferView", chunk);
  }));
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
    const bytes = writev ? concatenatedChunks(data) : typeof data === "string" ? Buffer.from(data, encoding) : data;
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
};

Socket.prototype._writev = function (chunks, cb) {
  this._writeGeneric(true, chunks, "", cb);
};

Socket.prototype._write = function (data, encoding, cb) {
  this._writeGeneric(false, data, encoding, cb);
};

Socket.prototype._final = function (cb) {
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

Socket.prototype.end = function (data, encoding, cb) {
  Duplex.prototype.end.call(this, data, encoding, cb);
  return this;
};

Socket.prototype.pause = function () {
  // Stops the read loop after its current read.
  if (this[kBuffer] && !this.connecting && this._handle?.reading && !this.destroyed) this._handle.reading = false;
  return Duplex.prototype.pause.call(this);
};

Socket.prototype.resume = function () {
  if (this[kBuffer] && !this.connecting && this._handle && !this._handle.reading) tryReadStart(this);
  return Duplex.prototype.resume.call(this);
};

Socket.prototype.read = function (n) {
  if (this[kBuffer] && !this.connecting && this._handle && !this._handle.reading) tryReadStart(this);
  return Duplex.prototype.read.call(this, n);
};

Socket.prototype._read = function (n) {
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

Socket.prototype._destroy = function (exception, cb) {
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
function reconnect(socket, options) {
  releaseBoundSource(socket);
  const previous = socket._handle;
  socket._handle = null;
  previous.reading = false;
  socket.connecting = true;
  const connectAgain = () => initializeConnection(socket, options);
  previous.socket.close().then(connectAgain, connectAgain);
}

Socket.prototype.connect = function (...args) {
  const normalized = Array.isArray(args[0]) && args[0][normalizedArgsSymbol] ? args[0] : _normalizeArgs(args);
  const [options, cb] = normalized;
  if (this.connecting) throw codedError(Error, "Socket is already connecting", "ERR_SOCKET_CONNECTING");
  if (this._aborted) {
    if (!cb) throw abortError();
    cb(abortError());
    return undefined;
  }
  if (cb !== null) this.once("connect", cb);
  if (this._parentWrap?.connecting) return this;
  if (options.port === undefined && options.path == null) {
    throw invalidArgument("The \"options\" or \"port\" or \"path\" argument must be specified", "ERR_MISSING_ARGS");
  }
  if (this.write !== Socket.prototype.write) this.write = Socket.prototype.write;
  if (options.path != null) throw invalidValue("path", options.path, "is not supported");
  this[kBytesRead] = 0;
  this[kBytesWritten] = 0;
  if (this._handle) reconnect(this, options);
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
function bytesWritten(socket) {
  const data = socket._pendingData;
  if (!socket.writableBuffer) return undefined;
  if (!data) return socket.writableLength + (socket._handle != null ? socket._handle.bytesWritten : socket[kBytesWritten]);
  if (!Array.isArray(data)) return socket._bytesDispatched + (typeof data === "string" ? Buffer.byteLength(data, socket._pendingEncoding) : data.length);
  let bytes = socket._bytesDispatched;
  for (const chunk of data) {
    if (chunk == null) continue;
    bytes += chunk instanceof Buffer || data.allBuffers ? chunk.length : Buffer.byteLength(chunk.chunk, chunk.encoding);
  }
  return bytes;
}

function enumerableGetter(get) {
  return { __proto__: null, configurable: false, enumerable: true, get };
}

Object.defineProperties(Socket.prototype, {
  _connecting: { __proto__: null, get() { return this.connecting; } },
  pending: { __proto__: null, get() { return !this._handle || this.connecting; }, configurable: true },
  readyState: {
    __proto__: null,
    get() {
      if (this.connecting) return "opening";
      if (this.readable && this.writable) return "open";
      if (this.readable) return "readOnly";
      return this.writable ? "writeOnly" : "closed";
    },
  },
  bufferSize: { __proto__: null, get() { return this._handle ? this.writableLength : undefined; } },
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
function cleanupAfterDestroy(socket, cb, error) {
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

function initializeConnection(socket, options) {
  const { host = "localhost", family, hints, autoSelectFamily, autoSelectFamilyAttemptTimeout, lookup, localAddress, localPort } = options;
  let { port } = options;
  if (socket[kBoundSource] != null && (localAddress !== undefined || localPort !== undefined)) {
    throw invalidValue("options", options, "localAddress and localPort cannot be used with an adopted bound socket");
  }
  if (localAddress && !isIP(localAddress)) throw invalidIPAddress(localAddress);
  if (localPort) validateType(localPort, "options.localPort", "number");
  if (typeof port !== "number" && typeof port !== "string") throw invalidType("options.port", "one of type number or string", port);
  validatePort(port);
  port |= 0;
  if (autoSelectFamily != null) validateType(autoSelectFamily, "options.autoSelectFamily", "boolean");
  if (autoSelectFamilyAttemptTimeout != null) validateInt32(autoSelectFamilyAttemptTimeout, "options.autoSelectFamilyAttemptTimeout", 1);
  socket._unrefTimer();
  socket.connecting = true;
  if (lookup != null) validateFunction(lookup, "options.lookup");
  const addressType = isIP(host);
  const connection = { localAddress, localPort, port, addressType };
  if (addressType !== 0 || lookup == null) {
    openConnection(socket, connection, host, addressType);
    return;
  }
  // The lookup function resolves a host that is not an IP address.
  lookup(host, { family: family || addressType, hints }, (error, address, resolvedFamily) => {
    socket.emit("lookup", error, address, resolvedFamily, host);
    if (error) {
      socket.destroy(error);
      return;
    }
    if (isIP(address) === 0) throw invalidIPAddress(address);
    if (![4, 6, "IPv4", "IPv6"].includes(resolvedFamily)) throw invalidValue("family", resolvedFamily, "must be 4 or 6");
    openConnection(socket, connection, address, resolvedFamily);
  });
}

// Autobinds the local endpoint, as connect(2) does; only a nonzero localPort
// reserves a port, and localAddress alone is a label.
function autobind(socket, localAddress, localPort, family) {
  const address = localAddress ?? (family === 6 ? DEFAULT_IPV6_ADDR : DEFAULT_IPV4_ADDR);
  const reserved = localPort != null && localPort !== 0;
  socket[kBoundSource] = { address, family: isIP(address) === 6 ? "IPv6" : "IPv4", port: reserved ? bindPort(address, localPort) : tcpPorts.ephemeral() };
  if (!reserved) return;
  socket[kBoundReserved] = true;
  tcpPorts.register(socket, socket[kBoundSource].port);
}

function openConnection(socket, { localAddress, localPort, port, addressType }, host, family) {
  socket[kSocketInfo] = { remoteAddress: { address: host, port, family: family === 4 ? "IPv4" : family === 6 ? "IPv6" : undefined } };
  const authority = family === 6 ? `[${host}]` : `${host}`;
  socket.autoSelectFamilyAttemptedAddresses = [`${authority}:${port}`];
  socket.emit("connectionAttempt", authority, port, addressType);
  socket._host = authority;
  try {
    if (socket[kBoundSource] == null) autobind(socket, localAddress, localPort, family);
    // The platform does not report an outbound socket's source address.
    socket[kBoundSource] = concreteLocal(socket[kBoundSource]);
    // The Duplex owns the half-open policy, and a Node socket can always be upgraded to TLS.
    const handle = connectSocket(`${authority}:${port}`, { allowHalfOpen: true, secureTransport: socket.encrypted ? "on" : "starttls" });
    socket._handle = socketHandle(handle, { host: socket._host, port, addressType });
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

function onConnectionOpened() {
  if (this.destroyed) return;
  this.connecting = false;
  this._unrefTimer();
  this.emit("connect");
  this.emit("ready");
  if (!this.isPaused()) tryReadStart(this);
}

function onConnectionClosed() {
  const handle = this._handle;
  if (handle?.socket.upgraded) return;
  clearSocketTimers(this);
  // A pending read reports the connection's end or failure itself.
  if (this.destroyed || handle == null || handle.readLoopActive) return;
  // Nobody is reading, so the readable ends here.
  this.push(null);
  this.read(0);
}

// The view a read loop reads into: `buffer`, or its continuation over the
// store that a previous read transferred it to.
function readView(socket, buffer) {
  if (!types.isUint8Array(buffer)) throw invalidType("onread.buffer", "an instance of Buffer or Uint8Array", buffer);
  const view = socket[kBufferContinuations]?.get(buffer) ?? buffer;
  if (view.byteLength === 0) throw Object.assign(codedError(Error, "read ENOBUFS", "ENOBUFS"), { syscall: "read" });
  return view;
}

// The range a view covered, over the store its read transferred it to; empty
// when that store is too small for it.
function continuedView(store, byteOffset, byteLength) {
  return byteOffset + byteLength <= store.byteLength ? new Uint8Array(store, byteOffset, byteLength) : new Uint8Array(0);
}

// Hands `buffer` to the onread callback, or pushes a copy; false when reading stops.
function deliverChunk(socket, buffer) {
  if (typeof socket[kBufferCb] === "function") return socket[kBufferCb](buffer.byteLength, buffer) !== false;
  return socket.push(Buffer.from(buffer));
}

// Each read is handled in the turn it completes, so that a stop leaves the
// loop before the stream asks for more.
async function startRead(socket) {
  const handle = socket._handle;
  if (!handle || handle.readLoopActive) return;
  handle.readLoopActive = true;
  try {
    while (handle.reading && socket._handle === handle) {
      const generatedBuffer = socket[kBufferGen]?.();
      const view = readView(socket, generatedBuffer);
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
      socket[kBufferContinuations]?.set(generatedBuffer, continuedView(value.buffer, byteOffset, byteLength));
      if (value.byteLength === 0) continue;
      handle.bytesRead += value.byteLength;
      if (!deliverChunk(socket, Buffer.from(value.buffer, value.byteOffset, value.byteLength))) break;
    }
  } catch (error) {
    // A read rejected by the socket's own destroy or TLS upgrade is not news.
    if (socket._handle === handle && !handle.handedOver && !socket.destroyed) socket.destroy(error);
  } finally {
    handle.readLoopActive = false;
    handle.reading = false;
  }
}

function tryReadStart(socket) {
  if (socket._handle != null) socket._handle.reading = true;
  startRead(socket).catch(error => socket.destroy(error));
}

function writeAfterFIN(chunk, encoding, cb) {
  if (!this.writableEnded) return Duplex.prototype.write.call(this, chunk, encoding, cb);
  if (typeof encoding === "function") cb = encoding;
  const error = codedError(Error, "This socket has been ended by the other party", "EPIPE");
  if (typeof cb === "function") queueMicrotask(() => cb(error));
  this.destroy(error);
  return false;
}

function onReadableStreamEnd() {
  if (!this.allowHalfOpen) this.write = writeAfterFIN;
}

function getTimerDuration(msecs, name) {
  validateType(msecs, name, "number");
  if (msecs < 0 || !Number.isFinite(msecs)) throw outOfRange(name, "a non-negative finite number", msecs);
  return Math.min(msecs, TIMEOUT_MAX);
}

function toNumber(value) {
  const number = Number(value);
  return number >= 0 ? number : false;
}

function isPipeName(value) {
  return typeof value === "string" && toNumber(value) === false;
}

export function _normalizeArgs(args) {
  let normalized;
  if (args.length === 0) {
    normalized = [{}, null];
    normalized[normalizedArgsSymbol] = true;
    return normalized;
  }
  const [first] = args;
  let options = {};
  if (typeof first === "object" && first !== null) options = first;
  else if (isPipeName(first)) options.path = first;
  else {
    options.port = first;
    if (args.length > 1 && typeof args[1] === "string") options.host = args[1];
  }
  const cb = args.at(-1);
  normalized = [options, typeof cb === "function" ? cb : null];
  normalized[normalizedArgsSymbol] = true;
  return normalized;
}

// Marks `socket` aborted once `signal` aborts.
function addClientAbortSignalOption(socket, signal) {
  validateAbortSignal(signal, "options.signal");
  const onAbort = () => { socket._aborted = true; };
  if (signal.aborted) queueMicrotask(onAbort);
  else queueMicrotask(() => addAbortListener(signal, onAbort));
}

function addAbortListener(signal, listener) {
  if (signal.aborted) queueMicrotask(listener);
  else signal.addEventListener("abort", listener, { once: true });
}

function isFamilyAddress(address, family) {
  return ipVersion(address) === (family === "ipv4" ? 4 : 6);
}

function requireAddress(address, family) {
  if (!isFamilyAddress(address, family)) throw invalidAddress(family);
}

function ipv4Parts(address) {
  const parts = address.split(".").map(part => parseInt(part, 10));
  return parts.length === 4 && parts.every(part => part >= 0 && part <= 255) ? parts : undefined;
}

function ipv6Parts(address) {
  const mapped = /^::ffff:(\d+\.\d+\.\d+\.\d+)$/i.exec(address);
  const ipv4 = mapped ? ipv4Parts(mapped[1]) : undefined;
  if (ipv4) return [0, 0, 0, 0, 0, 0xffff, (ipv4[0] << 8) | ipv4[1], (ipv4[2] << 8) | ipv4[3]];
  const parts = ipv6Groups(address).map(group => parseInt(group || "0", 16));
  return parts.length === 8 && parts.every(part => part >= 0 && part <= 0xffff) ? parts : undefined;
}

// The colon-separated groups of `address`, with "::" expanded to zero groups.
function ipv6Groups(address) {
  const halves = address.split("::");
  if (halves.length !== 2) return address.split(":");
  const [left, right] = halves.map(half => half.split(":"));
  return [...left, ...Array(Math.max(0, 8 - left.length - right.length)).fill("0"), ...right];
}

// A signed 32-bit number, the order in which BlockList compares IPv4 addresses.
function ipv4Number(address) {
  const parts = ipv4Parts(address);
  if (parts === undefined) throw invalidAddress("ipv4");
  return (parts[0] << 24) | (parts[1] << 16) | (parts[2] << 8) | parts[3];
}

function ipv6Number(address) {
  const parts = ipv6Parts(address);
  if (parts === undefined) throw invalidAddress("ipv6");
  return parts.reduce((value, part) => (value << 16n) | BigInt(part), 0n);
}

const ipv6Mask = (1n << 128n) - 1n;

function inSubnet(address, rule) {
  if (rule.prefix === 0) return true;
  if (rule.family === "ipv4") {
    const mask = (0xffffffff << (32 - rule.prefix)) >>> 0;
    return (ipv4Number(address) & mask) === (ipv4Number(rule.network) & mask);
  }
  const mask = (ipv6Mask << BigInt(128 - rule.prefix)) & ipv6Mask;
  return (ipv6Number(address) & mask) === (ipv6Number(rule.network) & mask);
}

// An IPv4 rule also matches an IPv6 address written "::ffff:" and an IPv4 address.
function ruleMatches(rule, address, family) {
  if (rule.family !== family) return rule.family === "ipv4" && address.startsWith("::ffff:") && ruleMatches(rule, address.slice(7), "ipv4");
  if (rule.kind === "address") return rule.address === address;
  if (rule.kind === "subnet") return inSubnet(address, rule);
  const number = family === "ipv4" ? ipv4Number : ipv6Number;
  return number(address) >= number(rule.start) && number(address) <= number(rule.end);
}

function ruleText(rule) {
  const family = rule.family === "ipv4" ? "IPv4" : "IPv6";
  if (rule.kind === "address") return `Address: ${family} ${rule.address}`;
  if (rule.kind === "range") return `Range: ${family} ${rule.start}-${rule.end}`;
  return `Subnet: ${family} ${rule.network}/${rule.prefix}`;
}

// The address and family of `value`: a SocketAddress, or an address string of `family`.
function addressOf(value, family, name) {
  if (value instanceof SocketAddress) return { address: value.address, family: value.family };
  validateType(value, name, "string");
  validateFamily(family);
  return { address: value, family };
}

// `options` one level deeper, for the value a custom inspection formats.
function nestedInspectOptions(options) {
  return { ...options, depth: options.depth == null ? null : options.depth - 1 };
}

// A BlockList JSON rule's family marker, pattern and the call that adds it.
const jsonRules = [
  ["IPv4", /Subnet: IPv4 (\d{1,3}(?:\.\d{1,3}){3})\/(\d{1,2})/, (list, [, network, prefix]) => list.addSubnet(network, parseInt(prefix, 10))],
  ["IPv4", /Address: IPv4 (\d{1,3}(?:\.\d{1,3}){3})/, (list, [, address]) => list.addAddress(address)],
  ["IPv4", /Range: IPv4 (\d{1,3}(?:\.\d{1,3}){3})-(\d{1,3}(?:\.\d{1,3}){3})/, (list, [, start, end]) => list.addRange(start, end)],
  ["IPv6", /Subnet: IPv6 ([0-9a-fA-F:]{1,39})\/(\d{1,3})/i, (list, [, network, prefix]) => list.addSubnet(network, parseInt(prefix, 10), "ipv6")],
  ["IPv6", /Address: IPv6 ([0-9a-fA-F:]{1,39})/i, (list, [, address]) => list.addAddress(address, "ipv6")],
  ["IPv6", /Range: IPv6 ([0-9a-fA-F:]{1,39})-([0-9a-fA-F:]{1,39})/i, (list, [, start, end]) => list.addRange(start, end, "ipv6")],
];

// Adds the first rule `item` describes to `list`; an item describing none is skipped.
function addJSONRule(list, item) {
  for (const [family, pattern, add] of jsonRules) {
    const match = item.includes(family) && item.match(pattern);
    if (match) return add(list, match);
  }
}

function parseRulesJSON(data) {
  try { return JSON.parse(data); }
  catch { throw invalidValue("data", data, "must be valid JSON"); }
}

export class BlockList {
  #rules = [];

  static isBlockList(value) { return value instanceof BlockList; }

  addAddress(address, family = "ipv4") {
    const rule = addressOf(address, family, "address");
    requireAddress(rule.address, rule.family);
    this.#rules.push({ kind: "address", ...rule });
  }

  addRange(start, end, family = "ipv4") {
    const first = addressOf(start, family, "start");
    const endIsAddress = end instanceof SocketAddress;
    if (!endIsAddress) validateType(end, "end", "string");
    else if (start instanceof SocketAddress && end.family !== first.family) throw invalidValue("end", end, "must be same family as start");
    const last = endIsAddress ? end.address : end;
    requireAddress(first.address, first.family);
    requireAddress(last, first.family);
    const number = first.family === "ipv4" ? ipv4Number : ipv6Number;
    if (number(first.address) > number(last)) throw invalidValue("start", first.address, "must come before end");
    this.#rules.push({ kind: "range", family: first.family, start: first.address, end: last });
  }

  addSubnet(network, prefix, family = "ipv4") {
    const subnet = addressOf(network, family, "network");
    validateType(prefix, "prefix", "number");
    if (!Number.isInteger(prefix)) throw outOfRange("prefix", "an integer", prefix);
    if (prefix !== (prefix | 0)) throw outOfRange("prefix", ">= -2147483648 && <= 2147483647", prefix);
    const bits = subnet.family === "ipv4" ? 32 : 128;
    if (prefix < 0 || prefix > bits) throw outOfRange("prefix", `between 0 and ${bits}`, prefix);
    requireAddress(subnet.address, subnet.family);
    this.#rules.push({ kind: "subnet", family: subnet.family, network: subnet.address, prefix });
  }

  check(address, family = "ipv4") {
    const candidate = addressOf(address, family, "address");
    if (!isFamilyAddress(candidate.address, candidate.family)) return false;
    return this.#rules.some(rule => ruleMatches(rule, candidate.address, candidate.family));
  }

  get rules() { return this.#rules.map(ruleText); }

  [inspect.custom](depth, options) {
    if (depth < 0) return "[BlockList]";
    return `BlockList {\n  rules: ${inspect(this.rules, nestedInspectOptions(options))}\n}`;
  }

  toJSON() { return this.rules; }

  // Iterating `data` rather than the parsed rules makes JSON text add no rules.
  fromJSON(data) {
    const rules = typeof data === "string" ? parseRulesJSON(data) : data;
    if (!Array.isArray(rules)) throw invalidType("data", "an instance of Array", rules);
    for (const item of data) addJSONRule(this, item);
  }
}

export class SocketAddress {
  #address;
  #port;
  #family;
  #flowlabel;

  static isSocketAddress(value) { return value instanceof SocketAddress; }

  constructor(options = {}) {
    if (options === null || typeof options !== "object" || Array.isArray(options)) throw invalidType("options", "of type object", options);
    const requestedFamily = options.family || "ipv4";
    const family = typeof requestedFamily.toLowerCase === "function" ? requestedFamily.toLowerCase() : requestedFamily;
    validateFamily(family, "options.family");
    const { address = family === "ipv4" ? "127.0.0.1" : "::", port = 0, flowlabel = 0 } = options;
    validateType(address, "options.address", "string");
    const portNumber = validatePort(port, "options.port");
    validateUint32(flowlabel, "options.flowlabel");
    if (!isFamilyAddress(address, family)) throw invalidArgument("Invalid socket address", "ERR_INVALID_ADDRESS");
    this.#address = address;
    this.#port = portNumber;
    this.#family = family;
    this.#flowlabel = flowlabel;
  }

  get address() { return this.#address; }
  get port() { return this.#port; }
  get family() { return this.#family; }
  get flowlabel() { return this.#flowlabel; }

  [inspect.custom](depth, options) {
    if (depth < 0) return this;
    return `SocketAddress ${inspect(this.toJSON(), nestedInspectOptions(options))}`;
  }

  toJSON() { return { address: this.address, port: this.port, family: this.family, flowlabel: this.flowlabel }; }

  // The address `input` gives as "host[:port]", or undefined when it is not an IP socket address.
  static parse(input) {
    validateType(input, "input", "string");
    try {
      const url = URL.parse(`http://${input}`);
      if (url === null) return undefined;
      const port = url.port | 0;
      if (url.hostname.startsWith("[") && url.hostname.endsWith("]")) return new SocketAddress({ address: url.hostname.slice(1, -1), port, family: "ipv6" });
      return new SocketAddress({ address: url.hostname, port });
    } catch {
      return undefined;
    }
  }
}

export function connect(...args) {
  const normalized = _normalizeArgs(args);
  const [options] = normalized;
  const socket = new Socket(options);
  if (options.timeout) socket.setTimeout(options.timeout);
  if (socket.destroyed) return socket;
  return socket.connect(normalized);
}

export const createConnection = connect;

export function createServer(options, connectionListener) {
  return new Server(options, connectionListener);
}

export function getDefaultAutoSelectFamily() {
  return false;
}

export function setDefaultAutoSelectFamily(value) {
  if (value) throw invalidValue("val", value, "is invalid");
}

// Read back only; no connection uses it.
let autoSelectFamilyAttemptTimeout = 10;

export function getDefaultAutoSelectFamilyAttemptTimeout() {
  return autoSelectFamilyAttemptTimeout;
}

export function setDefaultAutoSelectFamilyAttemptTimeout(value) {
  validateInt32(value, "val", 1);
  autoSelectFamilyAttemptTimeout = Math.max(value, 10);
}

export function isIP(value) {
  return ipVersion(`${value}`);
}
export const isIPv4 = value => isIP(value) === 4;
export const isIPv6 = value => isIP(value) === 6;
export const Stream = Socket;

export default {
  BlockList, BoundSocket, SocketAddress, Stream, Server, Socket, connect, createConnection, createServer,
  getDefaultAutoSelectFamily, setDefaultAutoSelectFamily, getDefaultAutoSelectFamilyAttemptTimeout,
  setDefaultAutoSelectFamilyAttemptTimeout, isIP, isIPv4, isIPv6, _normalizeArgs,
};
