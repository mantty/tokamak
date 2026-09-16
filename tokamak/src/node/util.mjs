import { Buffer } from "./buffer.mjs";
import { TextDecoder, TextEncoder } from "../streams/text.mjs";

const objectToString = Object.prototype.toString;
const inspectCustom = Symbol.for("nodejs.util.inspect.custom");

function quote(value) {
  return `'${String(value).replaceAll("\\", "\\\\").replaceAll("'", "\\'").replaceAll("\n", "\\n").replaceAll("\r", "\\r").replaceAll("\t", "\\t")}'`;
}

function key(value) {
  return /^[A-Za-z_$][\w$]*$/.test(value) ? value : quote(value);
}

function inspectValue(value, options, level, seen) {
  if (typeof value === "string") return quote(value);
  if (value === undefined) return "undefined";
  if (value === null) return "null";
  if (typeof value === "bigint") return `${value}n`;
  if (typeof value === "symbol") return String(value);
  if (typeof value === "function") return `[Function${value.name ? `: ${value.name}` : ""}]`;
  if (typeof value !== "object") return String(value);
  if (seen.includes(value)) return "[Circular]";

  const depth = options.depth ?? 2;
  if (depth !== null && level > depth) {
    if (Array.isArray(value)) return "[Array]";
    if (value instanceof Map) return "[Map]";
    if (value instanceof Set) return "[Set]";
    return "[Object]";
  }

  if (typeof value[inspectCustom] === "function") {
    const custom = value[inspectCustom](depth === null ? null : depth - level, options, inspect);
    if (typeof custom === "string") return custom;
  }
  if (value instanceof Buffer) return value.inspect();
  if (value instanceof Date) return Number.isNaN(value.getTime()) ? "Invalid Date" : value.toISOString();
  if (value instanceof RegExp) return String(value);
  if (value instanceof Error) return `${value.name}: ${value.message}`;

  const nextSeen = [...seen, value];
  if (Array.isArray(value)) return `[ ${value.map(item => inspectValue(item, options, level + 1, nextSeen)).join(", ")} ]`;
  if (value instanceof Map) {
    const entries = [...value].map(([entryKey, entryValue]) => `${inspectValue(entryKey, options, level + 1, nextSeen)} => ${inspectValue(entryValue, options, level + 1, nextSeen)}`);
    return `Map(${value.size}) { ${entries.join(", ")} }`;
  }
  if (value instanceof Set) return `Set(${value.size}) { ${[...value].map(item => inspectValue(item, options, level + 1, nextSeen)).join(", ")} }`;
  if (ArrayBuffer.isView(value) && !(value instanceof DataView)) {
    return `${value.constructor.name}(${value.length}) [ ${[...value].map(item => inspectValue(item, options, level + 1, nextSeen)).join(", ")} ]`;
  }

  const entries = Object.keys(value).map(name => `${key(name)}: ${inspectValue(value[name], options, level + 1, nextSeen)}`);
  return `{ ${entries.join(", ")} }`;
}

export function inspect(value, options = {}) {
  const settings = typeof options === "number" ? { depth: options } : options ?? {};
  return inspectValue(value, settings, 0, []);
}

inspect.custom = inspectCustom;
inspect.defaultOptions = { showHidden: false, depth: 2, colors: false, customInspect: true, showProxy: false, maxArrayLength: 100, maxStringLength: 10000, breakLength: 80, compact: 3, sorted: false, getters: false };
inspect.colors = {};
inspect.styles = {};

function json(value) {
  try { return JSON.stringify(value); }
  catch { return "[Circular]"; }
}

function formatSpecifier(specifier, value, options) {
  if (specifier === "s") return typeof value === "string" ? value : inspect(value, { ...options, depth: 0 });
  if (specifier === "d") return String(Number(value));
  if (specifier === "i") return String(Number.parseInt(value, 10));
  if (specifier === "f") return String(Number.parseFloat(value));
  if (specifier === "j") return json(value);
  if (specifier === "o") return inspect(value, { ...options, showHidden: true, showProxy: true, depth: 4 });
  if (specifier === "O") return inspect(value, options);
  if (specifier === "c") return "";
  return `%${specifier}`;
}

export function format(first, ...values) {
  if (typeof first !== "string") return [first, ...values].map(value => typeof value === "string" ? value : inspect(value)).join(" ");
  let index = 0;
  const output = first.replace(/%[sdifjoOc%]/g, token => {
    if (token === "%%") return "%";
    if (index >= values.length) return token;
    return formatSpecifier(token[1], values[index++], {});
  });
  return index >= values.length ? output : `${output} ${values.slice(index).map(value => typeof value === "string" ? value : inspect(value)).join(" ")}`;
}

export function formatWithOptions(options, first, ...values) {
  if (typeof first !== "string") return [first, ...values].map(value => typeof value === "string" ? value : inspect(value, options)).join(" ");
  let index = 0;
  const output = first.replace(/%[sdifjoOc%]/g, token => {
    if (token === "%%") return "%";
    if (index >= values.length) return token;
    return formatSpecifier(token[1], values[index++], options ?? {});
  });
  return index >= values.length ? output : `${output} ${values.slice(index).map(value => typeof value === "string" ? value : inspect(value, options)).join(" ")}`;
}

export function isArray(value) { return Array.isArray(value); }
export function isBoolean(value) { return typeof value === "boolean"; }
export function isBuffer(value) { return Buffer.isBuffer(value); }
export function isDate(value) { return value instanceof Date; }
export function isDeepStrictEqual(left, right) { return deepEqual(left, right, []); }
export function isError(value) { return value instanceof Error || objectToString.call(value) === "[object Error]"; }
export function isFunction(value) { return typeof value === "function"; }
export function isNull(value) { return value === null; }
export function isNullOrUndefined(value) { return value == null; }
export function isNumber(value) { return typeof value === "number"; }
export function isObject(value) { return value !== null && typeof value === "object"; }
export function isPrimitive(value) { return value === null || (typeof value !== "object" && typeof value !== "function"); }
export function isRegExp(value) { return value instanceof RegExp; }
export function isString(value) { return typeof value === "string"; }
export function isSymbol(value) { return typeof value === "symbol"; }
export function isUndefined(value) { return value === undefined; }

function deepEqual(left, right, seen) {
  if (Object.is(left, right)) return true;
  if (left === null || right === null || typeof left !== "object" || typeof right !== "object") return false;
  if (left.constructor !== right.constructor) return false;
  if (seen.some(pair => pair[0] === left && pair[1] === right)) return true;
  seen.push([left, right]);
  if (left instanceof Date) return left.getTime() === right.getTime();
  if (left instanceof RegExp) return String(left) === String(right);
  if (left instanceof Map) return left.size === right.size && [...left].every(([key, value]) => [...right].some(([otherKey, otherValue]) => deepEqual(key, otherKey, seen) && deepEqual(value, otherValue, seen)));
  if (left instanceof Set) return left.size === right.size && [...left].every(value => [...right].some(other => deepEqual(value, other, seen)));
  if (ArrayBuffer.isView(left)) {
    if (left.byteLength !== right.byteLength) return false;
    const leftBytes = new Uint8Array(left.buffer, left.byteOffset, left.byteLength);
    const rightBytes = new Uint8Array(right.buffer, right.byteOffset, right.byteLength);
    return leftBytes.every((value, index) => value === rightBytes[index]);
  }
  const leftKeys = Reflect.ownKeys(left).filter(name => Object.prototype.propertyIsEnumerable.call(left, name));
  const rightKeys = Reflect.ownKeys(right).filter(name => Object.prototype.propertyIsEnumerable.call(right, name));
  return leftKeys.length === rightKeys.length && leftKeys.every(name => rightKeys.includes(name) && deepEqual(left[name], right[name], seen));
}

export function inherits(ctor, superCtor) {
  if (typeof ctor !== "function" || typeof superCtor !== "function") throw new TypeError("The super constructor must be a function");
  Object.setPrototypeOf(ctor.prototype, superCtor.prototype);
  Object.setPrototypeOf(ctor, superCtor);
  return ctor;
}

export function promisify(original) {
  if (typeof original !== "function") throw new TypeError("The last argument must be of type Function");
  if (typeof original[promisify.custom] === "function") return original[promisify.custom];
  return function (...args) {
    return new Promise((resolve, reject) => {
      original.call(this, ...args, (error, ...values) => {
        if (error) { reject(error); return; }
        resolve(values[0]);
      });
    });
  };
}
promisify.custom = Symbol.for("nodejs.util.promisify.custom");

export function callbackify(original) {
  if (typeof original !== "function") throw new TypeError("The first argument must be of type Function");
  return function (...args) {
    const callback = args.pop();
    if (typeof callback !== "function") throw new TypeError("The last argument must be of type Function");
    Promise.resolve().then(() => original.apply(this, args)).then(value => callback(null, value), error => callback(error || new Error("Promise was rejected with a falsy value")));
  };
}

export function toUSVString(value) {
  return new TextDecoder().decode(new TextEncoder().encode(String(value)));
}

export function stripVTControlCharacters(value) {
  return String(value).replace(/[\u001b\u009b][[\]()#;?]*(?:(?:(?:[a-zA-Z\d]*(?:;[-a-zA-Z\d/#&.:=?%@~_]+)*)?\u0007)|(?:(?:\d{1,4}(?:;\d{0,4})*)?[\dA-PR-TZcf-nq-uy=><~]))/g, "");
}

export function debuglog() { return () => {}; }
export const debug = debuglog;
export function log(...args) { globalThis.console?.log?.(...args); }
export function deprecate(function_, message, code) {
  let warned = false;
  return function (...args) {
    if (!warned) { warned = true; globalThis.console?.warn?.(code ? `${message} [${code}]` : message); }
    return function_.apply(this, args);
  };
}

export function _extend(target, source) { return Object.assign(target, source); }
export function _errnoException(errorNumber, syscall, message) { const error = new Error(message ?? `${syscall} ${errorNumber}`); error.errno = errorNumber; error.code = getSystemErrorName(errorNumber); error.syscall = syscall; return error; }
export function _exceptionWithHostPort(errorNumber, syscall, address, port) { const error = _errnoException(errorNumber, syscall); error.address = address; error.port = port; return error; }
export function getSystemErrorName(errorNumber) { return errnoNames.get(Number(errorNumber)) ?? `Unknown system error ${errorNumber}`; }
export function getSystemErrorMessage(errorNumber) { return errnoMessages.get(Number(errorNumber)) ?? `Unknown system error ${errorNumber}`; }
export function getSystemErrorMap() { return new Map([...errnoNames].map(([number, name]) => [number, [name, errnoMessages.get(number)]])); }

const errnoNames = new Map([[1, "EPERM"], [2, "ENOENT"], [5, "EIO"], [9, "EBADF"], [11, "EAGAIN"], [12, "ENOMEM"], [13, "EACCES"], [17, "EEXIST"], [20, "ENOTDIR"], [21, "EISDIR"], [22, "EINVAL"], [28, "ENOSPC"], [32, "EPIPE"], [34, "ERANGE"], [38, "ENOSYS"], [110, "ETIMEDOUT"]]);
const errnoMessages = new Map([...errnoNames].map(([number, name]) => [number, name]));

function typeTag(value, tag) { return objectToString.call(value) === `[object ${tag}]`; }
function typedArray(value, ctor) { return value instanceof ctor; }

export const types = {
  isAnyArrayBuffer: value => value instanceof ArrayBuffer || (typeof SharedArrayBuffer !== "undefined" && value instanceof SharedArrayBuffer),
  isArgumentsObject: value => typeTag(value, "Arguments"),
  isArrayBuffer: value => value instanceof ArrayBuffer,
  isArrayBufferView: value => ArrayBuffer.isView(value),
  isAsyncFunction: value => typeTag(value, "AsyncFunction"),
  isBigInt64Array: value => typedArray(value, BigInt64Array),
  isBigIntObject: value => typeTag(value, "BigInt"),
  isBigUint64Array: value => typedArray(value, BigUint64Array),
  isBooleanObject: value => typeTag(value, "Boolean"),
  isBoxedPrimitive: value => ["Boolean", "Number", "String", "Symbol", "BigInt"].some(tag => typeTag(value, tag)),
  isCryptoKey: value => typeTag(value, "CryptoKey"),
  isDataView: value => value instanceof DataView,
  isDate: value => value instanceof Date,
  isExternal: () => false,
  isFloat16Array: value => typeof Float16Array !== "undefined" && value instanceof Float16Array,
  isFloat32Array: value => typedArray(value, Float32Array),
  isFloat64Array: value => typedArray(value, Float64Array),
  isGeneratorFunction: value => typeTag(value, "GeneratorFunction"),
  isGeneratorObject: value => typeTag(value, "Generator"),
  isInt16Array: value => typedArray(value, Int16Array),
  isInt32Array: value => typedArray(value, Int32Array),
  isInt8Array: value => typedArray(value, Int8Array),
  isKeyObject: value => typeTag(value, "KeyObject"),
  isMap: value => value instanceof Map,
  isMapIterator: value => typeTag(value, "Map Iterator"),
  isModuleNamespaceObject: value => typeTag(value, "Module"),
  isNativeError: value => value instanceof Error,
  isNumberObject: value => typeTag(value, "Number"),
  isPromise: value => value instanceof Promise,
  isProxy: () => false,
  isRegExp: value => value instanceof RegExp,
  isSet: value => value instanceof Set,
  isSetIterator: value => typeTag(value, "Set Iterator"),
  isSharedArrayBuffer: value => typeof SharedArrayBuffer !== "undefined" && value instanceof SharedArrayBuffer,
  isStringObject: value => typeTag(value, "String"),
  isSymbolObject: value => typeTag(value, "Symbol"),
  isTypedArray: value => ArrayBuffer.isView(value) && !(value instanceof DataView),
  isUint16Array: value => typedArray(value, Uint16Array),
  isUint32Array: value => typedArray(value, Uint32Array),
  isUint8Array: value => typedArray(value, Uint8Array),
  isUint8ClampedArray: value => typedArray(value, Uint8ClampedArray),
  isWeakMap: value => value instanceof WeakMap,
  isWeakSet: value => value instanceof WeakSet,
};

export class MIMEParams {
  #values = new Map();

  constructor(input = "") {
    for (const item of String(input).split(";")) {
      const index = item.indexOf("=");
      if (index < 0) continue;
      const name = item.slice(0, index).trim().toLowerCase();
      let value = item.slice(index + 1).trim();
      if (value.startsWith('"') && value.endsWith('"')) value = value.slice(1, -1);
      if (name) this.#values.set(name, value);
    }
  }

  get size() { return this.#values.size; }
  get(name) { return this.#values.get(String(name).toLowerCase()) ?? ""; }
  has(name) { return this.#values.has(String(name).toLowerCase()); }
  set(name, value) { this.#values.set(String(name).toLowerCase(), String(value)); return this; }
  delete(name) { return this.#values.delete(String(name).toLowerCase()); }
  entries() { return this.#values.entries(); }
  keys() { return this.#values.keys(); }
  values() { return this.#values.values(); }
  forEach(callback, thisArg) { this.#values.forEach((value, name) => callback.call(thisArg, value, name, this)); }
  [Symbol.iterator]() { return this.entries(); }
  toString() { return [...this.#values].map(([name, value]) => `${name}=${value}`).join(";"); }
}

export class MIMEType {
  constructor(value) {
    const input = String(value);
    const parts = input.split(";");
    const essence = parts.shift().trim().toLowerCase();
    const slash = essence.indexOf("/");
    if (slash <= 0 || slash === essence.length - 1) throw new TypeError("The MIME type is invalid");
    this.type = essence.slice(0, slash);
    this.subtype = essence.slice(slash + 1);
    this.params = new MIMEParams(parts.join(";"));
  }

  get essence() { return `${this.type}/${this.subtype}`; }
  toString() { const params = this.params.toString(); return params ? `${this.essence};${params}` : this.essence; }
  toJSON() { return this.toString(); }
}

export function aborted(signal) {
  return new Promise(resolve => {
    if (signal?.aborted) { resolve(); return; }
    signal?.addEventListener("abort", resolve, { once: true });
  });
}

export function transferableAbortController() { return new AbortController(); }
export function transferableAbortSignal(signal) { return signal; }
export function styleText(_styles, value) { return String(value); }
export function parseArgs() { throw new Error("node:util parseArgs is not implemented"); }
export function parseEnv(content) {
  if (typeof content !== "string") throw new TypeError("The \"content\" argument must be of type string");
  const values = Object.create(null);
  const lines = content.split(/\r?\n/);
  for (let index = 0; index < lines.length; index += 1) {
    let line = lines[index].trim();
    if (line === "" || line.startsWith("#")) continue;
    if (line.startsWith("export ")) line = line.slice(7).trimStart();
    const match = /^([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)$/.exec(line);
    if (!match) continue;
    const [, name, source] = match;
    let value = source;
    if (value.startsWith("\"") || value.startsWith("'")) {
      const quote = value[0];
      let closing = -1;
      for (let position = 1; position < value.length; position += 1) {
        if (value[position] === quote && value[position - 1] !== "\\") { closing = position; break; }
      }
      while (closing < 0 && index + 1 < lines.length) {
        value += `\n${lines[++index]}`;
        for (let position = value.length - lines[index].length - 1; position < value.length; position += 1) {
          if (value[position] === quote && value[position - 1] !== "\\") { closing = position; break; }
        }
      }
      if (closing >= 0) value = value.slice(1, closing);
      else value = value.slice(1);
      if (quote === "\"") value = value.replace(/\\([\\\"nrt])/g, (_, escaped) => ({ "\\": "\\", "\"": "\"", n: "\n", r: "\r", t: "\t" }[escaped]));
    } else {
      value = value.replace(/\s+#.*$/, "").trim();
    }
    values[name] = value;
  }
  return values;
}
export function getCallSite() { const error = new Error("node:util getCallSite is not implemented"); error.code = "ERR_METHOD_NOT_IMPLEMENTED"; throw error; }
export function getCallSites() { const error = new Error("node:util getCallSites is not implemented"); error.code = "ERR_METHOD_NOT_IMPLEMENTED"; throw error; }

const exported = {
  MIMEParams, MIMEType, TextDecoder, TextEncoder, _errnoException, _exceptionWithHostPort, _extend, aborted, callbackify,
  debug, debuglog, deprecate, format, formatWithOptions, getCallSite, getCallSites, getSystemErrorMap, getSystemErrorMessage,
  getSystemErrorName, inherits, inspect, isArray, isBoolean, isBuffer, isDate, isDeepStrictEqual, isError, isFunction, isNull,
  isNullOrUndefined, isNumber, isObject, isPrimitive, isRegExp, isString, isSymbol, isUndefined, log, parseArgs, parseEnv,
  promisify, stripVTControlCharacters, styleText, toUSVString, transferableAbortController, transferableAbortSignal, types,
};

export default exported;
