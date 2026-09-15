import EventEmitter from "../events/events.mjs";
import { connect as connectSocket } from "../builtins/cloudflare-sockets.mjs";
import { Duplex } from "../streams/node.mjs";
import { Buffer } from "./buffer.mjs";
import { unsupported } from "./unsupported.mjs";

const socketStates = new WeakMap();

function missingArguments() {
  const error = new TypeError("The \"options\" or \"port\" argument must be specified");
  error.code = "ERR_MISSING_ARGS";
  return error;
}

function invalidArgument(message, code = "ERR_INVALID_ARG_VALUE") {
  const error = new TypeError(message);
  error.code = code;
  return error;
}

function normalizeConnection(args) {
  if (args.length === 0) throw missingArguments();
  const first = args[0];
  const options = first !== null && typeof first === "object" ? { ...first } : { port: first, host: args[1] };
  const callback = typeof args.at(-1) === "function" ? args.at(-1) : undefined;
  if (callback && typeof first !== "object") options.host = typeof args[1] === "string" ? args[1] : "localhost";
  if (options.port === undefined || options.port === null || options.port === "") {
    throw invalidArgument("The \"port\" argument must be specified");
  }
  const port = Number(options.port);
  if (!Number.isInteger(port) || port < 0 || port > 65535) {
    throw invalidArgument(`Port should be >= 0 and < 65536. Received ${options.port}.`, "ERR_SOCKET_BAD_PORT");
  }
  const host = String(options.host ?? options.hostname ?? "localhost");
  if (host.length === 0) throw invalidArgument("The \"host\" argument must not be empty");
  return { options: { ...options, port, host }, callback };
}

function bytes(value, encoding) {
  return Buffer.from(value, typeof encoding === "string" ? encoding : undefined);
}

function emitError(socket, error) {
  if (socket.listenerCount("error") > 0) socket.emit("error", error);
  else queueMicrotask(() => socket.emit("error", error));
}

async function pump(socket, state) {
  try {
    await state.native.opened;
    state.reader = state.native.readable.getReader();
    while (!state.destroyed) {
      const { done, value } = await state.reader.read();
      if (done) break;
      state.bytesRead += value.byteLength;
      socket.push(Buffer.from(value));
    }
    socket.push(null);
    finish(socket, state);
  } catch (error) {
    if (!state.destroyed) fail(socket, state, error);
  }
}

function finish(socket, state) {
  if (state.closed) return;
  state.closed = true;
  state.readyState = "closed";
  state.destroyed = true;
  socket.emit("close");
}

function fail(socket, state, error) {
  if (state.closed) return;
  state.error = error;
  destroySocket(socket, state, error);
}

function destroySocket(socket, state, error) {
  if (state.closed) return socket;
  state.destroyed = true;
  state.readyState = "closed";
  state.reader?.cancel(error).catch(() => {});
  state.native?.close().catch(() => {});
  state.closed = true;
  if (error) emitError(socket, error);
  socket.emit("close");
  return socket;
}

function connectSocketToNode(socket, state, options, callback) {
  state.options = options;
  state.connecting = true;
  state.readyState = "opening";
  try {
    state.native = connectSocket(
      { hostname: options.host, port: options.port },
      { secureTransport: options.secureTransport ?? "off" },
    );
  } catch (error) {
    fail(socket, state, error);
    return socket;
  }
  state.native.opened.then(() => {
    if (state.destroyed) return;
    state.connecting = false;
    state.connected = true;
    state.readyState = "open";
    socket.emit(options.secureTransport === "on" ? "secureConnect" : "connect");
    callback?.call(socket);
    // Let connect listeners enqueue their first write before the blocking host
    // read starts. The host socket bridge is synchronous by design.
    queueMicrotask(() => void pump(socket, state));
  }).catch(error => fail(socket, state, error));
  return socket;
}

class SocketBase extends Duplex {
  write(chunk, encoding, callback) {
    if (typeof encoding === "function") [callback, encoding] = [encoding, undefined];
    const state = socketStates.get(this);
    if (state.destroyed) throw new Error("write after end");
    const value = bytes(chunk, encoding);
    state.writeChain = state.writeChain
      .then(async () => {
        await state.native?.opened;
        if (!state.native || state.destroyed) throw new Error("Socket is not connected");
        state.writer ??= state.native.writable.getWriter();
        await state.writer.write(value);
        state.bytesWritten += value.byteLength;
        callback?.();
        this.emit("drain");
      })
      .catch(error => {
        callback?.(error);
        if (!state.destroyed) fail(this, state, error);
      });
    return true;
  }

  destroy(error) { return destroySocket(this, socketStates.get(this), error); }
}

export class Socket extends SocketBase {
  constructor(options = {}) {
    super();
    const state = {
      options: { ...options },
      secureTransport: options.secureTransport ?? "off",
      native: null,
      reader: null,
      writer: null,
      writeChain: Promise.resolve(),
      connecting: false,
      connected: false,
      destroyed: false,
      closed: false,
      readyState: "opening",
      bytesRead: 0,
      bytesWritten: 0,
      error: null,
    };
    socketStates.set(this, state);
    Object.defineProperty(this, "destroyed", {
      configurable: true,
      enumerable: true,
      get: () => state.destroyed,
    });
  }

  get _connecting() { return socketStates.get(this).connecting; }
  get _bytesDispatched() { return socketStates.get(this).bytesWritten; }
  get bufferSize() { return 0; }
  get bytesRead() { return socketStates.get(this).bytesRead; }
  get bytesWritten() { return socketStates.get(this).bytesWritten; }
  get localAddress() { return undefined; }
  get localFamily() { return undefined; }
  get localPort() { return undefined; }
  get pending() { return !socketStates.get(this).connected; }
  get readyState() { return socketStates.get(this).readyState; }
  get remoteAddress() { return socketStates.get(this).options.host; }
  get remoteFamily() { return isIPv6(socketStates.get(this).options.host) ? "IPv6" : "IPv4"; }
  get remotePort() { return socketStates.get(this).options.port; }

  connect(...args) {
    const { options: connection, callback } = normalizeConnection(args);
    const state = socketStates.get(this);
    if (state.native && !state.closed) throw new Error("Socket is already connecting or connected");
    const options = { ...connection, secureTransport: connection.secureTransport ?? state.secureTransport };
    return connectSocketToNode(this, state, options, callback);
  }

  end(chunk, encoding, callback) {
    if (typeof chunk === "function") [callback, chunk] = [chunk, undefined];
    else if (chunk !== undefined) this.write(chunk, encoding);
    const state = socketStates.get(this);
    state.writeChain.then(() => {
      if (state.writer) return state.writer.close();
      return state.native?.close();
    }).then(() => {
      if (!state.destroyed) finish(this, state);
      callback?.();
      this.emit("finish");
    }).catch(error => fail(this, state, error));
    return this;
  }

  destroySoon() { return this.end(); }
  resetAndDestroy() { return this.destroy(); }
  pause() { return this; }
  resume() { return this; }
  read(size) { return super.read(size); }
  setKeepAlive() { return this; }
  setNoDelay() { return this; }
  setTimeout(timeout, callback) { if (callback) this.once("timeout", callback); this.__timeout = timeout; return this; }
  ref() { return this; }
  unref() { return this; }
  address() { return this.localAddress === undefined ? null : { address: this.localAddress, family: this.localFamily, port: this.localPort }; }
  _destroy(error, callback) { callback?.(error); }
  _final(callback) { callback?.(); }
  _getpeername() { return this.remoteAddress === undefined ? null : { address: this.remoteAddress, family: this.remoteFamily, port: this.remotePort }; }
  _getsockname() { return this.address(); }
  _onTimeout() { this.emit("timeout"); }
  _read() {}
  _reset() {}
  _unrefTimer() {}
  _write(chunk, encoding, callback) { this.write(chunk, encoding, callback); }
  _writeGeneric(chunk, encoding, callback) { this._write(chunk, encoding, callback); }
  _writev(chunks, callback) { for (const { chunk, encoding } of chunks) this.write(chunk, encoding); callback?.(); }
}

export class Server extends EventEmitter {
  listen() { return unsupported("net.Server.listen()"); }
  close(callback) { callback?.(); return this; }
}

export class BlockList {
  constructor() { this.__rules = []; }
  addAddress(address, type = "ipv4") { this.__rules.push({ address: String(address), type }); }
  addRange(start, end, type = "ipv4") { this.__rules.push({ start: String(start), end: String(end), type }); }
  addSubnet(network, prefix, type = "ipv4") { this.__rules.push({ network: String(network), prefix: Number(prefix), type }); }
  check(address, type = "ipv4") { return this.__rules.some(rule => rule.address === String(address) && rule.type === type); }
  get rules() { return this.__rules; }
}

export class SocketAddress {
  constructor(options = {}) { Object.assign(this, options); }
}

export function connect(...args) {
  const socket = new Socket();
  const { options, callback } = normalizeConnection(args);
  return connectSocketToNode(socket, socketStates.get(socket), options, callback);
}

export const createConnection = connect;
export function createServer() {
  throw new Error("net.Server is not supported in the Workers runtime");
}
export const _normalizeArgs = (...args) => args;
export function getDefaultAutoSelectFamily() { return true; }
export function getDefaultAutoSelectFamilyAttemptTimeout() { return 250; }
export function setDefaultAutoSelectFamily() {}
export function setDefaultAutoSelectFamilyAttemptTimeout() {}
export function isIP(value) {
  const input = String(value);
  return isIPv6(input) ? 6 : /^(?:\d{1,3}\.){3}\d{1,3}$/.test(input) ? 4 : 0;
}
export const isIPv4 = value => isIP(value) === 4;
export const isIPv6 = value => String(value).includes(":");

export default {
  BlockList, Server, Socket, SocketAddress, Stream: Duplex, _normalizeArgs, connect, createConnection, createServer,
  getDefaultAutoSelectFamily, getDefaultAutoSelectFamilyAttemptTimeout, isIP, isIPv4, isIPv6,
  setDefaultAutoSelectFamily, setDefaultAutoSelectFamilyAttemptTimeout,
};
