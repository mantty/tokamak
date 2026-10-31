import { Buffer } from "./buffer.ts";
import { TextDecoder, TextEncoder } from "../streams/text.ts";
import { AbortController } from "../events/abort.ts";
import { objectClass } from "tokamak:host";
import { isDeepStrictEqual } from "./comparisons.ts";
import { codedError } from "./errors.ts";
import { callMethod, isFunction, isObject as isObjectOrFunction, isTypedArray, objectOf, type AnyFunction } from "../globals/objects.ts";
import { string } from "../globals/conversions.ts";

// `inspect`'s options over its defaults, as the caller gave them.
type InspectOptions = Record<PropertyKey, unknown>;

const objectToString = Object.prototype.toString;
const inspectCustom = Symbol.for("nodejs.util.inspect.custom");

function quote(value: unknown): string {
  return `'${String(value).replaceAll("\\", "\\\\").replaceAll("'", "\\'").replaceAll("\n", "\\n").replaceAll("\r", "\\r").replaceAll("\t", "\\t")}'`;
}

function key(value: string): string {
  return /^[A-Za-z_$][\w$]*$/.test(value) ? value : quote(value);
}

function braces(open: string, items: string[], close: string): string {
  return items.length === 0 ? `${open}${close}` : `${open} ${items.join(", ")} ${close}`;
}

// What `value`'s own util.inspect.custom method returns, or `value` itself when none applies.
function customInspection(value: Record<PropertyKey, unknown>, options: InspectOptions, level: number): unknown {
  const method = value[inspectCustom];
  if (!options.customInspect || !isFunction(method) || method === inspect) return value;
  if (isObjectOrFunction(value.constructor) && value.constructor.prototype === value) return value;
  const depth = depthOf(options);
  return method.call(value, depth === null ? null : depth - level, { ...options }, inspect);
}

// How many levels below the top `options` expands, or null for every level.
function depthOf(options: InspectOptions): number | null {
  return options.depth === null ? null : Number(options.depth);
}

// The name of the nearest constructor on `value`'s prototype chain, as Node
// names an object it does not expand.
function constructorName(value: object): string {
  for (let object: object | null = value; object !== null; object = Object.getPrototypeOf(object)) {
    const constructor = Object.getOwnPropertyDescriptor(object, "constructor")?.value;
    if (typeof constructor === "function" && constructor.name !== "") return constructor.name;
  }
  return "Object: null prototype";
}

function inspectValue(value: unknown, options: InspectOptions, level: number, seen: unknown[]): string {
  if (typeof value === "string") return quote(value);
  if (value === undefined) return "undefined";
  if (value === null) return "null";
  if (typeof value === "bigint") return `${value}n`;
  if (!isObjectOrFunction(value)) return String(value);
  const custom = customInspection(value, options, level);
  if (custom !== value) return typeof custom === "string" ? custom : inspectValue(custom, options, level, seen);
  if (isFunction(value)) return `[Function${value.name ? `: ${value.name}` : " (anonymous)"}]`;
  if (seen.includes(value)) return "[Circular]";

  const depth = depthOf(options);
  if (depth !== null && level > depth) return `[${constructorName(value)}]`;

  if (value instanceof Buffer) return value.inspect();
  if (objectClass(value) === "Date") return Number.isNaN(Date.prototype.getTime.call(value)) ? "Invalid Date" : Date.prototype.toISOString.call(value);
  if (objectClass(value) === "RegExp") return RegExp.prototype.toString.call(value);
  if (value instanceof Error) return `${value.name}: ${value.message}`;

  const nextSeen = [...seen, value];
  const inspectItem = (item: unknown) => inspectValue(item, options, level + 1, nextSeen);
  if (Array.isArray(value)) return braces("[", value.map(inspectItem), "]");
  if (value instanceof Map) return `Map(${value.size}) ${braces("{", [...value].map(([entryKey, entryValue]) => `${inspectItem(entryKey)} => ${inspectItem(entryValue)}`), "}")}`;
  if (value instanceof Set) return `Set(${value.size}) ${braces("{", [...value].map(inspectItem), "}")}`;
  if (isTypedArray(value)) return `${value.constructor.name}(${value.length}) ${braces("[", [...value].map(inspectItem), "]")}`;
  return braces("{", Object.keys(value).map(name => `${key(name)}: ${inspectItem(value[name])}`), "}");
}

function stylizeNoColor(text: unknown): unknown { return text; }

export function inspect(value: unknown, options: unknown = {}): string {
  const settings = typeof options === "number" ? { depth: options } : objectOf(options ?? {});
  return inspectValue(value, { ...inspect.defaultOptions, stylize: stylizeNoColor, ...settings }, 0, []);
}

inspect.custom = inspectCustom;
inspect.defaultOptions = { showHidden: false, depth: 2, colors: false, customInspect: true, showProxy: false, maxArrayLength: 100, maxStringLength: 10000, breakLength: 80, compact: 3, sorted: false, getters: false };
inspect.colors = {};
inspect.styles = {};

function json(value: unknown): string {
  try { return JSON.stringify(value); }
  catch { return "[Circular]"; }
}

// `options` are what the caller gave `formatWithOptions`, which `inspect` interprets.
function formatSpecifier(specifier: string, value: unknown, options: {}): string {
  if (specifier === "s") return typeof value === "string" ? value : inspect(value, { ...objectOf(options), depth: 0 });
  if (specifier === "d") return String(Number(value));
  if (specifier === "i") return String(Number.parseInt(string(value), 10));
  if (specifier === "f") return String(Number.parseFloat(string(value)));
  if (specifier === "j") return json(value);
  if (specifier === "o") return inspect(value, { ...objectOf(options), showHidden: true, showProxy: true, depth: 4 });
  if (specifier === "O") return inspect(value, options);
  if (specifier === "c") return "";
  return `%${specifier}`;
}

function formatValues(options: unknown, first: unknown, values: unknown[]): string {
  if (typeof first !== "string") return [first, ...values].map(value => typeof value === "string" ? value : inspect(value, options)).join(" ");
  let index = 0;
  const output = first.replace(/%[sdifjoOc%]/g, token => {
    if (token === "%%") return "%";
    if (index >= values.length) return token;
    return formatSpecifier(token[1], values[index++], options ?? {});
  });
  return index >= values.length ? output : `${output} ${values.slice(index).map(value => typeof value === "string" ? value : inspect(value, options)).join(" ")}`;
}

export function format(first?: unknown, ...values: unknown[]): string { return formatValues(undefined, first, values); }
export function formatWithOptions(options: unknown, first?: unknown, ...values: unknown[]): string { return formatValues(options, first, values); }

export function isArray(value: unknown): boolean { return Array.isArray(value); }
export function isBoolean(value: unknown): boolean { return typeof value === "boolean"; }
export function isBuffer(value: unknown): boolean { return Buffer.isBuffer(value); }
export function isDate(value: unknown): boolean { return value instanceof Date; }
export { isDeepStrictEqual, isFunction };
export function isError(value: unknown): boolean { return value instanceof Error || objectToString.call(value) === "[object Error]"; }
export function isNull(value: unknown): boolean { return value === null; }
export function isNullOrUndefined(value: unknown): boolean { return value == null; }
export function isNumber(value: unknown): boolean { return typeof value === "number"; }
export function isObject(value: unknown): boolean { return value !== null && typeof value === "object"; }
export function isPrimitive(value: unknown): boolean { return value === null || (typeof value !== "object" && typeof value !== "function"); }
export function isRegExp(value: unknown): boolean { return value instanceof RegExp; }
export function isString(value: unknown): boolean { return typeof value === "string"; }
export function isSymbol(value: unknown): boolean { return typeof value === "symbol"; }
export function isUndefined(value: unknown): boolean { return value === undefined; }

export function inherits(ctor: unknown, superCtor: unknown): Function {
  if (typeof ctor !== "function" || typeof superCtor !== "function") throw new TypeError("The super constructor must be a function");
  Object.setPrototypeOf(ctor.prototype, superCtor.prototype);
  Object.setPrototypeOf(ctor, superCtor);
  return ctor;
}

export function promisify(original: unknown): AnyFunction {
  if (!isFunction(original)) throw new TypeError("The last argument must be of type Function");
  const custom = Reflect.get(original, promisify.custom);
  if (isFunction(custom)) return custom;
  return function (this: unknown, ...args: unknown[]) {
    return new Promise((resolve, reject) => {
      original.call(this, ...args, (error: unknown, ...values: unknown[]) => {
        if (error) { reject(error); return; }
        resolve(values[0]);
      });
    });
  };
}
promisify.custom = Symbol.for("nodejs.util.promisify.custom");

export function callbackify(original: unknown): AnyFunction {
  if (!isFunction(original)) throw new TypeError("The first argument must be of type Function");
  return function (this: unknown, ...args: unknown[]) {
    const callback = args.pop();
    if (!isFunction(callback)) throw new TypeError("The last argument must be of type Function");
    Promise.resolve().then(() => original.apply(this, args)).then(value => callback(null, value), error => callback(error || new Error("Promise was rejected with a falsy value")));
  };
}

export function toUSVString(value: unknown): string {
  return String(value).toWellFormed();
}

export function stripVTControlCharacters(value: unknown): string {
  return String(value).replace(/[\u001b\u009b][[\]()#;?]*(?:(?:(?:[a-zA-Z\d]*(?:;[-a-zA-Z\d/#&.:=?%@~_]+)*)?\u0007)|(?:(?:\d{1,4}(?:;\d{0,4})*)?[\dA-PR-TZcf-nq-uy=><~]))/g, "");
}

export function debuglog(): () => void { return () => {}; }
export const debug = debuglog;
export function log(...args: unknown[]): void { console.log(...args); }
export function deprecate(function_: unknown, message: unknown, code: unknown): AnyFunction {
  let warned = false;
  return function (this: unknown, ...args: unknown[]) {
    if (!warned) { warned = true; console.warn(code ? `${message} [${code}]` : message); }
    return callMethod(function_, "apply", this, args);
  };
}

export function _extend(target: unknown, source: unknown): unknown {
  if (target == null) throw new TypeError("Cannot convert undefined or null to object");
  return Object.assign(target, source);
}
export function _errnoException(): never { throw codedError(Error, "The _errnoException method is not implemented", "ERR_METHOD_NOT_IMPLEMENTED"); }
export function _exceptionWithHostPort(): never { throw codedError(Error, "The _exceptionWithHostPort method is not implemented", "ERR_METHOD_NOT_IMPLEMENTED"); }
export function getSystemErrorName(): never { throw new Error("node:util getSystemErrorName is not implemented"); }
export function getSystemErrorMessage(): never { throw new Error("node:util getSystemErrorMessage is not implemented"); }
export function getSystemErrorMap(): never { throw new Error("node:util getSystemErrorMap is not implemented"); }

function typeTag(value: unknown, tag: string): boolean { return objectToString.call(value) === `[object ${tag}]`; }

export const types: Record<string, (value: unknown) => boolean> = {
  isAnyArrayBuffer: value => value instanceof ArrayBuffer || (typeof SharedArrayBuffer !== "undefined" && value instanceof SharedArrayBuffer),
  isArgumentsObject: value => typeTag(value, "Arguments"),
  isArrayBuffer: value => value instanceof ArrayBuffer,
  isArrayBufferView: value => ArrayBuffer.isView(value),
  isAsyncFunction: value => typeTag(value, "AsyncFunction"),
  isBigInt64Array: value => value instanceof BigInt64Array,
  isBigIntObject: value => typeTag(value, "BigInt"),
  isBigUint64Array: value => value instanceof BigUint64Array,
  isBooleanObject: value => typeTag(value, "Boolean"),
  isBoxedPrimitive: value => ["Boolean", "Number", "String", "Symbol", "BigInt"].some(tag => typeTag(value, tag)),
  isCryptoKey: value => typeTag(value, "CryptoKey"),
  isDataView: value => value instanceof DataView,
  isDate: value => value instanceof Date,
  isExternal: () => false,
  isFloat16Array: value => typeof Float16Array !== "undefined" && value instanceof Float16Array,
  isFloat32Array: value => value instanceof Float32Array,
  isFloat64Array: value => value instanceof Float64Array,
  isGeneratorFunction: value => typeTag(value, "GeneratorFunction"),
  isGeneratorObject: value => typeTag(value, "Generator"),
  isInt16Array: value => value instanceof Int16Array,
  isInt32Array: value => value instanceof Int32Array,
  isInt8Array: value => value instanceof Int8Array,
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
  isTypedArray,
  isUint16Array: value => value instanceof Uint16Array,
  isUint32Array: value => value instanceof Uint32Array,
  isUint8Array: value => value instanceof Uint8Array,
  isUint8ClampedArray: value => value instanceof Uint8ClampedArray,
  isWeakMap: value => value instanceof WeakMap,
  isWeakSet: value => value instanceof WeakSet,
};

export class MIMEParams {
  #values = new Map<string, string>();

  constructor(input: unknown = "") {
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
  get(name: unknown): string { return this.#values.get(String(name).toLowerCase()) ?? ""; }
  has(name: unknown): boolean { return this.#values.has(String(name).toLowerCase()); }
  set(name: unknown, value: unknown): this { this.#values.set(String(name).toLowerCase(), String(value)); return this; }
  delete(name: unknown): boolean { return this.#values.delete(String(name).toLowerCase()); }
  entries() { return this.#values.entries(); }
  keys() { return this.#values.keys(); }
  values() { return this.#values.values(); }
  forEach(callback: unknown, thisArg?: unknown): void {
    this.#values.forEach((value, name) => callMethod(callback, "call", thisArg, value, name, this));
  }
  [Symbol.iterator]() { return this.entries(); }
  toString() { return [...this.#values].map(([name, value]) => `${name}=${value}`).join(";"); }
}

export class MIMEType {
  declare type: string;
  declare subtype: string;
  declare params: MIMEParams;

  constructor(value: unknown) {
    const [first, ...parameters] = String(value).split(";");
    const essence = first.trim().toLowerCase();
    const slash = essence.indexOf("/");
    if (slash <= 0 || slash === essence.length - 1) throw new TypeError("The MIME type is invalid");
    this.type = essence.slice(0, slash);
    this.subtype = essence.slice(slash + 1);
    this.params = new MIMEParams(parameters.join(";"));
  }

  get essence() { return `${this.type}/${this.subtype}`; }
  toString() { const params = this.params.toString(); return params ? `${this.essence};${params}` : this.essence; }
  toJSON() { return this.toString(); }
}

export function aborted(signal: unknown): Promise<void> {
  return new Promise(resolve => {
    if (signal == null) return;
    if (objectOf(signal).aborted) { resolve(); return; }
    callMethod(signal, "addEventListener", "abort", resolve, { once: true });
  });
}

export function transferableAbortController(): AbortController { return new AbortController(); }
export function transferableAbortSignal(signal: unknown): unknown { return signal; }
export function styleText(_styles: unknown, value: unknown): string { return String(value); }
export function parseArgs(): never { throw new Error("node:util parseArgs is not implemented"); }

function closingQuote(value: string, quote: string, start: number): number {
  for (let position = start; position < value.length; position += 1) {
    if (value[position] === quote && value[position - 1] !== "\\") return position;
  }
  return -1;
}

const escapes: Record<string, string> = { "\\": "\\", "\"": "\"", n: "\n", r: "\r", t: "\t" };

export function parseEnv(content: unknown): Record<string, string> {
  if (typeof content !== "string") throw new TypeError("The \"content\" argument must be of type string");
  const values: Record<string, string> = Object.create(null);
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
      let closing = closingQuote(value, quote, 1);
      while (closing < 0 && index + 1 < lines.length) {
        value += `\n${lines[++index]}`;
        closing = closingQuote(value, quote, value.length - lines[index].length - 1);
      }
      if (closing >= 0) value = value.slice(1, closing);
      else value = value.slice(1);
      if (quote === "\"") value = value.replace(/\\([\\\"nrt])/g, (_, escaped: string) => escapes[escaped]);
    } else {
      value = value.replace(/\s+#.*$/, "").trim();
    }
    values[name] = value;
  }
  return values;
}

function notImplemented(name: string): never {
  throw codedError(Error, `node:util ${name} is not implemented`, "ERR_METHOD_NOT_IMPLEMENTED");
}
export function getCallSite(): never { notImplemented("getCallSite"); }
export function getCallSites(): never { notImplemented("getCallSites"); }

export default {
  MIMEParams, MIMEType, TextDecoder, TextEncoder, _errnoException, _exceptionWithHostPort, _extend, aborted, callbackify,
  debug, debuglog, deprecate, format, formatWithOptions, getCallSite, getCallSites, getSystemErrorMap, getSystemErrorMessage,
  getSystemErrorName, inherits, inspect, isArray, isBoolean, isBuffer, isDate, isDeepStrictEqual, isError, isFunction, isNull,
  isNullOrUndefined, isNumber, isObject, isPrimitive, isRegExp, isString, isSymbol, isUndefined, log, parseArgs, parseEnv,
  promisify, stripVTControlCharacters, styleText, toUSVString, transferableAbortController, transferableAbortSignal, types,
};
