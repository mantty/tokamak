import { markHostObject } from "./objects.mjs";
import { decodeBase64, encodeBase64, randomBytes, digest, cryptoCreateDigest, cryptoHmac, cryptoAesGcm, cryptoPbkdf2, cryptoHkdf, cryptoGenerateKey, cryptoImportKey, cryptoExportKey, cryptoSign, cryptoVerify, cryptoEncrypt, cryptoDecrypt, cryptoDerive, cryptoTimingSafeEqual } from "tokamak:host";
import { bufferSourceBytes, sharedBufferSourceBytes } from "./conversions.mjs";
import { DOMException } from "./dom-exception.mjs";
import { TextDecoder, TextEncoder } from "../streams/text.mjs";
import { WritableStream } from "../streams/web.mjs";

export function randomUUID() {
  const bytes = randomBytes(16);
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = [...bytes].map(value => value.toString(16).padStart(2, "0")).join("");
  return hex.slice(0, 8) + "-" + hex.slice(8, 12) + "-" + hex.slice(12, 16) + "-" + hex.slice(16, 20) + "-" + hex.slice(20);
}

const keyRecords = new WeakMap();
const cryptoKeyBrand = {};
const hashNames = ["SHA-1", "SHA-256", "SHA-384", "SHA-512"];
const symmetricNames = ["AES-CTR", "AES-CBC", "AES-GCM", "AES-KW"];
const ecCurves = ["P-256", "P-384", "P-521"];

export class SubtleCrypto {
  constructor() { markHostObject(this); }
  async digest(algorithm, data) {
    return digest(normalizeDigest(algorithm), toBytes(data));
  }
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
      validateUsages(keyUsages, secretUsages(name));
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
    const key = cryptoImportKey({ format: formatName, kind: asymmetric.kind, curve: asymmetric.namedCurve, key: bytes });
    return asymmetricKey(importedAlgorithm(asymmetric, key), extractable, keyUsages, key, type === "private");
  }
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
    const output = cryptoExportKey({ format: formatName, keyFormat: record.meta.private ? "pkcs8" : "spki", kind: record.meta.kind, key: record.data });
    if (formatName === "jwk") return addJwkMetadata(JSON.parse(new TextDecoder().decode(output)), record);
    return output;
  }
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
      validateUsages(keyUsages, secretUsages(name));
      return secretKey({ name, length }, extractable, keyUsages, randomBytes(length / 8), { kind: "aes", format: "raw" });
    }
    if (name === "HKDF" || name === "PBKDF2") throw notSupported();
    const asymmetric = normalizeAsymmetricAlgorithm(algorithm);
    validateAsymmetricUsages(keyUsages, name, "pair");
    const key = cryptoGenerateKey({ kind: asymmetric.kind, curve: asymmetric.namedCurve, modulusLength: asymmetric.modulusLength, publicExponent: asymmetric.publicExponent });
    const publicKey = createKey(asymmetric.keyAlgorithm, extractable, asymmetricPublicUsages(name, keyUsages), "public", key.public, { kind: asymmetric.kind, format: "spki", private: false });
    const privateKey = createKey(asymmetric.keyAlgorithm, extractable, asymmetricPrivateUsages(name, keyUsages), "private", key.private, { kind: asymmetric.kind, format: "pkcs8", private: true });
    return { publicKey, privateKey };
  }
  async decrypt(algorithm, key, data) {
    validateKey(key, "decrypt");
    const record = requireAlgorithmKey(key, algorithm);
    if (record.type === "secret" && symmetricNames.includes(record.algorithm.name) && record.algorithm.name !== "AES-KW") return aesCrypt(false, record, algorithm, () => toBytes(data));
    if (record.algorithm.name !== "RSA-OAEP") throw notSupported();
    return rsaOaep(false, record, algorithm, toBytes(data), "pkcs8");
  }
  async encrypt(algorithm, key, data) {
    validateKey(key, "encrypt");
    const record = requireAlgorithmKey(key, algorithm);
    if (record.type === "secret" && symmetricNames.includes(record.algorithm.name) && record.algorithm.name !== "AES-KW") return aesCrypt(true, record, algorithm, () => toBytes(data));
    if (record.algorithm.name !== "RSA-OAEP") throw notSupported();
    return rsaOaep(true, record, algorithm, toBytes(data), "spki");
  }
  async sign(algorithm, key, data) {
    validateKey(key, "sign");
    const record = requireAlgorithmKey(key, algorithm);
    if (record.algorithm.name === "HMAC") return cryptoHmac(record.algorithm.hash.name, record.data, toBytes(data));
    const hash = signatureHash(record, algorithm);
    return cryptoSign(toBytes(data), { kind: record.meta.kind, format: record.meta.format, key: record.data, hash, saltLength: saltLength(record, algorithm) });
  }
  async verify(algorithm, key, signature, data) {
    validateKey(key, "verify");
    const record = requireAlgorithmKey(key, algorithm);
    const actual = toBytes(signature);
    if (record.algorithm.name === "HMAC") {
      const expected = new Uint8Array(cryptoHmac(record.algorithm.hash.name, record.data, toBytes(data)));
      if (expected.byteLength !== actual.byteLength) return false;
      return cryptoTimingSafeEqual(expected, actual);
    }
    const hash = signatureHash(record, algorithm);
    return cryptoVerify(actual, { kind: record.meta.kind, format: record.meta.format, key: record.data, hash, message: toBytes(data), saltLength: saltLength(record, algorithm) });
  }
  timingSafeEqual(left, right) { return cryptoTimingSafeEqual(toBytes(left), toBytes(right)); }
  async deriveBits(algorithm, key, length) {
    return deriveBitsWithUsage(algorithm, key, length, "deriveBits");
  }
  async deriveKey(algorithm, baseKey, derivedKeyAlgorithm, extractable, usages) {
    const length = derivedKeyLength(derivedKeyAlgorithm);
    const bits = new Uint8Array(await deriveBitsWithUsage(algorithm, baseKey, length, "deriveKey"));
    return importKey("raw", bits, derivedKeyAlgorithm, extractable, usages);
  }
  async wrapKey(format, key, wrappingKey, wrapAlgorithm) {
    const source = requireKey(key);
    if (!source.extractable) throw new DOMException("Key is not extractable", "InvalidAccessError");
    validateKey(wrappingKey, "wrapKey");
    const exported = await exportKey(format, key);
    const bytes = normalizeFormat(format) === "jwk" ? new TextEncoder().encode(JSON.stringify(exported)) : new Uint8Array(exported);
    return wrapCrypt(true, wrappingKey, wrapAlgorithm, bytes);
  }
  async unwrapKey(format, wrappedKey, unwrappingKey, unwrapAlgorithm, unwrappedKeyAlgorithm, extractable, usages) {
    validateKey(unwrappingKey, "unwrapKey");
    const bytes = await wrapCrypt(false, unwrappingKey, unwrapAlgorithm, toBytes(wrappedKey));
    const source = normalizeFormat(format) === "jwk" ? parseWrappedJwk(bytes) : bytes;
    return importKey(format, source, unwrappedKeyAlgorithm, extractable, usages);
  }
}
// deriveKey, wrapKey and unwrapKey use the original methods, whatever the prototype later holds.
const { importKey, exportKey } = SubtleCrypto.prototype;
const subtle = new SubtleCrypto();

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
function requiredBytes(algorithm, name, prefix = "") {
  if (algorithm === null || typeof algorithm !== "object" || algorithm[name] === undefined) throw operationError(`${prefix}${name} is required`);
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
    return cryptoHkdf(algorithmHash(algorithm), record.data, requiredBytes(algorithm, "salt"), optionalBytes(algorithm?.info), bitLength / 8);
  }
  if (name === "PBKDF2") {
    if (record.algorithm.name !== "PBKDF2") throw invalidAccess();
    return cryptoPbkdf2(algorithmHash(algorithm), record.data, requiredBytes(algorithm, "salt"), positiveInteger(algorithm?.iterations, "PBKDF2 iterations"), bitLength / 8);
  }
  if (name !== "ECDH" && name !== "X25519") throw notSupported();
  if (record.type !== "private" || record.algorithm.name !== name) throw invalidAccess();
  const peer = algorithm?.public;
  if (!(peer instanceof CryptoKey) || peer.type !== "public" || peer.algorithm.name !== name) throw invalidAccess();
  if (name === "ECDH" && record.algorithm.namedCurve !== peer.algorithm.namedCurve) throw invalidAccess();
  const output = cryptoDerive({ kind: name === "X25519" ? "x25519" : "ec", format: "pkcs8", keyFormat: "pkcs8", key: record.data, peer: requireKey(peer).data });
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
function secretUsages(name) {
  return name === "HMAC" ? ["sign", "verify"] : name === "AES-KW" ? ["wrapKey", "unwrapKey"] : ["encrypt", "decrypt"];
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
function requireAlgorithmKey(key, algorithm) {
  const record = requireKey(key);
  if (algorithmName(record.algorithm) !== algorithmName(algorithm)) throw notSupported();
  return record;
}
function toBytes(value) {
  const bytes = bufferSourceBytes(value);
  if (!bytes) throw new TypeError("Data must be an ArrayBuffer or ArrayBufferView");
  return bytes.slice();
}

function aesOptions(tagLength, mode, additionalData) { return { tagLength, mode, additionalData }; }
// AES-GCM, AES-CBC or AES-CTR. `input()` supplies the data once the IV or counter and the length are read.
function aesCrypt(encrypt, record, algorithm, input) {
  const counter = record.algorithm.name === "AES-CTR";
  const parameter = requiredBytes(algorithm, counter ? "counter" : "iv", "AES algorithm ");
  const length = counter ? ctrLength(algorithm) : tagLength(algorithm);
  return cryptoAesGcm(encrypt, record.data, parameter, input(), aesOptions(length, record.algorithm.name, optionalBytes(algorithm?.additionalData)));
}
function rsaOaep(encrypt, record, algorithm, bytes, keyFormat) {
  const options = { kind: "rsa-oaep", format: record.meta.format, keyFormat, key: record.data, hash: record.algorithm.hash.name, label: optionalBytes(algorithm?.label) };
  return encrypt ? cryptoEncrypt(bytes, options) : cryptoDecrypt(bytes, options);
}
function signatureHash(record, algorithm) {
  return record.algorithm.hash?.name ?? (["Ed25519", "NODE-ED25519"].includes(record.algorithm.name) ? "SHA-256" : algorithmHash(algorithm));
}
function saltLength(record, algorithm) {
  if (record.algorithm.name !== "RSA-PSS") return undefined;
  if (algorithm === null || typeof algorithm !== "object") throw new TypeError("RSA-PSS saltLength is required");
  const length = integer(algorithm.saltLength, "saltLength");
  if (length > 0x7fffffff) throw new TypeError("RSA-PSS saltLength is too large");
  return length;
}
function createKey(algorithm, extractable, usages, type, data, meta) { return new CryptoKey(algorithm, extractable, usages, type, data, cryptoKeyBrand, meta); }
function secretKey(algorithm, extractable, usages, data, meta) { return createKey(algorithm, extractable, usages, "secret", data, meta); }
function asymmetricKey(algorithm, extractable, usages, key, privateOnly) {
  if (privateOnly) return createKey(algorithm.keyAlgorithm, extractable, usages, "private", key.private, { kind: algorithm.kind, format: "pkcs8", private: true });
  return createKey(algorithm.keyAlgorithm, extractable, usages, "public", key.public, { kind: algorithm.kind, format: "spki", private: false });
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

function importJwk(jwk, algorithm, extractable, usages, expectedName) {
  if (jwk === null || typeof jwk !== "object" || jwk instanceof ArrayBuffer || ArrayBuffer.isView(jwk) || Array.isArray(jwk)) {
    throw new TypeError("JsonWebKey must be an object");
  }
  if (jwk.ext === false && extractable) throw dataError("JWK is not extractable");
  if (Array.isArray(jwk.key_ops) && jwk.key_ops.some(value => !usages.includes(value))) throw dataError("JWK key operations do not match usages");
  if (expectedName === "HMAC" || symmetricNames.includes(expectedName)) {
    if (jwk.kty !== "oct" || typeof jwk.k !== "string") throw dataError("JWK key type does not match the algorithm");
    const bytes = base64urlDecode(jwk.k);
    validateSecretLength(expectedName, bytes);
    const hash = expectedName === "HMAC" ? algorithmHash(algorithm) : undefined;
    if (expectedName === "HMAC") validateHmacImportLength(algorithm, bytes);
    validateUsages(usages, secretUsages(expectedName));
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
  const key = cryptoImportKey({ format: "jwk", kind: asymmetric.kind, curve: asymmetric.namedCurve, jwk: JSON.stringify(jwk) });
  return asymmetricKey(importedAlgorithm(asymmetric, key), extractable, usages, key, privateKey);
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
// An RSA key algorithm describes the imported key's modulus and exponent.
function importedAlgorithm(asymmetric, key) {
  if (key.modulusLength === undefined) return asymmetric;
  return { ...asymmetric, keyAlgorithm: { ...asymmetric.keyAlgorithm, modulusLength: key.modulusLength, publicExponent: key.publicExponent } };
}
function base64urlEncode(bytes) { return encodeBase64(bytes).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, ""); }
function base64urlDecode(value) {
  const bytes = /^[A-Za-z0-9_-]*$/.test(value) ? decodeBase64(value.replaceAll("-", "+").replaceAll("_", "/")) : null;
  if (!bytes) throw dataError("Invalid base64url value");
  return bytes;
}
function parseWrappedJwk(bytes) {
  try { return JSON.parse(new TextDecoder().decode(bytes)); }
  catch { throw dataError("Invalid JWK"); }
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
function wrapCrypt(encrypt, key, algorithm, bytes) {
  const record = requireAlgorithmKey(key, algorithm);
  const name = record.algorithm.name;
  if (name === "AES-KW") return cryptoAesGcm(encrypt, record.data, new Uint8Array(), bytes, aesOptions(128, "AES-KW"));
  if (name === "AES-GCM" || name === "AES-CBC" || name === "AES-CTR") return aesCrypt(encrypt, record, algorithm, () => bytes);
  if (name === "RSA-OAEP") return rsaOaep(encrypt, record, algorithm, bytes, encrypt && !record.meta.private ? "spki" : "pkcs8");
  throw notSupported();
}

export class DigestStream extends WritableStream {
  #digest;
  #written;

  constructor(algorithm) {
    const name = normalizeDigest(algorithm);
    const digest = Promise.withResolvers();
    const written = { bytes: 0n };
    let state = cryptoCreateDigest(name);
    const update = chunk => {
      const bytes = toBytes(chunk);
      state.update(bytes);
      written.bytes += BigInt(bytes.byteLength);
    };
    const fail = error => { state = null; digest.reject(error); };
    super({
      write(chunk) {
        try { update(chunk); }
        catch (error) { fail(error); throw error; }
      },
      close() {
        try { digest.resolve(state.finish()); }
        catch (error) { digest.reject(error); throw error; }
        finally { state = null; }
      },
      abort: fail,
    });
    this.#digest = digest.promise;
    this.#written = written;
  }
  get digest() { return this.#digest; }
  get bytesWritten() { return this.#written.bytes; }
}
Object.defineProperty(DigestStream.prototype, Symbol.toStringTag, { value: "DigestStream", configurable: true });

export class Crypto {
  constructor() { markHostObject(this); }
  get subtle() { return subtle; }
  get DigestStream() { return DigestStream; }
  getRandomValues(value) {
    if (!ArrayBuffer.isView(value)) throw new TypeError("Expected an integer typed array");
    if (value instanceof DataView || value instanceof Float32Array || value instanceof Float64Array || (typeof Float16Array !== "undefined" && value instanceof Float16Array)) throw new DOMException("The provided value is not an integer typed array.", "TypeMismatchError");
    if (value.byteLength > 65536) throw new DOMException("The requested length exceeds 65,536 bytes.", "QuotaExceededError");
    sharedBufferSourceBytes(value).set(randomBytes(value.byteLength));
    return value;
  }
  randomUUID() { return randomUUID(); }
}

export const crypto = new Crypto();
