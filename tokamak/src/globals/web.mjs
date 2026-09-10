import { Blob, Headers, Request, Response } from "../network/fetch.mjs";
import { URL, URLSearchParams } from "../network/url.mjs";
import { ReadableStream, TransformStream, WritableStream } from "../streams/web.mjs";
import { TextDecoder, TextEncoder } from "../streams/text.mjs";
import { decodeBase64, encodeBase64, randomBytes } from "tokamak:host";

export class DOMException extends Error {
  constructor(message = "", name = "Error") { super(binaryString(message)); this.name = binaryString(name); }
  get code() { return Object.hasOwn(exceptionCodes, this.name) ? exceptionCodes[this.name] : 0; }
}

const exceptionCodes = {
  IndexSizeError: 1, HierarchyRequestError: 3, WrongDocumentError: 4,
  InvalidCharacterError: 5, NoModificationAllowedError: 7, NotFoundError: 8,
  NotSupportedError: 9, InUseAttributeError: 10, InvalidStateError: 11,
  SyntaxError: 12, InvalidModificationError: 13, NamespaceError: 14,
  InvalidAccessError: 15, TypeMismatchError: 17, SecurityError: 18,
  NetworkError: 19, AbortError: 20, URLMismatchError: 21,
  QuotaExceededError: 22, TimeoutError: 23, InvalidNodeTypeError: 24, DataCloneError: 25,
};

class DateTimeFormat {
  constructor(locales = [], options = {}) {
    this.locales = locales;
    this.options = options;
  }
  format(value = new Date()) {
    const date = value instanceof Date ? value : new Date(value);
    const options = this.options;
    const pad = (part) => String(part).padStart(2, "0");
    const time = `${pad(date.getUTCHours())}:${pad(date.getUTCMinutes())}:${pad(date.getUTCSeconds())}`;
    if (options.hour || options.minute || options.second) return time;
    return `${pad(date.getUTCDate())}/${pad(date.getUTCMonth() + 1)}/${date.getUTCFullYear()}, ${time}`;
  }
  resolvedOptions() { return { locale: "en-GB", timeZone: "UTC", ...this.options }; }
  formatToParts(value = new Date()) { return [{ type: "literal", value: this.format(value) }]; }
}

class NumberFormat {
  constructor(locales = [], options = {}) { this.locales = locales; this.options = options; }
  format(value) { return String(value); }
  formatToParts(value) { return [{ type: "integer", value: this.format(value) }]; }
  resolvedOptions() { return { locale: "en-GB", ...this.options }; }
}

class PluralRules {
  select(value) { return Number(value) === 1 ? "one" : "other"; }
  resolvedOptions() { return { locale: "en-GB", type: "cardinal" }; }
}

const intl = { DateTimeFormat, NumberFormat, PluralRules, getCanonicalLocales: (locales) => Array.isArray(locales) ? locales : [locales] };

export function installWebGlobals() {
  const globals = { TextEncoder, TextDecoder, Headers, URL, URLSearchParams, Request, Response, ReadableStream, WritableStream, TransformStream, Blob, DOMException };
  Object.assign(globalThis, globals);
  globalThis.Intl ??= intl;
  if (!Date.prototype.toLocaleString) Date.prototype.toLocaleString = function toLocaleString(locales, options) { return new DateTimeFormat(locales, options).format(this); };
  globalThis.WebAssembly ??= {
    compile: async () => ({}),
    instantiate: async () => ({ exports: {} }),
  };
  globalThis.atob = atob;
  globalThis.btoa = btoa;
  globalThis.crypto = { getRandomValues };
  globalThis.queueMicrotask = (callback) => {
    if (typeof callback !== "function") throw new TypeError("Callback must be a function");
    Promise.resolve().then(callback);
  };
  globalThis.scheduler = { wait() { return Promise.reject(new Error("scheduler.wait is not supported by tokamak")); } };
  globalThis.performance ??= { timeOrigin: Date.now(), now: () => Date.now() };
}

function binaryString(value) {
  if (typeof value === "symbol") throw new TypeError("Cannot convert a Symbol to a string");
  return String(value);
}

function atob(value) {
  if (!arguments.length) throw new TypeError("atob requires an argument");
  const input = binaryString(value).replace(/[\t\n\f\r ]/g, "");
  // Forgiving Base64 only accepts padding on a complete quartet.
  const valid = !input.includes("=") || (input.length % 4 === 0 && /^[A-Za-z0-9+/]*={1,2}$/.test(input));
  const bytes = valid ? decodeBase64(input) : null;
  if (!bytes) throw new DOMException("Invalid Base64 data", "InvalidCharacterError");
  let output = "";
  for (const byte of bytes) output += String.fromCharCode(byte);
  return output;
}

function btoa(value) {
  if (!arguments.length) throw new TypeError("btoa requires an argument");
  const input = binaryString(value);
  const bytes = new Uint8Array(input.length);
  for (let index = 0; index < input.length; index++) {
    const code = input.charCodeAt(index);
    if (code > 255) throw new DOMException("Input is not a binary string", "InvalidCharacterError");
    bytes[index] = code;
  }
  return encodeBase64(bytes);
}

function getRandomValues(array) {
  if (!ArrayBuffer.isView(array)) throw new TypeError("Expected an ArrayBuffer view");
  if (array instanceof DataView || array instanceof Float16Array || array instanceof Float32Array || array instanceof Float64Array) {
    throw new DOMException("Expected an integer typed array", "TypeMismatchError");
  }
  if (array.byteLength > 65536) throw new DOMException("Random data exceeds 65536 bytes", "QuotaExceededError");
  new Uint8Array(array.buffer, array.byteOffset, array.byteLength).set(randomBytes(array.byteLength));
  return array;
}
