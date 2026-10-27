import EventEmitter from "../events/events.mjs";
import { connect as connectSocket } from "../builtins/cloudflare-sockets.mjs";
import { clearTimeout, setTimeout } from "../globals/timers.mjs";
import { Duplex } from "../streams/node.mjs";
import { Buffer } from "./buffer.mjs";
import { unsupported } from "./unsupported.mjs";
import { ipVersion } from "tokamak:host";
import { codedError } from "./errors.mjs";
import { inspect } from "./util.mjs";

const socketStates = new WeakMap();

function invalidArgument(message, code = "ERR_INVALID_ARG_VALUE") {
  return codedError(TypeError, message, code);
}

function normalizeConnection(args) {
  if (args.length === 0) throw invalidArgument("The \"options\" or \"port\" argument must be specified", "ERR_MISSING_ARGS");
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
  state.peer = { address: options.host, port: options.port, family: { 4: "IPv4", 6: "IPv6" }[isIP(options.host)] };
  try {
    state.native = connectSocket(
      { hostname: options.host, port: options.port },
      { secureTransport: options.secureTransport ?? "off", allowHalfOpen: true },
    );
    state.sockname = { address: "0.0.0.0", port: 0, family: "IPv4" };
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
      peer: null,
      sockname: null,
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
  get localAddress() { return this._getsockname().address; }
  get localFamily() { return this._getsockname().family; }
  get localPort() { return this._getsockname().port; }
  get pending() { return !socketStates.get(this).connected; }
  get readyState() {
    if (this.destroyed) return "closed";
    if (this._connecting) return "opening";
    if (this.readable && this.writable) return "open";
    if (this.readable) return "readOnly";
    return this.writable ? "writeOnly" : "closed";
  }
  get remoteAddress() { return this._getpeername().address; }
  get remoteFamily() { return this._getpeername().family; }
  get remotePort() { return this._getpeername().port; }

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
  address() { return this._getsockname(); }
  _destroy(error, callback) {
    const state = socketStates.get(this);
    state.closed = true;
    state.connecting = false;
    clearTimeout(state.timer);
    const finish = closeError => {
      state.peer = null;
      callback(closeError);
    };
    Promise.resolve(state.native?.close()).then(() => finish(error), failure => finish(error ?? failure));
  }
  _final(callback) {
    const state = socketStates.get(this);
    Promise.resolve(state.native?.opened).then(() => {
      if (!state.native) throw new Error("Socket is not connected");
      state.writer ??= state.native.writable.getWriter();
      return state.writer.close();
    }).then(() => callback(), callback);
  }
  _getpeername() { return { ...socketStates.get(this).peer }; }
  _getsockname() { return socketStates.get(this).sockname ?? {}; }
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

function receivedValue(value) {
  if (value === null || value === undefined) return String(value);
  if (typeof value === "function" && value.name) return `function ${value.name}`;
  if (typeof value === "object") return value.constructor?.name ? `an instance of ${value.constructor.name}` : inspect(value, { depth: -1 });
  const inspected = inspect(value);
  return `type ${typeof value} (${inspected.length > 25 ? `${inspected.slice(0, 25)}...` : inspected})`;
}

// "property" for a dotted name such as "options.port", "argument" otherwise.
function nameKind(name) {
  return name.includes(".") ? "property" : "argument";
}

function invalidType(name, expected, value) {
  return invalidArgument(`The "${name}" ${nameKind(name)} must be ${expected}. Received ${receivedValue(value)}`, "ERR_INVALID_ARG_TYPE");
}

function validateType(value, name, type) {
  if (typeof value !== type) throw invalidType(name, `of type ${type}`, value);
}

function invalidValue(name, value, reason) {
  return invalidArgument(`The ${nameKind(name)} '${name}' ${reason}. Received ${inspect(value)}`);
}

function outOfRange(name, range, value) {
  return codedError(RangeError, `The value of "${name}" is out of range. It must be ${range}. Received ${inspect(value)}`, "ERR_OUT_OF_RANGE");
}

function validateFamily(family, name = "family") {
  if (family !== "ipv4" && family !== "ipv6") throw invalidValue(name, family, "must be one of: 'ipv4', 'ipv6'");
}

// The port `port` names, a number or a numeric string from 0 to 65535.
function validatePort(port, name) {
  const numeric = typeof port === "number" || (typeof port === "string" && port.trim() !== "");
  if (!numeric || +port !== +port >>> 0 || +port > 0xffff) {
    throw codedError(Error, `${name} should be >= 0 and < 65536. Received ${typeof port}.`, "ERR_SOCKET_BAD_PORT");
  }
  return +port | 0;
}

function validateUint32(value, name) {
  validateType(value, name, "number");
  if (!Number.isInteger(value)) throw outOfRange(name, "an integer", value);
  if (value < 0 || value > 0xffffffff) throw outOfRange(name, ">= 0 && < 4294967296", value);
}

function invalidAddress(family) {
  return invalidArgument(`Invalid IP address: Invalid ${family === "ipv4" ? "IPv4" : "IPv6"} address`, "ERR_INVALID_IP_ADDRESS");
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
