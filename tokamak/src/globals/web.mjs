import { markHostObject } from "./objects.mjs";
import { decodeBase64, encodeBase64, randomBytes, detachArrayBuffer, digest, cryptoHmac, cryptoAesGcm, cryptoPbkdf2, cryptoHkdf, cryptoGenerateKey, cryptoImportKey, cryptoExportKey, cryptoSign, cryptoVerify, cryptoEncrypt, cryptoDecrypt, cryptoDerive, cryptoTimingSafeEqual, httpFetch } from "tokamak:host";
import { CloseEvent, CustomEvent, ErrorEvent, Event, EventTarget, ExtendableEvent, FetchEvent, MessageChannel, MessageEvent, MessagePort, PromiseRejectionEvent, ScheduledEvent, TailEvent, TraceEvent, WebSocketRequestResponsePair } from "../events/web.mjs";
import { Blob, Body, Cache, CacheStorage, EventSource, File, FormData, Headers, Request, Response } from "../network/fetch.mjs";
import { HTMLRewriter } from "../network/html-rewriter.mjs";
import { URL, URLPattern, URLSearchParams } from "../network/url.mjs";
import { ReadableByteStreamController, ReadableStream, ReadableStreamBYOBReader, ReadableStreamBYOBRequest, ReadableStreamDefaultController, ReadableStreamDefaultReader, WritableStream, WritableStreamDefaultController, WritableStreamDefaultWriter, TransformStream, TransformStreamDefaultController, CompressionStream, DecompressionStream, ByteLengthQueuingStrategy, CountQueuingStrategy, FixedLengthStream, IdentityTransformStream } from "../streams/web.mjs";
import { TextDecoder, TextEncoder, TextDecoderStream, TextEncoderStream } from "../streams/text.mjs";
import { installIntlGlobals } from "../intl.mjs";
import { captureAsyncContext, runInAsyncContext } from "../builtins/async-context.mjs";
import { scheduleTimer } from "tokamak:host";
import { cryptoCreateDigest } from "tokamak:host";
import { structuredClone } from "./structured-clone.mjs";
import { Performance, PerformanceEntry, PerformanceMark, PerformanceMeasure, PerformanceObserver, PerformanceObserverEntryList, PerformanceResourceTiming, performance } from "./performance.mjs";
export { Performance, PerformanceEntry, PerformanceMark, PerformanceMeasure, PerformanceObserver, PerformanceObserverEntryList, PerformanceResourceTiming };
export { structuredClone };

function hidden(object, name, value) {
  Object.defineProperty(object, name, {
    configurable: true,
    enumerable: false,
    writable: true,
    value,
  });
}

export class DOMException extends Error {
  constructor(message = "", name = "Error") {
    super(String(message));
    markHostObject(this, "DOMException");
    this.name = String(name);
    this.code = exceptionCode(this.name);
  }
  get [Symbol.toStringTag]() { return "DOMException"; }
}

for (const [name, code] of Object.entries({
  INDEX_SIZE_ERR: 1, DOMSTRING_SIZE_ERR: 2, HIERARCHY_REQUEST_ERR: 3, WRONG_DOCUMENT_ERR: 4,
  INVALID_CHARACTER_ERR: 5, NO_DATA_ALLOWED_ERR: 6, NO_MODIFICATION_ALLOWED_ERR: 7,
  NOT_FOUND_ERR: 8, NOT_SUPPORTED_ERR: 9, INUSE_ATTRIBUTE_ERR: 10, INVALID_STATE_ERR: 11,
  SYNTAX_ERR: 12, INVALID_MODIFICATION_ERR: 13, NAMESPACE_ERR: 14, INVALID_ACCESS_ERR: 15,
  TYPE_MISMATCH_ERR: 17, SECURITY_ERR: 18, NETWORK_ERR: 19, ABORT_ERR: 20,
  URL_MISMATCH_ERR: 21, QUOTA_EXCEEDED_ERR: 22, TIMEOUT_ERR: 23, INVALID_NODE_TYPE_ERR: 24,
  DATA_CLONE_ERR: 25, VALIDATION_ERR: 0,
})) {
  DOMException[name] = code;
  DOMException.prototype[name] = code;
}

export class AbortSignal extends EventTarget {
  constructor() {
    super();
    hidden(this, "__aborted", false);
    hidden(this, "__reason", undefined);
    hidden(this, "__onabort", null);
  }
  get aborted() { return this.__aborted; }
  get reason() { return this.__reason; }
  get onabort() { return this.__onabort; }
  set onabort(value) { this.__onabort = value; }
  throwIfAborted() { if (this.aborted) throw this.reason; }
  __abort(reason = new DOMException("The operation was aborted", "AbortError")) {
    if (this.aborted) return;
    this.__aborted = true;
    this.__reason = reason;
    const event = new Event("abort");
    this.dispatchEvent(event);
    this.__onabort?.call(this, event);
  }
  static abort(reason = new DOMException("The operation was aborted", "AbortError")) {
    const signal = new AbortSignal();
    signal.__abort(reason);
    return signal;
  }
  static timeout(milliseconds) {
    const signal = new AbortSignal();
    const delayMilliseconds = Math.max(0, Number(milliseconds) || 0);
    setTimeout(() => signal.__abort(new DOMException("The operation timed out.", "TimeoutError")), delayMilliseconds);
    return signal;
  }
  static any(signals) {
    const result = new AbortSignal();
    for (const signal of signals) {
      if (signal.aborted) { result.__abort(signal.reason); break; }
      signal.addEventListener("abort", () => result.__abort(signal.reason), { once: true });
    }
    return result;
  }
}
export class AbortController {
  constructor() { markHostObject(this); hidden(this, "__signal", new AbortSignal()); }
  get signal() { return this.__signal; }
  abort(reason) { this.signal.__abort(reason === undefined ? new DOMException("The operation was aborted", "AbortError") : reason); }
}


function exceptionCode(name) {
  const exceptionCodes = {
    IndexSizeError: 1, DOMStringSizeError: 2, HierarchyRequestError: 3, WrongDocumentError: 4,
    InvalidCharacterError: 5, NoModificationAllowedError: 7, NotFoundError: 8,
    NotSupportedError: 9, InUseAttributeError: 10, InvalidStateError: 11,
    SyntaxError: 12, InvalidModificationError: 13, NamespaceError: 14,
    InvalidAccessError: 15, TypeMismatchError: 17, SecurityError: 18,
    NetworkError: 19, AbortError: 20, URLMismatchError: 21, QuotaExceededError: 22,
    TimeoutError: 23, DataCloneError: 25, NotAllowedError: 0,
  };
  return Object.hasOwn(exceptionCodes, name) ? exceptionCodes[name] : 0;
}

function randomUUID() {
  const bytes = randomBytes(16);
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = [...bytes].map(value => value.toString(16).padStart(2, "0")).join("");
  return hex.slice(0, 8) + "-" + hex.slice(8, 12) + "-" + hex.slice(12, 16) + "-" + hex.slice(16, 20) + "-" + hex.slice(20);
}

function binaryString(value) {
  if (typeof value === "symbol") throw new TypeError("Cannot convert a Symbol to a string");
  return String(value);
}

function atob(value) {
  if (arguments.length === 0) throw new TypeError("1 argument required");
  const input = binaryString(value).replace(/[\t\n\f\r ]/g, "");
  const valid = !input.includes("=") || (input.length % 4 === 0 && /^[A-Za-z0-9+/]*={1,2}$/.test(input));
  const bytes = valid ? decodeBase64(input) : null;
  if (!bytes) throw new DOMException("Invalid Base64 data", "InvalidCharacterError");
  let output = "";
  for (const byte of bytes) output += String.fromCharCode(byte);
  return output;
}

function btoa(value) {
  if (arguments.length === 0) throw new TypeError("1 argument required");
  const input = binaryString(value);
  const bytes = new Uint8Array(input.length);
  for (let index = 0; index < input.length; index += 1) {
    const code = input.charCodeAt(index);
    if (code > 255) throw new DOMException("Input is not a binary string", "InvalidCharacterError");
    bytes[index] = code;
  }
  return encodeBase64(bytes);
}

const keyRecords = new WeakMap();
const cryptoKeyBrand = {};
const digestStreamRecords = new WeakMap();
const hashNames = ["SHA-1", "SHA-256", "SHA-384", "SHA-512"];
const symmetricNames = ["AES-CTR", "AES-CBC", "AES-GCM", "AES-KW"];
const ecCurves = ["P-256", "P-384", "P-521"];

const subtle = {
  async digest(algorithm, data) {
    return digest(normalizeDigest(algorithm), toBytes(data));
  },
  async importKey(format, data, algorithm, extractable, usages) {
    const formatName = normalizeFormat(format);
    const name = algorithmName(algorithm);
    const keyUsages = normalizeUsages(usages);
    if (name === "HMAC") {
      if (formatName === "jwk") return importJwk(data, algorithm, extractable, keyUsages, "HMAC");
      if (formatName !== "raw") throw notSupported();
      const hash = algorithmHash(algorithm);
      const bytes = toBytes(data);
      validateSecretLength(name, bytes);
      validateHmacImportLength(algorithm, bytes);
      validateUsages(keyUsages, ["sign", "verify"]);
      return secretKey({ name, hash: { name: hash }, length: algorithmLength(algorithm) ?? bytes.byteLength * 8 }, extractable, keyUsages, bytes, { kind: "hmac", format: "raw" });
    }
    if (symmetricNames.includes(name)) {
      if (formatName === "jwk") return importJwk(data, algorithm, extractable, keyUsages, name);
      if (formatName !== "raw") throw notSupported();
      const bytes = toBytes(data);
      validateSecretLength(name, bytes);
      validateUsages(keyUsages, name === "AES-KW" ? ["wrapKey", "unwrapKey"] : ["encrypt", "decrypt"]);
      return secretKey({ name, length: bytes.byteLength * 8 }, extractable, keyUsages, bytes, { kind: "aes", format: "raw" });
    }
    if (name === "HKDF" || name === "PBKDF2") {
      if (formatName !== "raw") throw notSupported();
      const bytes = toBytes(data);
      validateUsages(keyUsages, ["deriveBits", "deriveKey"]);
      return secretKey({ name }, false, keyUsages, bytes, { kind: name.toLowerCase(), format: "raw" });
    }
    const asymmetric = normalizeAsymmetricAlgorithm(algorithm);
    if (formatName === "jwk") return importJwk(data, algorithm, extractable, keyUsages, name);
    if (!["raw", "spki", "pkcs8"].includes(formatName)) throw notSupported();
    if (formatName === "raw" && !["ECDSA", "ECDH", "Ed25519", "X25519", "NODE-ED25519"].includes(asymmetric.name)) throw notSupported();
    const type = formatName === "pkcs8" ? "private" : "public";
    const bytes = toBytes(data);
    validateAsymmetricUsages(keyUsages, asymmetric.name, type);
    const bundle = cryptoImportKey(new Uint8Array(), { format: formatName, kind: asymmetric.kind, curve: asymmetric.namedCurve, key: bytes, publicExponent: asymmetric.publicExponent });
    return asymmetricKey(importedAlgorithm(asymmetric, formatName, bytes), extractable, keyUsages, bundle, type === "private");
  },
  async exportKey(format, key) {
    const record = requireKey(key);
    if (!record.extractable) throw new DOMException("Key is not extractable", "InvalidAccessError");
    const formatName = normalizeFormat(format);
    if (record.type === "secret") {
      if (record.algorithm.name === "HKDF" || record.algorithm.name === "PBKDF2") throw notSupported();
      if (formatName === "raw") return record.data.slice().buffer;
      if (formatName === "jwk") return secretJwk(record);
      throw notSupported();
    }
    if (formatName === "raw" && record.type !== "public") throw new DOMException("The key is not public", "InvalidAccessError");
    if (!["raw", "spki", "pkcs8", "jwk"].includes(formatName)) throw notSupported();
    const output = cryptoExportKey(new Uint8Array(), { format: formatName, keyFormat: record.meta.private ? "pkcs8" : "spki", kind: record.meta.kind, key: record.data });
    if (formatName === "jwk") return addJwkMetadata(JSON.parse(new TextDecoder().decode(output)), record);
    return output;
  },
  async generateKey(algorithm, extractable, usages) {
    const name = algorithmName(algorithm);
    const keyUsages = normalizeUsages(usages);
    if (name === "HMAC") {
      const hash = algorithmHash(algorithm);
      const length = algorithmLength(algorithm) ?? hashBlockLength(hash);
      validateHmacLength(length);
      validateUsages(keyUsages, ["sign", "verify"]);
      return secretKey({ name, hash: { name: hash }, length }, extractable, keyUsages, randomBytes(length / 8), { kind: "hmac", format: "raw" });
    }
    if (symmetricNames.includes(name)) {
      const length = requiredAesLength(algorithm);
      validateUsages(keyUsages, name === "AES-KW" ? ["wrapKey", "unwrapKey"] : ["encrypt", "decrypt"]);
      return secretKey({ name, length }, extractable, keyUsages, randomBytes(length / 8), { kind: "aes", format: "raw" });
    }
    if (name === "HKDF" || name === "PBKDF2") throw notSupported();
    const asymmetric = normalizeAsymmetricAlgorithm(algorithm);
    validateAsymmetricUsages(keyUsages, name, "pair");
    const bundle = cryptoGenerateKey({ kind: asymmetric.kind, curve: asymmetric.namedCurve, modulusLength: asymmetric.modulusLength, publicExponent: asymmetric.publicExponent });
    const keys = parseBundle(bundle);
    const publicKey = createKey(asymmetric.keyAlgorithm, extractable, asymmetricPublicUsages(name, keyUsages), "public", keys.public, { kind: asymmetric.kind, format: "spki", private: false });
    const privateKey = createKey(asymmetric.keyAlgorithm, extractable, asymmetricPrivateUsages(name, keyUsages), "private", keys.private, { kind: asymmetric.kind, format: "pkcs8", private: true });
    return { publicKey, privateKey };
  },
  async decrypt(algorithm, key, data) {
    validateKey(key, "decrypt");
    const record = requireAlgorithmKey(key, algorithm);
    if (record.type === "secret" && symmetricNames.includes(record.algorithm.name) && record.algorithm.name !== "AES-KW") {
      const parameter = record.algorithm.name === "AES-CTR" ? algorithmBytes(algorithm, "counter") : algorithmBytes(algorithm, "iv");
      const length = record.algorithm.name === "AES-CTR" ? ctrLength(algorithm) : tagLength(algorithm);
      return cryptoAesGcm(false, record.data, parameter, toBytes(data), aesOptions(length, record.algorithm.name, optionalBytes(algorithm?.additionalData)));
    }
    if (record.algorithm.name !== "RSA-OAEP") throw notSupported();
    return cryptoDecrypt(toBytes(data), { kind: "rsa-oaep", format: record.meta.format, keyFormat: "pkcs8", key: record.data, hash: record.algorithm.hash.name, label: optionalBytes(algorithm?.label) });
  },
  async encrypt(algorithm, key, data) {
    validateKey(key, "encrypt");
    const record = requireAlgorithmKey(key, algorithm);
    if (record.type === "secret" && symmetricNames.includes(record.algorithm.name) && record.algorithm.name !== "AES-KW") {
      const parameter = record.algorithm.name === "AES-CTR" ? algorithmBytes(algorithm, "counter") : algorithmBytes(algorithm, "iv");
      const length = record.algorithm.name === "AES-CTR" ? ctrLength(algorithm) : tagLength(algorithm);
      return cryptoAesGcm(true, record.data, parameter, toBytes(data), aesOptions(length, record.algorithm.name, optionalBytes(algorithm?.additionalData)));
    }
    if (record.algorithm.name !== "RSA-OAEP") throw notSupported();
    return cryptoEncrypt(toBytes(data), { kind: "rsa-oaep", format: record.meta.format, keyFormat: "spki", key: record.data, hash: record.algorithm.hash.name, label: optionalBytes(algorithm?.label) });
  },
  async sign(algorithm, key, data) {
    validateKey(key, "sign");
    const record = requireAlgorithmKey(key, algorithm);
    if (record.algorithm.name === "HMAC") return cryptoHmac(record.algorithm.hash.name, record.data, toBytes(data));
    const hash = record.algorithm.hash?.name ?? (["Ed25519", "NODE-ED25519"].includes(record.algorithm.name) ? "SHA-256" : algorithmHash(algorithm));
    return cryptoSign(toBytes(data), { kind: record.meta.kind, format: record.meta.format, key: record.data, hash, saltLength: record.algorithm.name === "RSA-PSS" ? requiredSaltLength(algorithm) : undefined });
  },
  async verify(algorithm, key, signature, data) {
    validateKey(key, "verify");
    const record = requireAlgorithmKey(key, algorithm);
    const actual = toBytes(signature);
    if (record.algorithm.name === "HMAC") {
      const expected = new Uint8Array(cryptoHmac(record.algorithm.hash.name, record.data, toBytes(data)));
      if (expected.byteLength !== actual.byteLength) return false;
      return timingSafeEqualBytes(expected, actual);
    }
    const hash = record.algorithm.hash?.name ?? (["Ed25519", "NODE-ED25519"].includes(record.algorithm.name) ? "SHA-256" : algorithmHash(algorithm));
    return cryptoVerify(actual, { kind: record.meta.kind, format: record.meta.format, key: record.data, hash, message: toBytes(data), saltLength: record.algorithm.name === "RSA-PSS" ? requiredSaltLength(algorithm) : undefined });
  },
  timingSafeEqual(left, right) { return timingSafeEqualBytes(toBytes(left), toBytes(right)); },
  async deriveBits(algorithm, key, length) {
    return deriveBitsWithUsage(algorithm, key, length, "deriveBits");
  },
  async deriveKey(algorithm, baseKey, derivedKeyAlgorithm, extractable, usages) {
    const length = derivedKeyLength(derivedKeyAlgorithm);
    const bits = new Uint8Array(await deriveBitsWithUsage(algorithm, baseKey, length, "deriveKey"));
    return subtle.importKey("raw", bits, derivedKeyAlgorithm, extractable, usages);
  },
  async wrapKey(format, key, wrappingKey, wrapAlgorithm) {
    const source = requireKey(key);
    if (!source.extractable) throw new DOMException("Key is not extractable", "InvalidAccessError");
    validateKey(wrappingKey, "wrapKey");
    const exported = await subtle.exportKey(format, key);
    const bytes = normalizeFormat(format) === "jwk" ? new TextEncoder().encode(JSON.stringify(exported)) : new Uint8Array(exported);
    return encryptForWrap(wrappingKey, wrapAlgorithm, bytes);
  },
  async unwrapKey(format, wrappedKey, unwrappingKey, unwrapAlgorithm, unwrappedKeyAlgorithm, extractable, usages) {
    validateKey(unwrappingKey, "unwrapKey");
    const bytes = await decryptForWrap(unwrappingKey, unwrapAlgorithm, toBytes(wrappedKey));
    const source = normalizeFormat(format) === "jwk" ? parseWrappedJwk(bytes) : bytes;
    return subtle.importKey(format, source, unwrappedKeyAlgorithm, extractable, usages);
  },
};

export class SubtleCrypto {
  constructor() { markHostObject(this); }
  encrypt(...args) { return subtle.encrypt(...args); }
  decrypt(...args) { return subtle.decrypt(...args); }
  sign(...args) { return subtle.sign(...args); }
  verify(...args) { return subtle.verify(...args); }
  digest(...args) { return subtle.digest(...args); }
  generateKey(...args) { return subtle.generateKey(...args); }
  deriveKey(...args) { return subtle.deriveKey(...args); }
  deriveBits(...args) { return subtle.deriveBits(...args); }
  importKey(...args) { return subtle.importKey(...args); }
  exportKey(...args) { return subtle.exportKey(...args); }
  wrapKey(...args) { return subtle.wrapKey(...args); }
  unwrapKey(...args) { return subtle.unwrapKey(...args); }
  timingSafeEqual(...args) { return subtle.timingSafeEqual(...args); }
}
Object.setPrototypeOf(subtle, SubtleCrypto.prototype);
markHostObject(subtle);

export class CryptoKey {
  constructor(algorithm, extractable, usages, type, data, brand, meta) {
    markHostObject(this);
    if (brand !== cryptoKeyBrand) throw new TypeError("Illegal constructor");
    const record = { algorithm: cloneAlgorithm(algorithm), extractable: Boolean(extractable), usages: [...usages], type, data: data.slice(), meta: { ...meta } };
    keyRecords.set(this, record);
    Object.defineProperties(this, {
      usages: { configurable: true, enumerable: true, get: () => record.usages.slice() },
      algorithm: { configurable: true, enumerable: true, get: () => cloneAlgorithm(record.algorithm) },
      extractable: { configurable: true, enumerable: true, get: () => record.extractable },
      type: { configurable: true, enumerable: true, get: () => record.type },
    });
  }
}
CryptoKey.prototype[Symbol.toStringTag] = "CryptoKey";

function notSupported() { return new DOMException("The requested cryptographic algorithm is not supported.", "NotSupportedError"); }
function invalidAccess() { return new DOMException("The requested key cannot be used with this operation", "InvalidAccessError"); }
function dataError(message) { return new DOMException(message, "DataError"); }
function operationError(message) { return new DOMException(message, "OperationError"); }
function cloneAlgorithm(value, seen = new Map()) {
  if (value === null || typeof value !== "object") return value;
  if (seen.has(value)) return seen.get(value);
  if (value instanceof Uint8Array) return value.slice();
  const copy = Array.isArray(value) ? [] : {};
  seen.set(value, copy);
  for (const key of Object.keys(value)) copy[key] = cloneAlgorithm(value[key], seen);
  return copy;
}
function algorithmName(algorithm) {
  const name = typeof algorithm === "string" ? algorithm : algorithm?.name;
  if (name === undefined || name === null || typeof name === "symbol") throw new TypeError("Algorithm name is required");
  return String(name).toUpperCase();
}
function normalizeDigest(algorithm) {
  const name = algorithmName(algorithm);
  if (![...hashNames, "MD5"].includes(name)) throw notSupported();
  return name;
}
function normalizeFormat(format) {
  if (typeof format !== "string") throw new TypeError("Key format must be a string");
  return format.toLowerCase();
}
function algorithmHash(algorithm) {
  const value = typeof algorithm === "object" && algorithm !== null ? algorithm.hash : undefined;
  const name = algorithmName(value);
  if (!hashNames.includes(name)) throw notSupported();
  return name;
}
function algorithmBytes(algorithm, name) {
  if (algorithm === null || typeof algorithm !== "object" || algorithm[name] === undefined) throw operationError(`AES algorithm ${name} is required`);
  return toBytes(algorithm[name]);
}
function requiredBytes(algorithm, name) {
  if (algorithm === null || typeof algorithm !== "object" || algorithm[name] === undefined) throw operationError(`${name} is required`);
  return toBytes(algorithm[name]);
}
function optionalBytes(value) { return value === undefined || value === null ? new Uint8Array() : toBytes(value); }
function deriveBitsWithUsage(algorithm, key, length, usage) {
  const name = algorithmName(algorithm);
  validateKey(key, usage);
  const record = requireAlgorithmKey(key, algorithm);
  const bitLength = deriveLength(length);
  if (name === "HKDF") {
    if (record.algorithm.name !== "HKDF") throw invalidAccess();
    return hkdf(algorithmHash(algorithm), record.data, requiredBytes(algorithm, "salt"), optionalBytes(algorithm?.info), bitLength);
  }
  if (name === "PBKDF2") {
    if (record.algorithm.name !== "PBKDF2") throw invalidAccess();
    return pbkdf2(algorithmHash(algorithm), record.data, requiredBytes(algorithm, "salt"), positiveInteger(algorithm?.iterations, "PBKDF2 iterations"), bitLength);
  }
  if (name !== "ECDH" && name !== "X25519") throw notSupported();
  if (record.type !== "private" || record.algorithm.name !== name) throw invalidAccess();
  const peer = algorithm?.public;
  if (!(peer instanceof CryptoKey) || peer.type !== "public" || peer.algorithm.name !== name) throw invalidAccess();
  if (name === "ECDH" && record.algorithm.namedCurve !== peer.algorithm.namedCurve) throw invalidAccess();
  const output = cryptoDerive({ kind: name === "X25519" ? "x25519" : "ec", format: "pkcs8", keyFormat: "pkcs8", key: record.data, peer: peerRecord(peer).data });
  if (bitLength > output.byteLength * 8) throw operationError("Derived secret is shorter than requested");
  return output.slice(0, bitLength / 8);
}
function tagLength(algorithm) {
  const value = algorithm?.tagLength === undefined ? 128 : Number(algorithm.tagLength);
  if (![32, 64, 96, 104, 112, 120, 128].includes(value)) throw operationError("Invalid AES-GCM tag length");
  return value;
}
function ctrLength(algorithm) {
  const value = Number(algorithm?.length);
  if (!Number.isInteger(value) || value < 1 || value > 128) throw operationError("Invalid AES-CTR length");
  return value;
}
function normalizeUsages(usages) {
  if (usages === undefined || usages === null) throw new TypeError("Key usages are required");
  let values;
  try { values = [...usages]; } catch { throw new DOMException("Invalid key usage.", "SyntaxError"); }
  if (values.some(value => typeof value !== "string") || new Set(values).size !== values.length) throw new DOMException("Invalid key usage.", "SyntaxError");
  return values;
}
function validateUsages(usages, allowed) {
  if (usages.some(usage => !allowed.includes(usage))) throw new DOMException("Invalid key usage.", "SyntaxError");
}
function validateAsymmetricUsages(usages, name, type) {
  const publicAllowed = name === "RSA-OAEP" ? ["encrypt"] : name === "ECDH" || name === "X25519" ? [] : ["verify"];
  const privateAllowed = name === "RSA-OAEP" ? ["decrypt"] : name === "ECDH" || name === "X25519" ? ["deriveBits", "deriveKey"] : ["sign"];
  if (type === "public") validateUsages(usages, publicAllowed);
  else if (type === "private") validateUsages(usages, privateAllowed);
  else validateUsages(usages, [...new Set([...publicAllowed, ...privateAllowed])]);
}
function validateKey(key, usage) { if (!requireKey(key).usages.includes(usage)) throw invalidAccess(); }
function requireKey(key) {
  const record = key instanceof CryptoKey ? keyRecords.get(key) : undefined;
  if (!record) throw invalidAccess();
  return record;
}
function peerRecord(key) { return requireKey(key); }
function requireAlgorithmKey(key, algorithm) {
  const record = requireKey(key);
  if (algorithmName(record.algorithm) !== algorithmName(algorithm)) throw notSupported();
  return record;
}
function toBytes(value) {
  try {
    if (value instanceof ArrayBuffer) return new Uint8Array(value).slice();
    if (ArrayBuffer.isView(value)) return new Uint8Array(value.buffer, value.byteOffset, value.byteLength).slice();
  } catch { throw new TypeError("ArrayBuffer is detached"); }
  throw new TypeError("Data must be an ArrayBuffer or ArrayBufferView");
}
function getRandomValues(value) {
  if (!ArrayBuffer.isView(value)) throw new TypeError("Expected an integer typed array");
  if (value instanceof DataView || value instanceof Float32Array || value instanceof Float64Array || (typeof Float16Array !== "undefined" && value instanceof Float16Array)) throw new DOMException("The provided value is not an integer typed array.", "TypeMismatchError");
  if (value.byteLength > 65536) throw new DOMException("The requested length exceeds 65,536 bytes.", "QuotaExceededError");
  new Uint8Array(value.buffer, value.byteOffset, value.byteLength).set(randomBytes(value.byteLength));
  return value;
}

function aesOptions(tagLength, mode, additionalData) { return { tagLength, mode, additionalData }; }
function createKey(algorithm, extractable, usages, type, data, meta) { return new CryptoKey(algorithm, extractable, usages, type, data, cryptoKeyBrand, meta); }
function secretKey(algorithm, extractable, usages, data, meta) { return createKey(algorithm, extractable, usages, "secret", data, meta); }
function parseBundle(value) {
  const bytes = toBytes(value);
  if (bytes.byteLength < 9 || bytes[0] !== 1) throw dataError("Invalid native key bundle");
  const publicLength = new DataView(bytes.buffer, bytes.byteOffset + 1, 4).getUint32(0);
  const privateLength = new DataView(bytes.buffer, bytes.byteOffset + 5, 4).getUint32(0);
  if (9 + publicLength + privateLength !== bytes.byteLength) throw dataError("Invalid native key bundle");
  return { public: bytes.slice(9, 9 + publicLength), private: privateLength ? bytes.slice(9 + publicLength) : null };
}
function asymmetricKey(algorithm, extractable, usages, bundle, privateOnly) {
  const keys = parseBundle(bundle);
  if (privateOnly) return createKey(algorithm.keyAlgorithm, extractable, usages, "private", keys.private, { kind: algorithm.kind, format: "pkcs8", private: true });
  return createKey(algorithm.keyAlgorithm, extractable, usages, "public", keys.public, { kind: algorithm.kind, format: "spki", private: false });
}
function normalizeAsymmetricAlgorithm(algorithm) {
  const name = algorithmName(algorithm);
  if (["RSASSA-PKCS1-V1_5", "RSA-PSS", "RSA-OAEP"].includes(name)) {
    if (typeof algorithm !== "object" || algorithm === null) throw new TypeError("An algorithm object is required");
    const modulusLength = algorithm.modulusLength === undefined ? undefined : integer(algorithm.modulusLength, "modulusLength");
    const publicExponent = algorithm.publicExponent === undefined ? undefined : toBytes(algorithm.publicExponent);
    const hash = algorithmHash(algorithm);
    return { name, kind: name === "RSA-PSS" ? "rsa-pss" : name === "RSA-OAEP" ? "rsa-oaep" : "rsa", modulusLength, publicExponent, keyAlgorithm: { name: canonicalAlgorithmName(name), modulusLength, publicExponent: publicExponent?.slice(), hash: { name: hash } } };
  }
  if (name === "ECDSA" || name === "ECDH") {
    const namedCurve = String(algorithm?.namedCurve ?? "").toUpperCase();
    if (!ecCurves.includes(namedCurve)) throw notSupported();
    return { name, kind: "ec", namedCurve, keyAlgorithm: { name, namedCurve } };
  }
  if (name === "ED25519") return { name: "Ed25519", kind: "ed25519", keyAlgorithm: { name: "Ed25519" } };
  if (name === "X25519") return { name, kind: "x25519", keyAlgorithm: { name } };
  if (name === "NODE-ED25519") {
    if (String(algorithm?.namedCurve ?? "").toUpperCase() !== "NODE-ED25519") throw new TypeError("namedCurve is required");
    return { name, kind: "ed25519", namedCurve: "NODE-ED25519", keyAlgorithm: { name, namedCurve: "NODE-ED25519" } };
  }
  throw notSupported();
}
function asymmetricPublicUsages(name, usages) { return name === "ECDH" || name === "X25519" ? [] : usages.filter(value => value === "verify" || value === "encrypt"); }
function asymmetricPrivateUsages(name, usages) { return name === "RSA-OAEP" ? usages.filter(value => value === "decrypt") : name === "ECDH" || name === "X25519" ? usages.filter(value => value === "deriveBits" || value === "deriveKey") : usages.filter(value => value === "sign"); }
function integer(value, name) { const number = Number(value); if (!Number.isSafeInteger(number) || number < 0) throw new TypeError(`${name} must be an integer`); return number; }
function positiveInteger(value, name) { const number = integer(value, name); if (number < 1) throw operationError(`${name} must be positive`); return number; }
function requiredAesLength(algorithm) { const length = integer(algorithm?.length, "length"); if (![128, 192, 256].includes(length)) throw dataError("Invalid AES key length"); return length; }
function validateSecretLength(name, data) {
  if (name === "HMAC" && data.byteLength === 0) throw dataError("Invalid HMAC key length");
  if (name !== "HMAC" && name !== "HKDF" && name !== "PBKDF2" && ![16, 24, 32].includes(data.byteLength)) throw dataError("Invalid AES key length");
}
function algorithmLength(algorithm) { if (algorithm?.length === undefined) return undefined; const length = integer(algorithm.length, "length"); if (length < 1 || length % 8 !== 0) throw dataError("Invalid HMAC key length"); return length; }
function validateHmacImportLength(algorithm, data) {
  const length = algorithmLength(algorithm);
  if (length !== undefined && length !== data.byteLength * 8) throw dataError("Invalid HMAC key length");
}
function validateHmacLength(length) { if (!Number.isSafeInteger(length) || length < 1 || length % 8 !== 0) throw dataError("Invalid HMAC key length"); }
function hashBlockLength(hash) { return hash === "SHA-384" || hash === "SHA-512" ? 1024 : 512; }
function hashByteLength(hash) { return { "SHA-1": 20, "SHA-256": 32, "SHA-384": 48, "SHA-512": 64 }[hash]; }
function requiredSaltLength(algorithm) {
  if (algorithm === null || typeof algorithm !== "object") throw new TypeError("RSA-PSS saltLength is required");
  const length = integer(algorithm.saltLength, "saltLength");
  if (length > 0x7fffffff) throw new TypeError("RSA-PSS saltLength is too large");
  return length;
}

function importJwk(data, algorithm, extractable, usages, expectedName) {
  if (data === null || typeof data !== "object" || data instanceof ArrayBuffer || ArrayBuffer.isView(data) || Array.isArray(data)) {
    throw new TypeError("JsonWebKey must be an object");
  }
  const jwk = data;
  if (jwk === null || typeof jwk !== "object" || Array.isArray(jwk)) throw dataError("Invalid JWK");
  if (jwk.ext === false && extractable) throw dataError("JWK is not extractable");
  if (Array.isArray(jwk.key_ops) && jwk.key_ops.some(value => !usages.includes(value))) throw dataError("JWK key operations do not match usages");
  if (expectedName === "HMAC" || symmetricNames.includes(expectedName)) {
    if (jwk.kty !== "oct" || typeof jwk.k !== "string") throw dataError("JWK key type does not match the algorithm");
    const bytes = base64urlDecode(jwk.k);
    validateSecretLength(expectedName, bytes);
    const hash = expectedName === "HMAC" ? algorithmHash(algorithm) : undefined;
    if (expectedName === "HMAC") validateHmacImportLength(algorithm, bytes);
    validateUsages(usages, expectedName === "HMAC" ? ["sign", "verify"] : expectedName === "AES-KW" ? ["wrapKey", "unwrapKey"] : ["encrypt", "decrypt"]);
    return secretKey(expectedName === "HMAC" ? { name: expectedName, hash: { name: hash }, length: algorithmLength(algorithm) ?? bytes.byteLength * 8 } : { name: expectedName, length: bytes.byteLength * 8 }, extractable, usages, bytes, { kind: expectedName === "HMAC" ? "hmac" : "aes", format: "jwk" });
  }
  const asymmetric = normalizeAsymmetricAlgorithm(algorithm);
  const expectedType = asymmetric.name === "RSASSA-PKCS1-V1_5" || asymmetric.name === "RSA-PSS" || asymmetric.name === "RSA-OAEP" ? "RSA" : asymmetric.kind === "ec" ? "EC" : "OKP";
  if (jwk.kty !== expectedType) throw dataError("JWK key type does not match the algorithm");
  if (asymmetric.kind === "ec" && jwk.crv !== asymmetric.namedCurve) throw dataError("JWK curve does not match the algorithm");
  if (asymmetric.kind === "ed25519" && jwk.crv !== "Ed25519") throw dataError("JWK curve does not match the algorithm");
  if (asymmetric.kind === "x25519" && jwk.crv !== "X25519") throw dataError("JWK curve does not match the algorithm");
  const privateKey = jwk.d !== undefined;
  validateAsymmetricUsages(usages, asymmetric.name, privateKey ? "private" : "public");
  const bundle = cryptoImportKey(new Uint8Array(), { format: "jwk", kind: asymmetric.kind, curve: asymmetric.namedCurve, jwk: JSON.stringify(jwk) });
  return asymmetricKey(importedAlgorithm(asymmetric, "jwk", jwk), extractable, usages, bundle, privateKey);
}

function secretJwk(record) {
  const value = { kty: "oct", k: base64urlEncode(record.data), key_ops: record.usages.slice(), ext: record.extractable };
  if (record.algorithm.name === "HMAC") value.alg = ({ "SHA-1": "HS1", "SHA-256": "HS256", "SHA-384": "HS384", "SHA-512": "HS512" })[record.algorithm.hash.name];
  else value.alg = `A${record.algorithm.length}${({ "AES-GCM": "GCM", "AES-CBC": "CBC", "AES-CTR": "CTR", "AES-KW": "KW" })[record.algorithm.name]}`;
  return value;
}
function addJwkMetadata(value, record) {
  value.key_ops = record.usages.slice();
  value.ext = record.extractable;
  const name = algorithmName(record.algorithm);
  const hash = record.algorithm.hash?.name;
  if (name === "RSASSA-PKCS1-V1_5") value.alg = ({ "SHA-1": "RS1", "SHA-256": "RS256", "SHA-384": "RS384", "SHA-512": "RS512" })[hash];
  else if (name === "RSA-PSS") value.alg = ({ "SHA-1": "PS1", "SHA-256": "PS256", "SHA-384": "PS384", "SHA-512": "PS512" })[hash];
  else if (name === "RSA-OAEP") value.alg = hash === "SHA-1" ? "RSA-OAEP" : `RSA-OAEP-${hash.slice(4)}`;
  else if (name === "Ed25519" || name === "NODE-ED25519") value.alg = "EdDSA";
  return value;
}

function canonicalAlgorithmName(name) {
  return name === "RSASSA-PKCS1-V1_5" ? "RSASSA-PKCS1-v1_5" : name === "ED25519" ? "Ed25519" : name;
}
function importedAlgorithm(asymmetric, format, data) {
  if (!["RSASSA-PKCS1-V1_5", "RSA-PSS", "RSA-OAEP"].includes(asymmetric.name)) return asymmetric;
  const jwk = format === "jwk"
    ? data
    : JSON.parse(new TextDecoder().decode(cryptoExportKey(new Uint8Array(), { format: "jwk", keyFormat: format, kind: asymmetric.kind, key: data })));
  const modulus = typeof jwk?.n === "string" ? base64urlDecode(jwk.n) : null;
  const publicExponent = typeof jwk?.e === "string" ? base64urlDecode(jwk.e) : null;
  if (!modulus?.byteLength || !publicExponent?.byteLength) throw dataError("Invalid RSA JWK");
  let first = modulus[0];
  let modulusLength = (modulus.byteLength - 1) * 8;
  while (first > 0) { modulusLength += 1; first >>>= 1; }
  return { ...asymmetric, keyAlgorithm: { ...asymmetric.keyAlgorithm, modulusLength, publicExponent } };
}
function base64urlEncode(bytes) { return encodeBase64(bytes).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, ""); }
function base64urlDecode(value) {
  if (!/^[A-Za-z0-9_-]*$/.test(value)) throw dataError("Invalid base64url value");
  const padded = value.replaceAll("-", "+").replaceAll("_", "/") + "===".slice((value.length + 3) % 4);
  try { return new Uint8Array([...atob(padded)].map(character => character.charCodeAt(0))); }
  catch { throw dataError("Invalid base64url value"); }
}
function parseWrappedJwk(bytes) {
  try { return JSON.parse(new TextDecoder().decode(bytes)); }
  catch { throw dataError("Invalid JWK"); }
}

function hkdf(hash, key, salt, info, length) {
  if (length / 8 > hashByteLength(hash) * 255) throw operationError("HKDF output is too long");
  return cryptoHkdf(hash, key, salt, info, length / 8);
}
function pbkdf2(hash, password, salt, iterations, length) {
  return cryptoPbkdf2(hash, password, salt, iterations, length / 8);
}
function deriveLength(length) {
  const value = integer(length, "length");
  if (value === 0 || value % 8 !== 0) throw operationError("Derived length must be a non-zero multiple of 8");
  return value;
}
function derivedKeyLength(algorithm) {
  const name = algorithmName(algorithm);
  if (symmetricNames.includes(name)) return requiredAesLength(algorithm);
  if (name === "HMAC") {
    const hash = algorithmHash(algorithm);
    return algorithmLength(algorithm) ?? hashBlockLength(hash);
  }
  throw notSupported();
}
function encryptForWrap(key, algorithm, bytes) {
  const record = requireAlgorithmKey(key, algorithm);
  if (record.algorithm.name === "AES-KW") return cryptoAesGcm(true, record.data, new Uint8Array(), bytes, aesOptions(128, "AES-KW"));
  if (record.algorithm.name === "AES-GCM" || record.algorithm.name === "AES-CBC" || record.algorithm.name === "AES-CTR") {
    const parameter = record.algorithm.name === "AES-CTR" ? algorithmBytes(algorithm, "counter") : algorithmBytes(algorithm, "iv");
    const length = record.algorithm.name === "AES-CTR" ? ctrLength(algorithm) : tagLength(algorithm);
    return cryptoAesGcm(true, record.data, parameter, bytes, aesOptions(length, record.algorithm.name, optionalBytes(algorithm?.additionalData)));
  }
  if (record.algorithm.name === "RSA-OAEP") return cryptoEncrypt(bytes, { kind: "rsa-oaep", format: record.meta.format, keyFormat: record.meta.private ? "pkcs8" : "spki", key: record.data, hash: record.algorithm.hash.name, label: optionalBytes(algorithm?.label) });
  throw notSupported();
}
function decryptForWrap(key, algorithm, bytes) {
  const record = requireAlgorithmKey(key, algorithm);
  if (record.algorithm.name === "AES-KW") return cryptoAesGcm(false, record.data, new Uint8Array(), bytes, aesOptions(128, "AES-KW"));
  if (record.algorithm.name === "AES-GCM" || record.algorithm.name === "AES-CBC" || record.algorithm.name === "AES-CTR") {
    const parameter = record.algorithm.name === "AES-CTR" ? algorithmBytes(algorithm, "counter") : algorithmBytes(algorithm, "iv");
    const length = record.algorithm.name === "AES-CTR" ? ctrLength(algorithm) : tagLength(algorithm);
    return cryptoAesGcm(false, record.data, parameter, bytes, aesOptions(length, record.algorithm.name, optionalBytes(algorithm?.additionalData)));
  }
  if (record.algorithm.name === "RSA-OAEP") return cryptoDecrypt(bytes, { kind: "rsa-oaep", format: record.meta.format, keyFormat: "pkcs8", key: record.data, hash: record.algorithm.hash.name, label: optionalBytes(algorithm?.label) });
  throw notSupported();
}
function timingSafeEqualBytes(left, right) {
  return cryptoTimingSafeEqual(left, right);
}

export class DigestStream extends WritableStream {
  constructor(algorithm) {
    const name = normalizeDigest(algorithm);
    let resolveDigest;
    let rejectDigest;
    const digestPromise = new Promise((resolve, reject) => { resolveDigest = resolve; rejectDigest = reject; });
    let state = cryptoCreateDigest(name);
    let bytesWritten = 0n;
    super({
      write(chunk) {
        try {
          const bytes = toBytes(chunk);
          state.update(bytes);
          bytesWritten += BigInt(bytes.byteLength);
        } catch (error) {
          state = null;
          rejectDigest(error);
          throw error;
        }
      },
      close() {
        try { resolveDigest(state.finish()); }
        catch (error) { rejectDigest(error); throw error; }
        finally { state = null; }
      },
      abort(reason) { state = null; rejectDigest(reason); },
    });
    digestStreamRecords.set(this, { digest: digestPromise, get bytesWritten() { return bytesWritten; } });
  }
  get digest() { return digestStreamRecords.get(this).digest; }
  get bytesWritten() { return digestStreamRecords.get(this).bytesWritten; }
}
Object.defineProperty(DigestStream.prototype, Symbol.toStringTag, { value: "DigestStream", configurable: true });

export class Crypto {
  constructor() { markHostObject(this); }
  get subtle() { return subtle; }
  get DigestStream() { return DigestStream; }
  getRandomValues(value) { return getRandomValues(value); }
  randomUUID() { return randomUUID(); }
}

const crypto = new Crypto();

const timers = new Map();
let nextTimer = 1;

export class Navigator {
  constructor() {
    markHostObject(this);
    this.userAgent = "Cloudflare-Workers";
    this.platform = "";
    this.language = "en";
    this.languages = Object.freeze(["en"]);
    this.hardwareConcurrency = 1;
  }
  sendBeacon() { return false; }
}


function timer(callback, timeout, repeat, args) {
  if (typeof callback !== "function") throw new TypeError("Timer callback must be a function");
  const id = nextTimer++;
  const milliseconds = Math.max(0, Math.min(2 ** 32 - 1, Number(timeout) || 0));
  const context = captureAsyncContext();
  const fire = () => {
    if (repeat) timers.set(id, scheduleTimer(fire, Math.max(1, milliseconds)));
    else timers.delete(id);
    runInAsyncContext(context, callback, undefined, args);
  };
  timers.set(id, scheduleTimer(fire, milliseconds));
  return id;
}

function clearTimer(id) {
  timers.get(id)?.();
  timers.delete(id);
}

function setTimeout(callback, timeout, ...args) { return timer(callback, timeout, false, args); }
function setInterval(callback, timeout, ...args) { return timer(callback, timeout, true, args); }
function setImmediate(callback, ...args) { return timer(callback, 0, false, args); }
function clearTimeout(id) { clearTimer(id); }
function clearInterval(id) { clearTimer(id); }
function clearImmediate(id) { clearTimer(id); }

export class WorkerGlobalScope extends EventTarget {}
export class ServiceWorkerGlobalScope extends WorkerGlobalScope {}
ServiceWorkerGlobalScope.prototype[Symbol.toStringTag] = "ServiceWorkerGlobalScope";
Object.defineProperty(WorkerGlobalScope.prototype, "EventTarget", {
  configurable: true,
  enumerable: true,
  value: EventTarget,
});

function reportError(error) {
  const event = new ErrorEvent("error", { error, message: error?.message ?? String(error) });
  if (!globalThis.dispatchEvent?.(event)) console.error(error);
}

async function fetch(input, init) {
  const request = new Request(input, init);
  if (request.signal.aborted) throw request.signal.reason;
  const task = httpFetch(
    request.url,
    request.method,
    JSON.stringify([...request.headers]),
    request.__body ?? request.__stream,
    request.redirect,
  );
  globalThis.__tokamak_context.waitUntil(task.upload);
  if (request.body !== null) request.__bodyConsumed = true;
  const abort = () => task.cancel();
  request.signal.addEventListener("abort", abort, { once: true });
  const finish = () => request.signal.removeEventListener("abort", abort);
  let response;
  try {
    response = await task.response;
    if (response.bodyless) await task.upload;
  } catch (error) {
    finish();
    if (request.signal.aborted) throw fetchAbortReason(request.signal);
    throw error;
  }
  if (request.signal.aborted) { task.cancel(); finish(); throw fetchAbortReason(request.signal); }
  const body = response.bodyless ? null : new ReadableStream({
    type: "bytes",
    async pull(controller) {
      try {
        request.signal.throwIfAborted();
        const chunk = await response.read();
        request.signal.throwIfAborted();
        if (chunk == null) { finish(); controller.close(); }
        else controller.enqueue(chunk);
      } catch (error) {
        task.cancel(); finish();
        controller.error(request.signal.aborted ? fetchAbortReason(request.signal) : error);
      }
    },
    cancel() { task.cancel(); finish(); },
  });
  if (response.bodyless) finish();
  return new Response(body, {
    status: response.status,
    statusText: response.statusText,
    headers: JSON.parse(response.headers),
    url: response.url,
    redirected: response.redirected,
  });
}

function fetchAbortReason(signal) {
  return signal.reason instanceof Error ? signal.reason : new Error(String(signal.reason));
}

function installWebGlobals() {
  Object.assign(globalThis, {
    AbortController, AbortSignal, Blob, Body, Cache, CacheStorage, ByteLengthQueuingStrategy, CountQueuingStrategy,
    CloseEvent, CustomEvent, DOMException, ErrorEvent, Event, EventSource, EventTarget, ExtendableEvent, FetchEvent, File, FormData, Headers, MessageChannel, MessageEvent, MessagePort,
    PromiseRejectionEvent, ScheduledEvent, TailEvent, TraceEvent, WebSocketRequestResponsePair,
    ReadableByteStreamController, ReadableStream, ReadableStreamBYOBReader, ReadableStreamBYOBRequest, ReadableStreamDefaultController, ReadableStreamDefaultReader, Request, Response, TextDecoder, TextDecoderStream, TextEncoder, TextEncoderStream,
    Crypto, CryptoKey, SubtleCrypto,
    TransformStream, TransformStreamDefaultController, CompressionStream, DecompressionStream, FixedLengthStream, IdentityTransformStream, URL, URLPattern, URLSearchParams, WritableStream, WritableStreamDefaultController, WritableStreamDefaultWriter, structuredClone,
    Navigator, Performance, PerformanceEntry, PerformanceMark, PerformanceMeasure, PerformanceObserver, PerformanceObserverEntryList, PerformanceResourceTiming,
    WorkerGlobalScope, ServiceWorkerGlobalScope,
    atob, btoa,
    fetch, reportError, setTimeout, clearTimeout, setInterval, clearInterval, setImmediate, clearImmediate,
    caches: new CacheStorage(), crypto, HTMLRewriter, navigator: new Navigator(),
    origin: "null", self: globalThis, Cloudflare: { compatibilityFlags: [] },
  });
  Object.setPrototypeOf(globalThis, ServiceWorkerGlobalScope.prototype);
  installIntlGlobals();
  delete globalThis.WebAssembly;
  globalThis.performance = performance;
  globalThis.scheduler = {
    wait: (delay, options = {}) => {
      if (options.signal?.aborted) return Promise.reject(options.signal.reason);
      return new Promise((resolve, reject) => {
        const id = setTimeout(() => {
          if (options.signal?.aborted) reject(options.signal.reason);
          else resolve();
        }, delay);
        options.signal?.addEventListener("abort", () => {
          clearTimeout(id);
          reject(options.signal.reason);
        }, { once: true });
      });
    },
  };
}

export { crypto, fetch, installWebGlobals, performance, reportError };
