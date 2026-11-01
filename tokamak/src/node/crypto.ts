import { cryptoHkdf, cryptoPbkdf2, cryptoCheckPrime, cryptoCreateCipher, cryptoCreateDigest, cryptoDecrypt, cryptoDhCompute, cryptoDhGenerate, cryptoDhParams, cryptoEcdhCompute, cryptoEcdhConvert, cryptoEcdhPublic, cryptoEncrypt, cryptoExportKey, cryptoGenerateKey, cryptoGeneratePrime, cryptoImportKey, cryptoRsaLegacyPrivateEncrypt, cryptoRsaLegacyPublicDecrypt, cryptoScrypt, cryptoSign, cryptoTimingSafeEqual, cryptoVerify, digest, randomBytes as hostRandomBytes } from "tokamak:host";
import type { CipherState, DigestState, HostKey, KeyGeneration } from "tokamak:host";
import { bufferBytes, bufferSourceBytes, joinBytes } from "../globals/conversions.ts";
import { CryptoKey, crypto as webcrypto, hostJwk } from "../globals/crypto.ts";
import { DOMException } from "../globals/dom-exception.ts";
import { callFunction, isFunction, objectOf, propertyOf } from "../globals/objects.ts";
import type { AnyFunction } from "../globals/objects.ts";
import { Transform } from "../streams/node.js";
import type { StreamCallback } from "../streams/node.js";
import { Buffer } from "./buffer.ts";
import { unsupportedFunction } from "./unsupported.ts";
import { codedError } from "./errors.ts";
import { invalidArgValue, validateNumber, validateString } from "./validators.ts";

const unsupportedCrypto = (name: string) => unsupportedFunction(`crypto.${name}`);
const hashToken = Symbol("hash");
const keyObjectToken = Symbol("keyObject");

export const subtle = webcrypto.subtle;
export { webcrypto };
export const randomUUID = webcrypto.randomUUID;
export const getRandomValues = webcrypto.getRandomValues;

type KeyType = "secret" | "public" | "private";

/** Node's details of an asymmetric key, each undefined where it does not apply. */
interface KeyDetails {
  modulusLength: number | undefined;
  publicExponent: bigint | undefined;
  divisorLength: undefined;
  namedCurve: string | undefined;
}

/** The state of a KeyObject: its key's bytes in `format`, of the kind the host knows it as. */
interface KeyRecord {
  type: KeyType;
  kind: string;
  format: string;
  bytes: Uint8Array;
  details?: KeyDetails;
}

function normalizeHash(algorithm: unknown) {
  const name = String(algorithm).toUpperCase().replaceAll("_", "-");
  return name.startsWith("SHA") && !name.startsWith("SHA-") ? "SHA-" + name.slice(3) : name;
}

function integerInRange(value: unknown, name: string, min: number) {
  const number = Number(value);
  if (!Number.isSafeInteger(number) || number < min) {
    throw codedError(RangeError, `The value of "${name}" is out of range. Received ${value}`, "ERR_OUT_OF_RANGE");
  }
  return number;
}

// Reports the result or error of `compute` to `callback` on a microtask.
function deliver(callback: AnyFunction, compute: () => unknown) {
  try {
    const value = compute();
    queueMicrotask(() => callback(null, value));
  } catch (error) {
    queueMicrotask(() => callback(error));
  }
}

// Returns the result of `compute` without a callback; with one, returns nothing
// and, when `report` is truthy, reports the result or error on a microtask.
function returnOrDeliver<T>(callback: unknown, compute: () => T, report = callback) {
  try {
    const value = compute();
    if (report) queueMicrotask(() => callFunction(callback, undefined, null, value));
    return callback ? undefined : value;
  } catch (error) {
    if (report) { queueMicrotask(() => callFunction(callback, undefined, error)); return undefined; }
    throw error;
  }
}

function transformUpdate(stream: { update(value: unknown, encoding: unknown): unknown }, chunk: unknown, encoding: unknown, callback: StreamCallback) {
  try { stream.update(chunk, encoding); callback(); } catch (error) { callback(error); }
}

function finalizedError() {
  return codedError(Error, "Digest already called", "ERR_CRYPTO_HASH_FINALIZED");
}

function nodeHashAlgorithm(algorithm: unknown) {
  if (typeof algorithm !== "string") throw codedError(TypeError, 'The "algorithm" argument must be of type string', "ERR_INVALID_ARG_TYPE");
  const name = getHashes().find(name => name.toLowerCase() === algorithm.toLowerCase());
  if (!name) throw new Error("Digest method not supported");
  if (["DSA-SHA", "DSA-SHA1", "ecdsa-with-SHA1"].includes(name)) return "SHA-1";
  return normalizeHash(name.replace(/^RSA-/, ""));
}

export class Hash extends Transform {
  declare __algorithm: unknown;
  declare __handle: DigestState;
  declare __finalized: boolean;

  constructor(algorithm: unknown, token: unknown, handle?: DigestState) {
    super();
    if (token !== hashToken) throw new Error("Illegal constructor");
    this.__algorithm = algorithm;
    this.__handle = handle ?? cryptoCreateDigest(nodeHashAlgorithm(algorithm));
    this.__finalized = false;
  }

  update(value: unknown, encoding?: unknown) { return updateDigest(this, value, encoding); }

  digest(encoding?: unknown) {
    if (this.__finalized) throw finalizedError();
    this.__finalized = true;
    return outputEncoding(Buffer.from(this.__handle.finish()), encoding);
  }

  copy() {
    if (this.__finalized) throw finalizedError();
    return new Hash(this.__algorithm, hashToken, this.__handle.copy());
  }

  _transform(chunk: unknown, encoding: BufferEncoding, callback: StreamCallback) { transformUpdate(this, chunk, encoding, callback); }
  _flush(callback: StreamCallback) { flushDigest(this, callback); }
}

export function createHash(algorithm: unknown) { return new Hash(algorithm, hashToken); }

// Workers rejects requests above the Web Crypto quota; matching keeps apps portable.
export function randomBytesSync(length: unknown) {
  const size = integerInRange(length, "size", 0);
  if (size > 65536) {
    throw new DOMException(
      `The requested length exceeds the quota (${size} > 65536)`,
      "QuotaExceededError",
    );
  }
  return Buffer.from(hostRandomBytes(size));
}

export function randomBytes(length: unknown, callback?: unknown) {
  if (callback === undefined) return randomBytesSync(length);
  if (!isFunction(callback)) throw new TypeError("The callback argument must be of type function");
  deliver(callback, () => randomBytesSync(length));
}

export function randomFillSync(buffer: unknown, offset: unknown = 0, size?: unknown) {
  const view = bufferBytes(buffer);
  if (!view) throw new TypeError("The first argument must be an ArrayBuffer or a view on an ArrayBuffer");
  const start = Number(offset);
  const length = size === undefined ? view.byteLength - start : Number(size);
  if (!Number.isSafeInteger(start) || start < 0 || !Number.isSafeInteger(length) || length < 0 || start + length > view.byteLength) {
    throw codedError(RangeError, "The value of \"offset\" is out of range", "ERR_OUT_OF_RANGE");
  }
  webcrypto.getRandomValues(view.subarray(start, start + length));
  return buffer;
}

export function randomFill(buffer: unknown, offset: unknown, size: unknown, callback: unknown) {
  const [start, length, done] = isFunction(offset) ? [0, undefined, offset] : isFunction(size) ? [offset, undefined, size] : [offset, size, callback];
  if (!isFunction(done)) throw codedError(TypeError, "The callback argument must be of type function", "ERR_INVALID_ARG_TYPE");
  deliver(done, () => randomFillSync(buffer, start, length));
}

export function randomInt(min: unknown, max: unknown, callback?: unknown) {
  const [low, high, done] = isFunction(max) ? [0, Number(min), max] : [Number(min), Number(max), callback];
  if (!Number.isSafeInteger(low) || !Number.isSafeInteger(high) || high <= low) throw codedError(RangeError, "Invalid randomInt range", "ERR_OUT_OF_RANGE");
  const range = high - low;
  const randomSpace = 2 ** 53;
  if (range > randomSpace) throw new RangeError("randomInt range is too large");
  const limit = randomSpace - (randomSpace % range);
  let value;
  do {
    const words = webcrypto.getRandomValues(new Uint32Array(2));
    value = (words[0] & 0x1fffff) * 2 ** 32 + words[1];
  } while (value >= limit);
  const result = low + (value % range);
  if (done === undefined) return result;
  if (!isFunction(done)) throw new TypeError("The callback argument must be of type function");
  queueMicrotask(() => done(null, result));
}

export function timingSafeEqual(left: unknown, right: unknown) {
  const a = bufferSourceBytes(left);
  const b = bufferSourceBytes(right);
  if (!a || !b) throw new TypeError("Input must be an ArrayBuffer or ArrayBufferView");
  if (a.byteLength !== b.byteLength) throw new TypeError("Input buffers must have the same byte length");
  return cryptoTimingSafeEqual(a, b);
}

export function createHmac(algorithm: unknown, key: unknown) { return new Hmac(algorithm, key, hashToken); }

export class Hmac extends Transform {
  declare __handle: DigestState;
  declare __finalized: boolean;

  constructor(algorithm: unknown, key: unknown, token: unknown) {
    super();
    if (token !== hashToken) throw new Error("Illegal constructor");
    const bytes = key instanceof KeyObject ? keyRecord(key).bytes : Buffer.from(key);
    this.__handle = cryptoCreateDigest(nodeHashAlgorithm(algorithm), bytes);
    this.__finalized = false;
  }

  update(value: unknown, encoding?: unknown) { return updateDigest(this, value, encoding); }

  digest(encoding?: unknown) {
    if (this.__finalized) return encoding === undefined ? Buffer.alloc(0) : "";
    this.__finalized = true;
    return outputEncoding(Buffer.from(this.__handle.finish()), encoding);
  }

  _transform(chunk: unknown, encoding: BufferEncoding, callback: StreamCallback) { transformUpdate(this, chunk, encoding, callback); }
  _flush(callback: StreamCallback) { try { callback(null, this.digest()); } catch (error) { callback(error); } }
}

export function hash(algorithm: unknown, data: unknown, options?: unknown) {
  if (options !== undefined) throw codedError(TypeError, "The \"options\" argument must be of type string or an instance of Buffer or Uint8Array", "ERR_INVALID_ARG_TYPE");
  return createHash(algorithm).update(data).digest("hex");
}

export function getFips() { return true; }
export const fips = true;
export function getHashes() {
  return [
    "md4", "md5", "sha1", "sha224", "sha256", "sha384", "sha512", "md5-sha1",
    "RSA-MD5", "RSA-SHA1", "RSA-SHA224", "RSA-SHA256", "RSA-SHA384", "RSA-SHA512",
    "DSA-SHA", "DSA-SHA1", "ecdsa-with-SHA1",
  ];
}
export function getCiphers() {
  return [
    "AES-128-CBC", "AES-192-CBC", "AES-256-CBC", "AES-128-CTR", "AES-192-CTR", "AES-256-CTR",
    "AES-128-ECB", "AES-192-ECB", "AES-256-ECB", "AES-128-OFB", "AES-192-OFB", "AES-256-OFB",
    "AES-128-GCM", "AES-192-GCM", "AES-256-GCM", "DES-CBC", "DES-ECB", "DES-EDE", "DES-EDE-CBC",
    "DES-EDE3-CBC", "RC2-CBC", "RC4", "aes-128-cbc", "aes-192-cbc", "aes-256-cbc", "aes-128-ctr",
    "aes-192-ctr", "aes-256-ctr", "aes-128-ecb", "aes-192-ecb", "aes-256-ecb", "aes-128-ofb",
    "aes-192-ofb", "aes-256-ofb", "aes-128-gcm", "aes-192-gcm", "aes-256-gcm", "des-cbc", "des-ecb",
    "des-ede", "des-ede-cbc", "des-ede3-cbc", "rc2-cbc", "rc4", "aes-256-eax", "aes-128-eax",
    "aes-128-ccm-bluetooth-8", "aes-128-ccm-matter", "aes-128-ccm-bluetooth", "aes-192-gcm", "aes-128-gcm",
    "aes-128-gcm-siv", "aes-256-gcm-siv", "aes-256-gcm-randnonce", "aes-256-ctr-hmac-sha256",
    "aes-128-ctr-hmac-sha256", "xchacha20-poly1305", "aes-256-gcm", "chacha20-poly1305", "aes-128-gcm-randnonce",
  ];
}
export function getCurves() { return ["secp224r1", "prime256v1", "secp384r1", "secp521r1"]; }
export const constants = {};
export function getDiffieHellman(name: unknown) { return createDiffieHellmanGroup(name); }
export function secureHeapUsed() { return { total: 0, used: 0, utilization: 0, min: 0 }; }

function illegalConstructor(): never { throw new Error("Illegal constructor"); }

export const Certificate = class Certificate { constructor() { illegalConstructor(); } };
const keyAgreementToken = Symbol("keyAgreement");
export class DiffieHellman {
  declare __prime: Buffer;
  declare __generator: Buffer;
  declare __privateKey: Buffer | null;
  declare __publicKey: Buffer | null;

  constructor(prime: unknown, generator: unknown, token: unknown) {
    if (token !== keyAgreementToken) illegalConstructor();
    this.__prime = Buffer.from(prime);
    this.__generator = Buffer.from(generator);
    this.__privateKey = null;
    this.__publicKey = null;
  }

  computeSecret(otherPublicKey: unknown, inputEncoding?: unknown, outputEncodingName?: unknown) {
    if (!this.__privateKey) throw new Error("Private key is not set");
    const output = cryptoDhCompute({
      prime: this.__prime,
      private: this.__privateKey,
      peer: Buffer.from(otherPublicKey, inputEncoding),
    });
    return outputEncoding(Buffer.from(output), outputEncodingName);
  }

  generateKeys(encoding?: unknown) {
    const keys = cryptoDhGenerate({ prime: this.__prime, generator: this.__generator, private: this.__privateKey ?? undefined });
    this.__publicKey = Buffer.from(keys.public);
    this.__privateKey = Buffer.from(keys.private);
    return outputEncoding(this.__publicKey, encoding);
  }

  getGenerator(encoding?: unknown) { return outputEncoding(Buffer.from(this.__generator), encoding); }
  getPrime(encoding?: unknown) { return outputEncoding(Buffer.from(this.__prime), encoding); }
  getPrivateKey(encoding?: unknown) {
    if (!this.__privateKey) throw new Error("Private key is not set");
    return outputEncoding(Buffer.from(this.__privateKey), encoding);
  }
  getPublicKey(encoding?: unknown) {
    if (!this.__publicKey) throw new Error("Public key is not set");
    return outputEncoding(Buffer.from(this.__publicKey), encoding);
  }
  setPrivateKey(privateKey: unknown, encoding?: unknown) {
    const value = Buffer.from(privateKey, encoding);
    const keys = cryptoDhGenerate({ prime: this.__prime, generator: this.__generator, private: value });
    this.__privateKey = Buffer.from(keys.private);
    this.__publicKey = Buffer.from(keys.public);
    return this;
  }
  setPublicKey(publicKey: unknown, encoding?: unknown) {
    this.__publicKey = Buffer.from(publicKey, encoding);
    return this;
  }
  get verifyError() { return 0; }
}

export class DiffieHellmanGroup extends DiffieHellman {}

export class ECDH {
  declare __curve: string;
  declare __privateKey: Buffer | null;
  declare __publicKey: Buffer | null;

  constructor(curve: unknown, token: unknown) {
    if (token !== keyAgreementToken) illegalConstructor();
    this.__curve = webCurveName(curve);
    this.__privateKey = null;
    this.__publicKey = null;
  }

  computeSecret(otherPublicKey: unknown, inputEncoding?: unknown, outputEncodingName?: unknown) {
    if (!this.__privateKey) throw new Error("Private key is not set");
    const output = cryptoEcdhCompute({ curve: this.__curve, private: this.__privateKey }, Buffer.from(otherPublicKey, inputEncoding));
    return outputEncoding(Buffer.from(output), outputEncodingName);
  }

  generateKeys(encoding?: unknown, format: unknown = "uncompressed") {
    const jwk = exportJwk("pkcs8", "ec", hostKeyBytes(cryptoGenerateKey({ kind: "ec", curve: this.__curve }), true));
    this.__privateKey = Buffer.from(jwk.d, "base64url");
    this.__publicKey = publicPoint(jwk);
    return this.getPublicKey(encoding, format);
  }

  getPrivateKey(encoding?: unknown) {
    if (!this.__privateKey) throw new Error("Private key is not set");
    return outputEncoding(Buffer.from(this.__privateKey), encoding);
  }

  getPublicKey(encoding?: unknown, format: unknown = "uncompressed") {
    if (!this.__publicKey) throw new Error("Public key is not set");
    const value = format === "uncompressed" ? this.__publicKey : Buffer.from(cryptoEcdhConvert(this.__publicKey, { curve: this.__curve, format: pointFormat(format) }));
    return outputEncoding(value, encoding);
  }

  setPrivateKey(privateKey: unknown, encoding?: unknown) {
    this.__privateKey = Buffer.from(privateKey, encoding);
    this.__publicKey = Buffer.from(cryptoEcdhPublic({ curve: this.__curve, private: this.__privateKey }));
    return this;
  }

  setPublicKey(publicKey: unknown, encoding?: unknown) {
    this.__publicKey = Buffer.from(cryptoEcdhConvert(Buffer.from(publicKey, encoding), { curve: this.__curve, format: "uncompressed" }));
    return this;
  }

  static convertKey(key: unknown, curve: unknown, inputEncoding?: unknown, outputEncodingName?: unknown, format: unknown = "uncompressed") {
    const value = Buffer.from(cryptoEcdhConvert(Buffer.from(key, inputEncoding), { curve: webCurveName(curve), format: pointFormat(format) }));
    return outputEncoding(value, outputEncodingName);
  }
}

// An EC point format, which the host checks is "compressed" or "uncompressed".
function pointFormat(format: unknown) {
  if (typeof format !== "string") throw new TypeError("The point format must be \"compressed\" or \"uncompressed\"");
  return format;
}

function nodeCurveName(curve: unknown) {
  const name = String(curve);
  if (name === "prime256v1" || name === "P-256") return "prime256v1";
  if (name === "secp384r1" || name === "P-384") return "secp384r1";
  if (name === "secp521r1" || name === "P-521") return "secp521r1";
  throw new Error(`Invalid EC curve name: ${curve}`);
}

function webCurveName(curve: unknown) {
  return { "prime256v1": "P-256", "secp384r1": "P-384", "secp521r1": "P-521" }[nodeCurveName(curve)];
}

function publicPoint(jwk: Record<PropertyKey, unknown>) {
  const x = Buffer.from(jwk.x, "base64url");
  const y = Buffer.from(jwk.y, "base64url");
  return Buffer.concat([Buffer.from([4]), x, y]);
}

const keyObjectState = new WeakMap<KeyObject, KeyRecord>();
const signToken = Symbol("sign");

function pemDecode(value: unknown) {
  const match = String(value).match(/-----BEGIN ([^-]+)-----([\s\S]*?)-----END \1-----/);
  if (!match) throw new TypeError("Invalid PEM formatted message");
  const label = match[1];
  return {
    bytes: Buffer.from(match[2].replace(/\s/g, ""), "base64"),
    format: label === "PRIVATE KEY" ? "pkcs8" : label === "PUBLIC KEY" ? "spki" : "der",
    type: label.includes("PRIVATE") ? "private" : "public",
  } as const;
}

function pemEncode(bytes: Uint8Array, label: string) {
  const encoded = Buffer.from(bytes).toString("base64");
  const lines = encoded.match(/.{1,64}/g)?.join("\n") ?? "";
  return `-----BEGIN ${label}-----\n${lines}\n-----END ${label}-----\n`;
}

function keyKind(jwk: unknown) {
  if (propertyOf(jwk, "kty") === "RSA") return "rsa";
  if (propertyOf(jwk, "kty") === "EC") return "ec";
  if (propertyOf(jwk, "kty") === "OKP" && propertyOf(jwk, "crv") === "Ed25519") return "ed25519";
  if (propertyOf(jwk, "kty") === "OKP" && propertyOf(jwk, "crv") === "X25519") return "x25519";
  throw new TypeError("Unsupported key type");
}

function keyFromJwk(jwk: unknown) {
  return createAsymmetricObject(asymmetricRecord(cryptoImportKey({ format: "jwk", kind: keyKind(jwk), jwk: JSON.stringify(jwk) })));
}

// The state of a KeyObject for a key the host imported or generated, as its private key when it has one.
function asymmetricRecord(key: HostKey, type: KeyType = key.private ? "private" : "public"): KeyRecord {
  const isPrivate = type === "private";
  return { type, kind: key.kind, format: isPrivate ? "pkcs8" : "spki", bytes: hostKeyBytes(key, isPrivate), details: keyDetails(key) };
}

// The DER bytes of `key`'s private key, in PKCS#8, or of its public key, in SPKI.
function hostKeyBytes(key: HostKey, isPrivate: boolean) {
  const bytes = isPrivate ? key.private : key.public;
  if (bytes === undefined) throw new TypeError("Invalid key material");
  return bytes;
}

// Every detail Node reports, undefined where it does not apply to the key.
function keyDetails(key: HostKey): KeyDetails {
  return {
    modulusLength: key.modulusLength,
    publicExponent: key.publicExponent && bigIntFromBytes(key.publicExponent),
    divisorLength: undefined,
    namedCurve: key.namedCurve && nodeCurveName(key.namedCurve),
  };
}

function bigIntFromBytes(bytes: Uint8Array) {
  let value = 0n;
  for (const byte of bytes) value = (value << 8n) | BigInt(byte);
  return value;
}

function exportJwk(keyFormat: string, kind: string, key: Uint8Array) {
  return hostJwk(cryptoExportKey({ format: "jwk", keyFormat, kind, key }));
}

// The key DER-encoded in `bytes` as `format`, which must be a `type` key when a type is given.
function importDer(bytes: Buffer, format: unknown, type?: KeyType) {
  if (typeof format !== "string") throw new TypeError("Invalid key material");
  let key;
  try { key = cryptoImportKey({ format, key: bytes }); }
  catch { throw new TypeError("Invalid key material"); }
  const record = asymmetricRecord(key);
  if (type !== undefined && record.type !== type) throw new TypeError("Invalid key material");
  return record;
}

function keyRecord(key: unknown) {
  const record = key instanceof KeyObject ? keyObjectState.get(key) : undefined;
  if (!record) throw new TypeError("The key argument must be a KeyObject");
  return record;
}

function createAsymmetricObject(record: KeyRecord) {
  const Constructor = record.type === "private" ? PrivateKeyObject : PublicKeyObject;
  const key = new Constructor(keyObjectToken);
  keyObjectState.set(key, record);
  return key;
}

function hostKeyExport(record: KeyRecord, format: string) {
  return Buffer.from(cryptoExportKey({ format, keyFormat: record.format, kind: record.kind, key: Uint8Array.from(record.bytes) }));
}

function encodeKey(record: KeyRecord, options: unknown) {
  if (record.type === "secret") return Buffer.from(record.bytes);
  const settings = typeof options === "object" ? options : undefined;
  const format = propertyOf(settings, "format") ?? "pem";
  if (format === "jwk") return exportJwk(record.format, record.kind, Uint8Array.from(record.bytes));
  const type = propertyOf(settings, "type") ?? (record.type === "private" ? "pkcs8" : "spki");
  if (typeof type !== "string") throw invalidArgValue("options.type", type);
  const der = hostKeyExport(record, type);
  if (format === "der") return der;
  if (format === "pem") {
    const label = type === "pkcs8" ? "PRIVATE KEY" : type === "spki" ? "PUBLIC KEY" : record.type === "private" ? "RSA PRIVATE KEY" : "RSA PUBLIC KEY";
    return pemEncode(der, label);
  }
  throw new TypeError("The \"format\" argument must be one of: \"der\", \"pem\", or \"jwk\"");
}

export class KeyObject {
  constructor(token: unknown) {
    if (token !== keyObjectToken) illegalConstructor();
  }
  get type() { return keyObjectState.get(this)?.type; }
  get asymmetricKeyType() { return keyObjectState.get(this)?.kind; }
  get asymmetricKeyDetails() { return keyObjectState.get(this)?.details; }
  get symmetricKeySize() {
    const record = keyObjectState.get(this);
    return record?.type === "secret" ? record.bytes.byteLength : undefined;
  }
  export(options?: unknown) { return encodeKey(keyRecord(this), options); }
  equals(other: unknown) {
    if (!(other instanceof KeyObject)) throw new TypeError("The \"otherKeyObject\" argument must be an instance of KeyObject");
    const left = keyRecord(this).bytes;
    const right = keyRecord(other).bytes;
    return left.byteLength === right.byteLength && left.every((value, index) => value === right[index]);
  }
}
export class PrivateKeyObject extends KeyObject {}
export class PublicKeyObject extends KeyObject {}
export class SecretKeyObject extends KeyObject {
  constructor(bytes: unknown, token: unknown) {
    super(token);
    keyObjectState.set(this, { type: "secret", kind: "secret", format: "raw", bytes: Buffer.from(bytes) });
  }
}
/** The constructor body the signing classes share. */
function initSigning(instance: Sign | Verify, algorithm: unknown, token: unknown) {
  if (token !== signToken) illegalConstructor();
  instance.__algorithm = normalizeHash(algorithm);
  instance.__chunks = [];
  instance.__finalized = false;
}
export class Sign extends Transform {
  declare __algorithm: string;
  declare __chunks: Buffer[];
  declare __finalized: boolean;

  constructor(algorithm: unknown, token: unknown) {
    super();
    initSigning(this, algorithm, token);
  }
  update(value: unknown, encoding?: unknown) { return updateSigning(this, value, encoding); }
  sign(key: unknown, options?: unknown, callback?: unknown) {
    const [settings, done] = isFunction(options) ? [undefined, options] : [options, callback];
    return returnOrDeliver(done, () => {
      const record = keyRecord(key instanceof KeyObject ? key : createPrivateKey(key));
      if (record.type !== "private") throw new TypeError("A private key is required");
      const kind = signingKind(record, settings);
      const output = Buffer.from(cryptoSign(joinBytes(this.__chunks), { kind, format: record.format, key: record.bytes, hash: this.__algorithm, saltLength: pssSaltLength(kind, settings) }));
      this.__finalized = true;
      return outputEncoding(output, typeof settings === "string" ? settings : undefined);
    });
  }
  _transform(chunk: unknown, encoding: BufferEncoding, callback: StreamCallback) { transformUpdate(this, chunk, encoding, callback); }
  _flush(callback: StreamCallback) { callback(); }
}
export class Verify extends Transform {
  declare __algorithm: string;
  declare __chunks: Buffer[];
  declare __finalized: boolean;

  constructor(algorithm: unknown, token: unknown) {
    super();
    initSigning(this, algorithm, token);
  }
  update(value: unknown, encoding?: unknown) { return updateSigning(this, value, encoding); }
  verify(key: unknown, signature?: unknown, callback?: unknown) {
    const [publicKey, actual, done] = isFunction(signature) ? [undefined, key, signature] : [key, signature, callback];
    return returnOrDeliver(done, () => {
      const record = keyRecord(publicKey instanceof KeyObject ? publicKey : createPublicKey(publicKey));
      const kind = signingKind(record);
      const value = cryptoVerify(Buffer.from(actual), joinBytes(this.__chunks), { kind, format: record.format, key: record.bytes, hash: this.__algorithm });
      this.__finalized = true;
      return value;
    });
  }
  _transform(chunk: unknown, encoding: BufferEncoding, callback: StreamCallback) { transformUpdate(this, chunk, encoding, callback); }
  _flush(callback: StreamCallback) { callback(); }
}
export const X509Certificate = class X509Certificate { constructor() { illegalConstructor(); } };

/** The `{ key, format, type }` settings object `input` carries, if any. */
function keySettings(input: unknown) {
  return typeof input === "object" && input !== null && !(input instanceof ArrayBuffer) && !ArrayBuffer.isView(input) ? objectOf(input) : {};
}

function flushDigest(instance: Hash, callback: StreamCallback) {
  try { callback(null, instance.digest()); } catch (error) { callback(error); }
}

function updateSigning<T extends Sign | Verify>(instance: T, value: unknown, encoding: unknown) {
  if (instance.__finalized) throw finalizedError();
  instance.__chunks.push(Buffer.from(value, encoding));
  return instance;
}

function updateDigest<T extends Hash | Hmac>(instance: T, value: unknown, encoding: unknown) {
  if (instance.__finalized) throw finalizedError();
  instance.__handle.update(Buffer.from(value, encoding));
  return instance;
}

function outputEncoding(value: Buffer, encoding: unknown) { return encoding === undefined ? value : value.toString(encoding); }

function signingKind(record: KeyRecord, options?: unknown) {
  const padding = propertyOf(options, "padding");
  if (record.kind === "rsa" && (padding === 6 || padding === "RSA_PKCS1_PSS_PADDING")) return "rsa-pss";
  return record.kind === "ec" ? "ecdsa" : record.kind;
}

// The salt length of an RSA-PSS signature, which `options` gives; other schemes have none.
function pssSaltLength(kind: string, options: unknown) {
  const length = propertyOf(options, "saltLength");
  if (kind !== "rsa-pss" || length === undefined) return undefined;
  if (typeof length !== "number") throw invalidArgValue("options.saltLength", length);
  return length;
}

function cipherInput(value: unknown, encoding: unknown) {
  if (typeof value === "string" && encoding === undefined) {
    throw codedError(TypeError, "The argument 'inputEncoding' If inputEncoding is not provided then the data must be a Buffer. Received undefined", "ERR_INVALID_ARG_VALUE");
  }
  return Buffer.from(value, encoding);
}

function cipherSpec(algorithm: unknown) {
  const match = String(algorithm).toLowerCase().match(/^aes-(128|192|256)-(gcm|cbc|ctr)$/);
  if (!match) throw new Error(`Unknown cipher: ${algorithm}`);
  return { keyLength: Number(match[1]) / 8, mode: match[2] };
}

const gcmTagLengths = [4, 8, 12, 13, 14, 15, 16];

// The tag length, in bytes, that a GCM cipher's `options` give; other modes have no tag.
function gcmTagLength(mode: string, options: unknown) {
  const length = propertyOf(options, "authLengthTag");
  if (mode !== "gcm" || length === undefined) return undefined;
  if (typeof length !== "number" || !gcmTagLengths.includes(length)) throw new Error("Invalid authentication tag length");
  return length;
}

class CipherBase extends Transform {
  declare __tagLength: number | undefined;
  declare __handle: CipherState;

  constructor(algorithm: unknown, key: unknown, iv: unknown, decrypt: boolean, options: unknown = {}) {
    super(options);
    const spec = cipherSpec(algorithm);
    const keyBytes = key instanceof KeyObject ? keyRecord(key).bytes : Buffer.from(key);
    if (keyBytes.length !== spec.keyLength) throw new RangeError("Invalid key length");
    const tagLength = gcmTagLength(spec.mode, options);
    const ivBytes = Buffer.from(iv);
    if (spec.mode !== "gcm" && ivBytes.length !== 16) throw new TypeError("Invalid initialization vector");
    if (spec.mode === "gcm" && ivBytes.length !== 12) throw new Error("Invalid initialization vector");
    this.__tagLength = tagLength;
    this.__handle = cryptoCreateCipher(spec.mode, !decrypt, keyBytes, ivBytes, tagLength);
  }
  update(value: unknown, inputEncoding?: unknown, outputEncodingName?: unknown) {
    return outputEncoding(Buffer.from(this.__handle.update(cipherInput(value, inputEncoding))), outputEncodingName);
  }
  final(outputEncodingName?: unknown) {
    return outputEncoding(Buffer.from(this.__handle.finish()), outputEncodingName);
  }
  setAAD(value: unknown, options: unknown = {}) { this.__handle.setAAD(Buffer.from(value, propertyOf(options, "encoding"))); return this; }
  setAutoPadding(value: unknown) { this.__handle.setAutoPadding(Boolean(value)); return this; }
  getAuthTag() { return Buffer.from(this.__handle.getAuthTag()); }
  _transform(chunk: unknown, encoding: BufferEncoding, callback: StreamCallback) { try { callback(null, this.update(chunk, encoding)); } catch (error) { callback(error); } }
  _flush(callback: StreamCallback) { try { callback(null, this.final()); } catch (error) { callback(error); } }
}

export class Cipheriv extends CipherBase {
  constructor(algorithm: unknown, key: unknown, iv: unknown, options: unknown) { super(algorithm, key, iv, false, options); }
}

export class Decipheriv extends CipherBase {
  constructor(algorithm: unknown, key: unknown, iv: unknown, options: unknown) { super(algorithm, key, iv, true, options); }
  setAuthTag(value: unknown) {
    const tag = Buffer.from(value);
    if (!gcmTagLengths.includes(tag.length) || (this.__tagLength !== undefined && this.__tagLength !== tag.length)) throw new Error("Invalid authentication tag length");
    this.__handle.setAuthTag(tag);
    return this;
  }
}

export class Cipher extends Cipheriv {}
export class Decipher extends Decipheriv {}

function primeChecks(options: unknown) {
  const checks = Number(propertyOf(options, "checks") ?? 0);
  if (!Number.isSafeInteger(checks) || checks < 0) {
    throw codedError(RangeError, `The value of "checks" is out of range. Received ${propertyOf(options, "checks")}`, "ERR_OUT_OF_RANGE");
  }
  return checks;
}

export function checkPrimeSync(candidate: unknown, options: unknown = {}) {
  return cryptoCheckPrime(Buffer.from(candidate), { checks: primeChecks(options) });
}

export function checkPrime(candidate: unknown, options: unknown, callback: unknown) {
  const [settings, done] = isFunction(options) ? [{}, options] : [options, callback];
  if (!isFunction(done)) throw new TypeError("The \"callback\" argument must be of type function");
  deliver(done, () => checkPrimeSync(candidate, settings));
}

function primeBytes(size: unknown, options: unknown = {}) {
  const bits = integerInRange(size, "size", 2);
  const safe = Boolean(propertyOf(options, "safe"));
  const add = propertyOf(options, "add");
  const rem = propertyOf(options, "rem");
  const value = cryptoGeneratePrime({
    bits,
    safe,
    add: add === undefined ? undefined : Buffer.from(add),
    rem: rem === undefined ? undefined : Buffer.from(rem),
  });
  return Buffer.from(value);
}

export function generatePrimeSync(size: unknown, options: unknown = {}) {
  const value = primeBytes(size, options);
  return propertyOf(options, "bigint") ? bigIntFromBytes(value) : Uint8Array.from(value).buffer;
}

export function generatePrime(size: unknown, options: unknown, callback: unknown) {
  const [settings, done] = isFunction(options) ? [{}, options] : [options, callback];
  if (!isFunction(done)) throw new TypeError("The \"callback\" argument must be of type function");
  deliver(done, () => generatePrimeSync(size, settings));
}

interface CipherInfo { name: string; nid: number; blockSize: number; ivLength: number; keyLength: number; mode: string }
const cipherInfo: Record<string, CipherInfo | undefined> = {
  "aes-128-cbc": { name: "AES-128-CBC", nid: 419, blockSize: 16, ivLength: 16, keyLength: 16, mode: "cbc" },
  "aes-192-cbc": { name: "AES-192-CBC", nid: 423, blockSize: 16, ivLength: 16, keyLength: 24, mode: "cbc" },
  "aes-256-cbc": { name: "AES-256-CBC", nid: 427, blockSize: 16, ivLength: 16, keyLength: 32, mode: "cbc" },
  "aes-128-ctr": { name: "AES-128-CTR", nid: 904, blockSize: 1, ivLength: 16, keyLength: 16, mode: "ctr" },
  "aes-192-ctr": { name: "AES-192-CTR", nid: 905, blockSize: 1, ivLength: 16, keyLength: 24, mode: "ctr" },
  "aes-256-ctr": { name: "AES-256-CTR", nid: 906, blockSize: 1, ivLength: 16, keyLength: 32, mode: "ctr" },
  "aes-128-gcm": { name: "id-aes128-GCM", nid: 895, blockSize: 1, ivLength: 12, keyLength: 16, mode: "gcm" },
  "aes-192-gcm": { name: "id-aes192-GCM", nid: 898, blockSize: 1, ivLength: 12, keyLength: 24, mode: "gcm" },
  "aes-256-gcm": { name: "id-aes256-GCM", nid: 901, blockSize: 1, ivLength: 12, keyLength: 32, mode: "gcm" },
};

export function getCipherInfo(name: unknown, options: unknown = {}) {
  const info = cipherInfo[String(name).toLowerCase()];
  if (!info) return undefined;
  const keyLength = propertyOf(options, "keyLength") === undefined ? info.keyLength : Number(propertyOf(options, "keyLength"));
  const ivLength = propertyOf(options, "ivLength") === undefined ? info.ivLength : Number(propertyOf(options, "ivLength"));
  if (!Number.isSafeInteger(keyLength) || !Number.isSafeInteger(ivLength)) return undefined;
  return { ...info, keyLength, ivLength };
}

function integerBytes(value: unknown) {
  const number = Number(value);
  if (!Number.isSafeInteger(number) || number < 0) throw new RangeError("Invalid integer value");
  if (number === 0) return Buffer.from([0]);
  const bytes = [];
  for (let current = number; current > 0; current = Math.floor(current / 256)) bytes.unshift(current % 256);
  return Buffer.from(bytes);
}

function diffieHellmanArgs(prime: unknown, primeEncoding: unknown, generator: unknown, generatorEncoding: unknown) {
  if (typeof prime === "number") {
    const actualGenerator = typeof primeEncoding === "number" ? primeEncoding : generator ?? 2;
    validateNumber(actualGenerator, "generator");
    const params = cryptoDhParams({ bits: prime, generator: actualGenerator });
    return { prime: Buffer.from(params.prime), generator: Buffer.from(params.generator) };
  }
  if (typeof primeEncoding === "number" || ArrayBuffer.isView(primeEncoding) || primeEncoding instanceof ArrayBuffer) {
    return { prime: Buffer.from(prime), generator: dhGenerator(primeEncoding, undefined) };
  }
  return { prime: Buffer.from(prime, primeEncoding), generator: dhGenerator(generator, generatorEncoding) };
}

function dhGenerator(generator: unknown, encoding: unknown) {
  if (generator === undefined) return integerBytes(2);
  return typeof generator === "number" ? integerBytes(generator) : Buffer.from(generator, encoding);
}

export function createDiffieHellman(prime: unknown, primeEncoding?: unknown, generator?: unknown, generatorEncoding?: unknown) {
  const args = diffieHellmanArgs(prime, primeEncoding, generator, generatorEncoding);
  return new DiffieHellman(args.prime, args.generator, keyAgreementToken);
}

export function createDiffieHellmanGroup(name: unknown): never {
  throw new Error(`The Diffie-Hellman group ${name} is not available in this runtime`);
}

export function createECDH(curve: unknown) { return new ECDH(curve, keyAgreementToken); }
export function createPrivateKey(input: unknown) {
  if (input instanceof KeyObject) {
    const record = keyRecord(input);
    if (record.type !== "private") throw new TypeError("The key is not private");
    return input;
  }
  const settings = keySettings(input);
  let value = settings.key ?? input;
  let format = settings.format;
  let type: KeyType | undefined = settings.type === "pkcs8" ? "private" : undefined;
  if (format === "jwk") return keyFromJwk(value);
  if (typeof value === "string") {
    const pem = pemDecode(value);
    value = pem.bytes;
    format ??= pem.format;
    type ??= pem.type;
  }
  return createAsymmetricObject(importDer(Buffer.from(value), format ?? "der", type));
}
export function createPublicKey(input: unknown): KeyObject {
  if (input instanceof KeyObject) {
    const record = keyRecord(input);
    if (record.type === "public") return input;
    const bytes = hostKeyExport(record, "spki");
    return createAsymmetricObject({ ...record, type: "public", format: "spki", bytes });
  }
  const settings = keySettings(input);
  let value = settings.key ?? input;
  let format = settings.format;
  let type;
  if (format === "jwk") {
    const key = keyFromJwk(value);
    return keyRecord(key).type === "private" ? createPublicKey(key) : key;
  }
  if (typeof value === "string") {
    const pem = pemDecode(value);
    value = pem.bytes;
    format ??= pem.format;
    type = pem.type;
  }
  if (type === "private") return createPublicKey(createPrivateKey({ key: value, format }));
  return createAsymmetricObject(importDer(Buffer.from(value), format ?? "der", "public"));
}
export function createSecretKey(key: unknown, encoding?: unknown) { return new SecretKeyObject(Buffer.from(key, encoding), keyObjectToken); }
export function createCipheriv(algorithm: unknown, key: unknown, iv: unknown, options?: unknown) { return new Cipheriv(algorithm, key, iv, options); }
export function createDecipheriv(algorithm: unknown, key: unknown, iv: unknown, options?: unknown) { return new Decipheriv(algorithm, key, iv, options); }
function evpBytesToKey(password: unknown, keyLength: number, ivLength: number) {
  const output = [];
  let previous = new Uint8Array();
  while (output.reduce((length, value) => length + value.length, 0) < keyLength + ivLength) {
    previous = new Uint8Array(digest("MD5", Buffer.concat([previous, Buffer.from(password)])));
    output.push(previous);
  }
  const bytes = Buffer.concat(output);
  return { key: bytes.subarray(0, keyLength), iv: bytes.subarray(keyLength, keyLength + ivLength) };
}
export function createCipher(algorithm: unknown, password: unknown) { const spec = cipherSpec(algorithm); const derived = evpBytesToKey(password, spec.keyLength, 16); return createCipheriv(algorithm, derived.key, derived.iv); }
export function createDecipher(algorithm: unknown, password: unknown) { const spec = cipherSpec(algorithm); const derived = evpBytesToKey(password, spec.keyLength, 16); return createDecipheriv(algorithm, derived.key, derived.iv); }
export function createSign(algorithm: unknown) { return new Sign(algorithm, signToken); }
export function createVerify(algorithm: unknown) { return new Verify(algorithm, signToken); }
export function generateKeyPairSync(type: unknown, options: unknown = {}) {
  const kind = type === "rsa" || type === "rsa-pss" || type === "rsa-oaep" ? "rsa" : type === "ec" ? "ec" : type;
  validateString(kind, "type");
  const key = cryptoGenerateKey(keyGeneration(kind, options));
  const publicKey = createAsymmetricObject(asymmetricRecord(key, "public"));
  const privateKey = createAsymmetricObject(asymmetricRecord(key, "private"));
  if (propertyOf(options, "publicKeyEncoding") || propertyOf(options, "privateKeyEncoding")) return { publicKey: encodeKey(keyRecord(publicKey), propertyOf(options, "publicKeyEncoding")), privateKey: encodeKey(keyRecord(privateKey), propertyOf(options, "privateKeyEncoding")) };
  return { publicKey, privateKey };
}
// What the host generates for a key of `kind` with Node's `options`.
function keyGeneration(kind: string, options: unknown): KeyGeneration {
  if (kind === "rsa") {
    const modulusLength = propertyOf(options, "modulusLength") ?? 2048;
    validateNumber(modulusLength, "options.modulusLength");
    return { kind, modulusLength, publicExponent: Buffer.from(propertyOf(options, "publicExponent") ?? new Uint8Array([1, 0, 1])) };
  }
  if (kind !== "ec") return { kind };
  const namedCurve = propertyOf(options, "namedCurve");
  const curve = namedCurve === "prime256v1" ? "P-256" : namedCurve === "secp384r1" ? "P-384" : namedCurve === "secp521r1" ? "P-521" : namedCurve;
  if (curve !== undefined) validateString(curve, "options.namedCurve");
  return { kind, curve };
}
export function generateKeyPair(type: unknown, options: unknown, callback: unknown) {
  const [settings, done] = isFunction(options) ? [{}, options] : [options, callback];
  if (!isFunction(done)) throw new TypeError("The callback argument must be of type function");
  try { const result = generateKeyPairSync(type, settings); queueMicrotask(() => done(null, result.publicKey, result.privateKey)); }
  catch (error) { queueMicrotask(() => done(error)); }
}
export function generateKeySync(type: unknown, options: unknown = {}) {
  if (type !== "aes" && type !== "hmac") throw new Error("Unsupported key type");
  const length = Number(propertyOf(options, "length") ?? (type === "aes" ? 256 : 512));
  if (!Number.isSafeInteger(length) || length < 1 || length % 8 !== 0) throw new RangeError("Invalid key length");
  return createSecretKey(randomBytesSync(Math.ceil(length / 8)));
}
export function generateKey(type: unknown, options: unknown, callback: unknown) {
  const [settings, done] = isFunction(options) ? [{}, options] : [options, callback];
  if (!isFunction(done)) throw new TypeError("The callback argument must be of type function");
  deliver(done, () => generateKeySync(type, settings));
}
function hkdfBytes(hash: unknown, key: Uint8Array, salt: Uint8Array, info: Uint8Array, length: number) {
  try { return cryptoHkdf(normalizeHash(hash), key, salt, info, length); }
  catch (error) {
    if (error instanceof DOMException && error.name === "OperationError") throw new RangeError("Invalid Hkdf key length");
    throw error;
  }
}

export function hkdfSync(hash: unknown, key: unknown, salt: unknown, info: unknown, keylen: unknown) {
  return hkdfBytes(hash, Buffer.from(key), Buffer.from(salt), Buffer.from(info), integerInRange(keylen, "keylen", 0));
}

export function hkdf(hash: unknown, key: unknown, salt: unknown, info: unknown, keylen: unknown, callback: unknown) {
  if (!isFunction(callback)) throw new TypeError("The \"callback\" argument must be of type function");
  deliver(callback, () => hkdfSync(hash, key, salt, info, keylen));
}

export function pbkdf2Sync(password: unknown, salt: unknown, iterations: unknown, keylen: unknown, hash: unknown) {
  return Buffer.from(new Uint8Array(cryptoPbkdf2(normalizeHash(hash), Buffer.from(password), Buffer.from(salt), integerInRange(iterations, "iterations", 1), integerInRange(keylen, "keylen", 0))));
}

export function pbkdf2(password: unknown, salt: unknown, iterations: unknown, keylen: unknown, hash: unknown, callback: unknown) {
  if (!isFunction(callback)) throw new TypeError("The \"callback\" argument must be of type function");
  deliver(callback, () => pbkdf2Sync(password, salt, iterations, keylen, hash));
}

function encryptionSettings(value: unknown) {
  if (value && typeof value === "object" && !(value instanceof KeyObject) && !(value instanceof ArrayBuffer) && !ArrayBuffer.isView(value) && Object.hasOwn(value, "key")) return objectOf(value);
  return { key: value };
}

function rsaKeySettings(options: unknown, isPrivate: boolean) {
  const settings = encryptionSettings(options);
  const record = keyRecord(isPrivate ? createPrivateKey(settings.key) : createPublicKey(settings.key));
  if (record.kind !== "rsa") throw new TypeError("An RSA key is required");
  return { settings, record };
}

function rsaOaep(operation: typeof cryptoEncrypt, options: unknown, buffer: unknown, isPrivate: boolean) {
  const { settings, record } = rsaKeySettings(options, isPrivate);
  const padding = settings.padding;
  if (padding !== undefined && padding !== 4 && padding !== "RSA_PKCS1_OAEP_PADDING") throw new Error("Only RSA-OAEP is supported");
  return Buffer.from(operation(Buffer.from(buffer), { format: record.format, key: record.bytes, hash: normalizeHash(settings.oaepHash ?? "sha1"), label: settings.oaepLabel ? Buffer.from(settings.oaepLabel) : new Uint8Array() }));
}

export function publicEncrypt(options: unknown, buffer: unknown) { return rsaOaep(cryptoEncrypt, options, buffer, false); }
export function privateDecrypt(options: unknown, buffer: unknown) { return rsaOaep(cryptoDecrypt, options, buffer, true); }

function legacyRsaPadding(value: unknown) {
  if (value === undefined || value === "RSA_PKCS1_PADDING") return 1;
  if (value === "RSA_NO_PADDING") return 3;
  if (value === "RSA_PKCS1_OAEP_PADDING") return 4;
  if (typeof value !== "number") throw invalidArgValue("options.padding", value);
  return value;
}

function rsaLegacy(operation: typeof cryptoRsaLegacyPrivateEncrypt, options: unknown, buffer: unknown, isPrivate: boolean) {
  const { settings, record } = rsaKeySettings(options, isPrivate);
  return Buffer.from(operation(Buffer.from(buffer), { kind: "rsa", format: record.format, key: record.bytes, padding: legacyRsaPadding(settings.padding) }));
}

export function privateEncrypt(options: unknown, buffer: unknown) { return rsaLegacy(cryptoRsaLegacyPrivateEncrypt, options, buffer, true); }
export const pseudoRandomBytes = randomBytes;
export function publicDecrypt(options: unknown, buffer: unknown) { return rsaLegacy(cryptoRsaLegacyPublicDecrypt, options, buffer, false); }

function scryptSettings(options: unknown, keylen: unknown) {
  const n = Number(propertyOf(options, "N") ?? 16_384);
  const r = Number(propertyOf(options, "r") ?? 8);
  const p = Number(propertyOf(options, "p") ?? 1);
  const maxmem = Number(propertyOf(options, "maxmem") ?? 32 * 1024 * 1024);
  if (!Number.isSafeInteger(n) || n < 2 || (n & (n - 1)) !== 0) throw new RangeError("Invalid scrypt N parameter");
  if (!Number.isSafeInteger(r) || r < 1) throw new RangeError("Invalid scrypt r parameter");
  if (!Number.isSafeInteger(p) || p < 1) throw new RangeError("Invalid scrypt p parameter");
  if (!Number.isSafeInteger(maxmem) || maxmem < 1) throw new RangeError("Invalid scrypt maxmem parameter");
  return { n, r, p, maxmem, keyLength: integerInRange(keylen, "keylen", 0) };
}

export function scryptSync(password: unknown, salt: unknown, keylen: unknown, options: unknown = {}) {
  const settings = scryptSettings(options, keylen);
  return Buffer.from(cryptoScrypt(Buffer.from(password), { salt: Buffer.from(salt), ...settings }));
}

export function scrypt(password: unknown, salt: unknown, keylen: unknown, options: unknown, callback?: unknown) {
  const [settings, done] = isFunction(options) ? [{}, options] : [options, callback];
  if (!isFunction(done)) throw new TypeError("The \"callback\" argument must be of type function");
  deliver(done, () => scryptSync(password, salt, keylen, settings));
}
export const setEngine = unsupportedCrypto("setEngine");
export const setFips = unsupportedCrypto("setFips");
export function sign(algorithm: unknown, data: unknown, key: unknown, callback?: unknown) {
  return returnOrDeliver(callback, () => createSign(algorithm).update(data).sign(key), typeof callback === "function");
}

export function verify(algorithm: unknown, data: unknown, key: unknown, signature: unknown, callback?: unknown) {
  return returnOrDeliver(callback, () => createVerify(algorithm).update(data).verify(key, signature), typeof callback === "function");
}

export { CryptoKey };

export default {
  Certificate, Cipher, Cipheriv, CryptoKey, Decipher, Decipheriv, DiffieHellman, DiffieHellmanGroup, ECDH, Hash, Hmac,
  KeyObject, PrivateKeyObject, PublicKeyObject, SecretKeyObject, Sign, Verify, X509Certificate, checkPrime, checkPrimeSync,
  constants, createCipher, createCipheriv, createDecipher, createDecipheriv, createDiffieHellman, createDiffieHellmanGroup,
  createECDH, createHash, createHmac, createPrivateKey, createPublicKey, createSecretKey, createSign, createVerify, fips,
  generateKey, generateKeyPair, generateKeyPairSync, generateKeySync, generatePrime, generatePrimeSync, getCipherInfo,
  getCiphers, getCurves, getDiffieHellman, getFips, getHashes, getRandomValues, hash, hkdf, hkdfSync, pbkdf2, pbkdf2Sync,
  privateDecrypt, privateEncrypt, pseudoRandomBytes, publicDecrypt, publicEncrypt, randomBytes, randomFill, randomFillSync,
  randomInt, randomUUID, scrypt, scryptSync, secureHeapUsed, setEngine, setFips, sign, subtle, timingSafeEqual, verify, webcrypto,
};
