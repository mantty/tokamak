import EventEmitter from "../events/events.mjs";
import { connect as connectSocket } from "../builtins/cloudflare-sockets.mjs";
import { Duplex } from "../streams/node.mjs";
import { Buffer } from "./buffer.mjs";
import { unsupported } from "./unsupported.mjs";
import { ipVersion } from "tokamak:host";

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

function refreshTimeout(socket, state) {
  clearTimeout(state.timer);
  if (state.timeout > 0 && !socket.destroyed) state.timer = setTimeout(() => socket._onTimeout(), state.timeout);
}

async function pump(socket, state) {
  try {
    await state.native.opened;
    state.reader ??= state.native.readable.getReader();
    while (!socket.destroyed) {
      const { done, value } = await state.reader.read();
      if (done) {
        state.readEnded = true;
        socket.push(null);
        return;
      }
      state.bytesRead += value.byteLength;
      refreshTimeout(socket, state);
      if (!socket.push(Buffer.from(value))) return;
    }
  } catch (error) {
    if (!socket.destroyed) socket.destroy(error);
  } finally {
    state.reading = false;
  }
}

function connectSocketToNode(socket, state, options, callback) {
  state.options = options;
  state.connecting = true;
  try {
    state.native = connectSocket(
      { hostname: options.host, port: options.port },
      { secureTransport: options.secureTransport ?? "off", allowHalfOpen: true },
    );
  } catch (error) {
    socket.destroy(error);
    return socket;
  }
  state.native.opened.then(() => {
    if (socket.destroyed) return;
    state.connecting = false;
    state.connected = true;
    refreshTimeout(socket, state);
    socket.emit("connect");
    if (options.secureTransport === "on") socket.emit("secureConnect");
    callback?.call(socket);
    socket._read();
  }).catch(error => socket.destroy(error));
  state.native.closed.catch(error => socket.destroy(error));
  return socket;
}

export class Socket extends Duplex {
  constructor(options = {}) {
    super({ ...options, allowHalfOpen: options.allowHalfOpen ?? false });
    const state = {
      options: { ...options },
      secureTransport: options.secureTransport ?? "off",
      native: null,
      reader: null,
      reading: false,
      readEnded: false,
      writer: null,
      connecting: false,
      connected: false,
      closed: false,
      bytesRead: 0,
      bytesWritten: 0,
      timer: undefined,
      timeout: 0,
    };
    socketStates.set(this, state);
  }

  get _connecting() { return socketStates.get(this).connecting; }
  get _bytesDispatched() { return socketStates.get(this).bytesWritten; }
  get bufferSize() { return this.writableLength; }
  get bytesRead() { return socketStates.get(this).bytesRead; }
  get bytesWritten() { return socketStates.get(this).bytesWritten; }
  get localAddress() { return undefined; }
  get localFamily() { return undefined; }
  get localPort() { return undefined; }
  get pending() { return !socketStates.get(this).connected; }
  get readyState() {
    if (this.destroyed) return "closed";
    if (this._connecting) return "opening";
    if (this.readable && this.writable) return "open";
    if (this.readable) return "readOnly";
    return this.writable ? "writeOnly" : "closed";
  }
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
    return super.end(chunk, encoding, callback);
  }

  destroySoon() {
    if (this.writableFinished) this.destroy();
    else { this.once("finish", () => this.destroy()); this.end(); }
    return this;
  }
  resetAndDestroy() { return this.destroy(); }
  pause() { return super.pause(); }
  resume() { return super.resume(); }
  read(size) { return super.read(size); }
  setKeepAlive() { return this; }
  setNoDelay() { return this; }
  setTimeout(timeout, callback) {
    if (typeof timeout !== "number" || !Number.isFinite(timeout) || timeout < 0) throw new RangeError("Invalid socket timeout");
    if (callback) this.once("timeout", callback);
    const state = socketStates.get(this);
    state.timeout = timeout;
    refreshTimeout(this, state);
    return this;
  }
  ref() { return this; }
  unref() { return this; }
  address() { return this.localAddress === undefined ? null : { address: this.localAddress, family: this.localFamily, port: this.localPort }; }
  _destroy(error, callback) {
    const state = socketStates.get(this);
    state.closed = true;
    state.connecting = false;
    clearTimeout(state.timer);
    Promise.resolve(state.native?.close()).then(() => callback(error), failure => callback(error ?? failure));
  }
  _final(callback) {
    const state = socketStates.get(this);
    Promise.resolve(state.native?.opened).then(() => {
      if (!state.native) throw new Error("Socket is not connected");
      state.writer ??= state.native.writable.getWriter();
      return state.writer.close();
    }).then(() => callback(), callback);
  }
  _getpeername() { return this.remoteAddress === undefined ? null : { address: this.remoteAddress, family: this.remoteFamily, port: this.remotePort }; }
  _getsockname() { return this.address(); }
  _onTimeout() { this.emit("timeout"); }
  _read() {
    const state = socketStates.get(this);
    if (!state.connected || this.destroyed || state.reading || state.readEnded) return;
    state.reading = true;
    void pump(this, state);
  }
  _reset() {}
  _unrefTimer() {}
  _write(chunk, encoding, callback) {
    const state = socketStates.get(this);
    const value = Buffer.from(chunk, encoding);
    Promise.resolve(state.native?.opened).then(async () => {
      if (!state.native || this.destroyed) throw new Error("Socket is not connected");
      state.writer ??= state.native.writable.getWriter();
      await state.writer.write(value);
      state.bytesWritten += value.byteLength;
      refreshTimeout(this, state);
    }).then(() => callback(), callback);
  }
  _writeGeneric(chunk, encoding, callback) { this._write(chunk, encoding, callback); }
  _writev(chunks, callback) { this._write(Buffer.concat(chunks.map(({ chunk, encoding }) => Buffer.from(chunk, encoding))), undefined, callback); }
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
  const { options, callback } = normalizeConnection(args);
  const socket = new Socket(options);
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
  return ipVersion(`${value}`);
}
export const isIPv4 = value => isIP(value) === 4;
export const isIPv6 = value => isIP(value) === 6;

export default {
  BlockList, Server, Socket, SocketAddress, Stream: Duplex, _normalizeArgs, connect, createConnection, createServer,
  getDefaultAutoSelectFamily, getDefaultAutoSelectFamilyAttemptTimeout, isIP, isIPv4, isIPv6,
  setDefaultAutoSelectFamily, setDefaultAutoSelectFamilyAttemptTimeout,
};
