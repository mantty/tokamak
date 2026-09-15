import { cryptoAesGcm, cryptoHmac, cryptoHkdf, cryptoPbkdf2, cryptoCheckPrime, cryptoDecrypt, cryptoDhCompute, cryptoDhGenerate, cryptoDhParams, cryptoEcdhCompute, cryptoEcdhConvert, cryptoEcdhPublic, cryptoEncrypt, cryptoExportKey, cryptoGenerateKey, cryptoGeneratePrime, cryptoImportKey, cryptoRsaLegacyPrivateEncrypt, cryptoRsaLegacyPublicDecrypt, cryptoScrypt, cryptoSign, cryptoVerify, digest, randomBytes as hostRandomBytes } from "tokamak:host";
import { CryptoKey, crypto as webcrypto } from "../globals/web.mjs";
import { Transform } from "../streams/node.mjs";
import { Buffer } from "./buffer.mjs";
import { unsupportedFunction } from "./unsupported.mjs";

const unsupportedCrypto = name => unsupportedFunction(`crypto.${name}`);
const hashToken = Symbol("hash");
const keyObjectToken = Symbol("keyObject");

export const subtle = webcrypto.subtle;
export { webcrypto };
export const randomUUID = webcrypto.randomUUID;
export const getRandomValues = webcrypto.getRandomValues;

function normalizeHash(algorithm) {
  const name = String(algorithm).toUpperCase().replaceAll("_", "-");
  return name.startsWith("SHA") && !name.startsWith("SHA-") ? "SHA-" + name.slice(3) : name;
}

function inputBytes(value, encoding) {
  return Buffer.from(value, encoding);
}

function joinChunks(chunks) {
  const size = chunks.reduce((total, chunk) => total + chunk.length, 0);
  const input = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    input.set(chunk, offset);
    offset += chunk.length;
  }
  return input;
}

function finalizedError() {
  const error = new Error("Digest already called");
  error.code = "ERR_CRYPTO_HASH_FINALIZED";
  return error;
}

export class Hash extends Transform {
  constructor(algorithm, token) {
    super();
    if (token !== hashToken) throw new Error("Illegal constructor");
    this.__algorithm = normalizeHash(algorithm);
    this.__chunks = [];
    this.__finalized = false;
  }

  update(value, encoding) {
    if (this.__finalized) throw finalizedError();
    this.__chunks.push(inputBytes(value, encoding));
    return this;
  }

  digest(encoding) {
    if (this.__finalized) throw finalizedError();
    this.__finalized = true;
    const output = Buffer.from(digest(this.__algorithm, joinChunks(this.__chunks)));
    return encoding === undefined ? output : output.toString(encoding);
  }

  copy() {
    if (this.__finalized) throw finalizedError();
    const copy = new Hash(this.__algorithm, hashToken);
    copy.__chunks = this.__chunks.map(chunk => Buffer.from(chunk));
    return copy;
  }

  _transform(chunk, encoding, callback) {
    try { this.update(chunk, encoding); callback(); }
    catch (error) { callback(error); }
  }

  _flush(callback) { callback(); }
}

export function createHash(algorithm) { return new Hash(algorithm, hashToken); }

function validateRandomLength(length) {
  const size = Number(length);
  if (!Number.isSafeInteger(size) || size < 0) {
    const error = new RangeError(`The value of "size" is out of range. Received ${length}`);
    error.code = "ERR_OUT_OF_RANGE";
    throw error;
  }
  return size;
}

// Workers rejects requests above the Web Crypto quota; matching keeps apps portable.
export function randomBytesSync(length) {
  const size = validateRandomLength(length);
  if (size > 65536) {
    throw new DOMException(
      `The requested length exceeds the quota (${size} > 65536)`,
      "QuotaExceededError",
    );
  }
  return Buffer.from(hostRandomBytes(size));
}

export function randomBytes(length, callback) {
  if (callback === undefined) return randomBytesSync(length);
  if (typeof callback !== "function") throw new TypeError("The callback argument must be of type function");
  try {
    const value = randomBytesSync(length);
    queueMicrotask(() => callback(null, value));
  } catch (error) {
    queueMicrotask(() => callback(error));
  }
}

function arrayBufferView(buffer) {
  if (buffer instanceof ArrayBuffer || (typeof SharedArrayBuffer !== "undefined" && buffer instanceof SharedArrayBuffer)) {
    return new Uint8Array(buffer);
  }
  if (ArrayBuffer.isView(buffer)) return new Uint8Array(buffer.buffer, buffer.byteOffset, buffer.byteLength);
  throw new TypeError("The first argument must be an ArrayBuffer or a view on an ArrayBuffer");
}

export function randomFillSync(buffer, offset = 0, size) {
  const view = arrayBufferView(buffer);
  const start = Number(offset);
  const length = size === undefined ? view.byteLength - start : Number(size);
  if (!Number.isSafeInteger(start) || start < 0 || !Number.isSafeInteger(length) || length < 0 || start + length > view.byteLength) {
    const error = new RangeError("The value of \"offset\" is out of range");
    error.code = "ERR_OUT_OF_RANGE";
    throw error;
  }
  webcrypto.getRandomValues(view.subarray(start, start + length));
  return buffer;
}

export function randomFill(buffer, offset, size, callback) {
  if (typeof offset === "function") {
    callback = offset;
    offset = 0;
    size = undefined;
  } else if (typeof size === "function") {
    callback = size;
    size = undefined;
  }
  if (typeof callback !== "function") {
    const error = new TypeError("The callback argument must be of type function");
    error.code = "ERR_INVALID_ARG_TYPE";
    throw error;
  }
  try {
    const value = randomFillSync(buffer, offset, size);
    queueMicrotask(() => callback(null, value));
  } catch (error) {
    queueMicrotask(() => callback(error));
  }
}

export function randomInt(min, max, callback) {
  if (typeof max === "function") {
    callback = max;
    max = min;
    min = 0;
  }
  min = Number(min);
  max = Number(max);
  if (!Number.isSafeInteger(min) || !Number.isSafeInteger(max) || max <= min) {
    const error = new RangeError("Invalid randomInt range");
    error.code = "ERR_OUT_OF_RANGE";
    throw error;
  }
  const range = max - min;
  const randomSpace = 2 ** 53;
  if (range > randomSpace) throw new RangeError("randomInt range is too large");
  const limit = randomSpace - (randomSpace % range);
  let value;
  do {
    const words = webcrypto.getRandomValues(new Uint32Array(2));
    value = (words[0] & 0x1fffff) * 2 ** 32 + words[1];
  } while (value >= limit);
  const result = min + (value % range);
  if (callback === undefined) return result;
  if (typeof callback !== "function") throw new TypeError("The callback argument must be of type function");
  queueMicrotask(() => callback(null, result));
}

export function timingSafeEqual(left, right) {
  if (left.byteLength !== right.byteLength) throw new TypeError("Input buffers must have the same byte length");
  const a = new Uint8Array(left.buffer ?? left, left.byteOffset ?? 0, left.byteLength);
  const b = new Uint8Array(right.buffer ?? right, right.byteOffset ?? 0, right.byteLength);
  let result = 0;
  for (let index = 0; index < a.length; index += 1) result |= a[index] ^ b[index];
  return result === 0;
}

export function createHmac(algorithm, key) { return new Hmac(algorithm, key, hashToken); }

export class Hmac extends Transform {
  constructor(algorithm, key, token) {
    super();
    if (token !== hashToken) throw new Error("Illegal constructor");
    this.__algorithm = normalizeHash(algorithm);
    this.__key = inputBytes(key);
    this.__chunks = [];
    this.__finalized = false;
  }

  update(value, encoding) {
    if (this.__finalized) throw finalizedError();
    this.__chunks.push(inputBytes(value, encoding));
    return this;
  }

  digest(encoding) {
    if (this.__finalized) throw finalizedError();
    this.__finalized = true;
    const output = Buffer.from(cryptoHmac(this.__algorithm, this.__key, joinChunks(this.__chunks)));
    return encoding === undefined ? output : output.toString(encoding);
  }

  _transform(chunk, encoding, callback) {
    try { this.update(chunk, encoding); callback(); }
    catch (error) { callback(error); }
  }

  _flush(callback) { callback(); }
}

export function hash(algorithm, data, options) {
  if (options !== undefined) {
    const error = new TypeError("The \"options\" argument must be of type string or an instance of Buffer or Uint8Array");
    error.code = "ERR_INVALID_ARG_TYPE";
    throw error;
  }
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
export function getDiffieHellman(name) { return createDiffieHellmanGroup(name); }
export function secureHeapUsed() { return { total: 0, used: 0, utilization: 0, min: 0 }; }

function illegalConstructor() { throw new Error("Illegal constructor"); }

export const Certificate = class Certificate { constructor() { illegalConstructor(); } };
const keyAgreementToken = Symbol("keyAgreement");
export class DiffieHellman {
  constructor(prime, generator, token) {
    if (token !== keyAgreementToken) illegalConstructor();
    this.__prime = Buffer.from(prime);
    this.__generator = Buffer.from(generator);
    this.__privateKey = null;
    this.__publicKey = null;
  }

  computeSecret(otherPublicKey, inputEncoding, outputEncodingName) {
    if (!this.__privateKey) throw new Error("Private key is not set");
    const output = cryptoDhCompute({
      prime: this.__prime,
      generator: this.__generator,
      private: this.__privateKey,
      peer: inputBytes(otherPublicKey, inputEncoding),
    });
    return outputEncoding(Buffer.from(output), outputEncodingName);
  }

  generateKeys(encoding) {
    const state = parseBundle(cryptoDhGenerate({ prime: this.__prime, generator: this.__generator }));
    this.__publicKey = Buffer.from(state.public);
    this.__privateKey = Buffer.from(state.private);
    return outputEncoding(this.getPublicKey(), encoding);
  }

  getGenerator(encoding) { return outputEncoding(Buffer.from(this.__generator), encoding); }
  getPrime(encoding) { return outputEncoding(Buffer.from(this.__prime), encoding); }
  getPrivateKey(encoding) {
    if (!this.__privateKey) throw new Error("Private key is not set");
    return outputEncoding(Buffer.from(this.__privateKey), encoding);
  }
  getPublicKey(encoding) {
    if (!this.__publicKey) throw new Error("Public key is not set");
    return outputEncoding(Buffer.from(this.__publicKey), encoding);
  }
  setPrivateKey(privateKey, encoding) {
    const value = inputBytes(privateKey, encoding);
    const state = parseBundle(cryptoDhGenerate({ prime: this.__prime, generator: this.__generator, private: value }));
    this.__privateKey = Buffer.from(state.private);
    this.__publicKey = Buffer.from(state.public);
    return this;
  }
  setPublicKey(publicKey, encoding) {
    this.__publicKey = Buffer.from(inputBytes(publicKey, encoding));
    return this;
  }
  get verifyError() { return 0; }
}

export class DiffieHellmanGroup extends DiffieHellman {}

export class ECDH {
  constructor(curve, token) {
    if (token !== keyAgreementToken) illegalConstructor();
    this.__curve = nodeCurveName(curve);
    this.__privateKey = null;
    this.__publicKey = null;
  }

  computeSecret(otherPublicKey, inputEncoding, outputEncodingName) {
    if (!this.__privateKey) throw new Error("Private key is not set");
    const output = cryptoEcdhCompute({
      curve: this.__curve,
      private: this.__privateKey,
      peer: inputBytes(otherPublicKey, inputEncoding),
    });
    return outputEncoding(Buffer.from(output), outputEncodingName);
  }

  generateKeys(encoding, format = "uncompressed") {
    const keys = generateKeyPairSync("ec", { namedCurve: webCurveName(this.__curve) });
    const privateJwk = keyRecord(keys.privateKey).jwk;
    const publicJwk = keyRecord(keys.publicKey).jwk;
    this.__privateKey = Buffer.from(privateJwk.d, "base64url");
    this.__publicKey = publicPoint(publicJwk);
    return this.getPublicKey(encoding, format);
  }

  getPrivateKey(encoding) {
    if (!this.__privateKey) throw new Error("Private key is not set");
    return outputEncoding(Buffer.from(this.__privateKey), encoding);
  }

  getPublicKey(encoding, format = "uncompressed") {
    if (!this.__publicKey) throw new Error("Public key is not set");
    const value = format === "uncompressed" ? this.__publicKey : Buffer.from(cryptoEcdhConvert(this.__publicKey, { curve: this.__curve, format }));
    return outputEncoding(value, encoding);
  }

  setPrivateKey(privateKey, encoding) {
    this.__privateKey = Buffer.from(inputBytes(privateKey, encoding));
    this.__publicKey = Buffer.from(cryptoEcdhPublic({ curve: this.__curve, private: this.__privateKey, format: "uncompressed" }));
    return this;
  }

  setPublicKey(publicKey, encoding) {
    this.__publicKey = Buffer.from(cryptoEcdhConvert(inputBytes(publicKey, encoding), { curve: this.__curve, format: "uncompressed" }));
    return this;
  }

  static convertKey(key, curve, inputEncoding, outputEncodingName, format = "uncompressed") {
    const value = Buffer.from(cryptoEcdhConvert(inputBytes(key, inputEncoding), { curve: nodeCurveName(curve), format }));
    return outputEncoding(value, outputEncodingName);
  }
}

function nodeCurveName(curve) {
  const name = String(curve);
  if (name === "prime256v1" || name === "P-256") return "prime256v1";
  if (name === "secp384r1" || name === "P-384") return "secp384r1";
  if (name === "secp521r1" || name === "P-521") return "secp521r1";
  throw new Error(`Invalid EC curve name: ${curve}`);
}

function webCurveName(curve) {
  return { "prime256v1": "P-256", "secp384r1": "P-384", "secp521r1": "P-521" }[nodeCurveName(curve)];
}

function publicPoint(jwk) {
  const x = Buffer.from(jwk.x, "base64url");
  const y = Buffer.from(jwk.y, "base64url");
  return Buffer.concat([Buffer.from([4]), x, y]);
}

const keyObjectState = new WeakMap();
const cipherToken = Symbol("cipher");
const signToken = Symbol("sign");

function parseBundle(value) {
  const bytes = Buffer.from(value);
  if (bytes.length < 9 || bytes[0] !== 1) throw new TypeError("Invalid native key bundle");
  const publicLength = new DataView(bytes.buffer, bytes.byteOffset + 1, 4).getUint32(0);
  const privateLength = new DataView(bytes.buffer, bytes.byteOffset + 5, 4).getUint32(0);
  if (9 + publicLength + privateLength !== bytes.length) throw new TypeError("Invalid native key bundle");
  return { public: Uint8Array.from(bytes.subarray(9, 9 + publicLength)), private: privateLength ? Uint8Array.from(bytes.subarray(9 + publicLength)) : null };
}

function pemDecode(value) {
  const match = String(value).match(/-----BEGIN ([^-]+)-----([\s\S]*?)-----END \1-----/);
  if (!match) throw new TypeError("Invalid PEM formatted message");
  const label = match[1];
  return {
    bytes: Buffer.from(match[2].replace(/\s/g, ""), "base64"),
    format: label === "PRIVATE KEY" || label === "PUBLIC KEY" ? (label === "PRIVATE KEY" ? "pkcs8" : "spki") : "der",
    type: label.includes("PRIVATE") ? "private" : "public",
  };
}

function pemEncode(bytes, label) {
  const encoded = Buffer.from(bytes).toString("base64");
  const lines = encoded.match(/.{1,64}/g)?.join("\n") ?? "";
  return `-----BEGIN ${label}-----\n${lines}\n-----END ${label}-----\n`;
}

function keyKind(jwk) {
  if (jwk?.kty === "RSA") return "rsa";
  if (jwk?.kty === "EC") return "ec";
  if (jwk?.kty === "OKP" && jwk.crv === "Ed25519") return "ed25519";
  if (jwk?.kty === "OKP" && jwk.crv === "X25519") return "x25519";
  throw new TypeError("Unsupported key type");
}

function keyFromJwk(jwk) {
  const kind = keyKind(jwk);
  const bundle = parseBundle(cryptoImportKey(new Uint8Array(), { format: "jwk", kind, curve: jwk.crv, jwk: JSON.stringify(jwk) }));
  const privateKey = jwk.d !== undefined;
  return createAsymmetricObject({ type: privateKey ? "private" : "public", kind, format: privateKey ? "pkcs8" : "spki", bytes: privateKey ? bundle.private : bundle.public, jwk });
}

function jwkDetails(jwk) {
  const kind = keyKind(jwk);
  if (kind === "rsa") {
    const modulus = Buffer.from(jwk.n, "base64url");
    let bits = Math.max(0, (modulus.length - 1) * 8);
    let first = modulus[0] ?? 0;
    while (first > 0) { bits += 1; first >>>= 1; }
    const exponent = Buffer.from(jwk.e, "base64url");
    let value = 0n;
    for (const byte of exponent) value = (value << 8n) | BigInt(byte);
    return { modulusLength: bits, publicExponent: value };
  }
  if (kind === "ec") return { namedCurve: jwk.crv };
  return undefined;
}

function describeKey(bytes, format, typeHint) {
  const formats = format === "der" ? ["pkcs8", "spki", "der"] : [format];
  const types = typeHint ? [typeHint] : ["private", "public"];
  const candidates = ["rsa", "ec", "ed25519", "x25519"];
  for (const type of types) {
    for (const keyFormat of formats) {
      for (const kind of candidates) {
        try {
          const jwk = JSON.parse(new TextDecoder().decode(cryptoExportKey(new Uint8Array(), { format: "jwk", keyFormat, kind, key: bytes })));
          const privateKey = jwk.d !== undefined;
          if ((type === "private") !== privateKey) continue;
          return { kind, format: keyFormat, type: privateKey ? "private" : "public", bytes: Buffer.from(bytes), jwk };
        } catch {}
      }
    }
  }
  throw new TypeError("Invalid key material");
}

function keyRecord(key) {
  const record = keyObjectState.get(key);
  if (!record) throw new TypeError("The key argument must be a KeyObject");
  return record;
}

function createAsymmetricObject(record) {
  const Constructor = record.type === "private" ? PrivateKeyObject : PublicKeyObject;
  const key = new Constructor(keyObjectToken);
  keyObjectState.set(key, record);
  return key;
}

function hostKeyExport(record, format) {
  return Buffer.from(cryptoExportKey(new Uint8Array(), { format, keyFormat: record.format, kind: record.kind, key: Uint8Array.from(record.bytes) }));
}

function encodeKey(record, options) {
  if (record.type === "secret") return Buffer.from(record.bytes);
  const settings = options && typeof options === "object" ? options : {};
  const format = settings.format ?? "pem";
  if (format === "jwk") {
    return JSON.parse(new TextDecoder().decode(hostKeyExport(record, "jwk")));
  }
  const type = settings.type ?? (record.type === "private" ? "pkcs8" : "spki");
  const der = hostKeyExport(record, type === "pkcs1" ? "pkcs1" : type);
  if (format === "der") return der;
  if (format === "pem") {
    const label = type === "pkcs8" ? "PRIVATE KEY" : type === "spki" ? "PUBLIC KEY" : record.type === "private" ? "RSA PRIVATE KEY" : "RSA PUBLIC KEY";
    return pemEncode(der, label);
  }
  throw new TypeError("The \"format\" argument must be one of: \"der\", \"pem\", or \"jwk\"");
}

export class KeyObject {
  constructor(token) {
    if (token !== keyObjectToken) illegalConstructor();
  }
  get type() { return keyObjectState.get(this)?.type; }
  get asymmetricKeyType() { return keyObjectState.get(this)?.kind; }
  get asymmetricKeyDetails() { return jwkDetails(keyObjectState.get(this)?.jwk); }
  get symmetricKeySize() { return keyObjectState.get(this)?.type === "secret" ? keyObjectState.get(this).bytes.byteLength : undefined; }
  export(options) { return encodeKey(keyRecord(this), options); }
  equals(other) {
    if (!(other instanceof KeyObject)) throw new TypeError("The \"otherKeyObject\" argument must be an instance of KeyObject");
    const left = keyRecord(this).bytes;
    const right = keyRecord(other).bytes;
    return left.byteLength === right.byteLength && left.every((value, index) => value === right[index]);
  }
}
export class PrivateKeyObject extends KeyObject {}
export class PublicKeyObject extends KeyObject {}
export class SecretKeyObject extends KeyObject {
  constructor(bytes, token) {
    super(token);
    keyObjectState.set(this, { type: "secret", kind: "secret", format: "raw", bytes: Buffer.from(bytes) });
  }
}
export class Sign extends Transform {
  constructor(algorithm, token) {
    super();
    if (token !== signToken) illegalConstructor();
    this.__algorithm = normalizeHash(algorithm);
    this.__chunks = [];
    this.__finalized = false;
  }
  update(value, encoding) {
    if (this.__finalized) throw finalizedError();
    this.__chunks.push(inputBytes(value, encoding));
    return this;
  }
  sign(key, options, callback) {
    if (typeof options === "function") { callback = options; options = undefined; }
    try {
      const record = keyRecord(key instanceof KeyObject ? key : createPrivateKey(key));
      if (record.type !== "private") throw new TypeError("A private key is required");
      const kind = signingKind(record, options);
      const output = Buffer.from(cryptoSign(joinChunks(this.__chunks), { kind, format: record.format, key: record.bytes, hash: this.__algorithm, saltLength: options?.saltLength }));
      this.__finalized = true;
      const value = outputEncoding(output, typeof options === "string" ? options : undefined);
      if (callback) queueMicrotask(() => callback(null, value));
      return callback ? undefined : value;
    } catch (error) {
      if (callback) { queueMicrotask(() => callback(error)); return undefined; }
      throw error;
    }
  }
  _transform(chunk, encoding, callback) { try { this.update(chunk, encoding); callback(); } catch (error) { callback(error); } }
  _flush(callback) { callback(); }
}
export class Verify extends Transform {
  constructor(algorithm, token) {
    super();
    if (token !== signToken) illegalConstructor();
    this.__algorithm = normalizeHash(algorithm);
    this.__chunks = [];
    this.__finalized = false;
  }
  update(value, encoding) {
    if (this.__finalized) throw finalizedError();
    this.__chunks.push(inputBytes(value, encoding));
    return this;
  }
  verify(key, signature, callback) {
    if (typeof signature === "function") { callback = signature; signature = key; key = undefined; }
    try {
      const record = keyRecord(key instanceof KeyObject ? key : createPublicKey(key));
      const kind = signingKind(record);
      const value = cryptoVerify(inputBytes(signature), { kind, format: record.format, key: record.bytes, hash: this.__algorithm, message: joinChunks(this.__chunks), saltLength: undefined });
      this.__finalized = true;
      if (callback) queueMicrotask(() => callback(null, value));
      return callback ? undefined : value;
    } catch (error) {
      if (callback) { queueMicrotask(() => callback(error)); return undefined; }
      throw error;
    }
  }
  _transform(chunk, encoding, callback) { try { this.update(chunk, encoding); callback(); } catch (error) { callback(error); } }
  _flush(callback) { callback(); }
}
export const X509Certificate = class X509Certificate { constructor() { illegalConstructor(); } };

function outputEncoding(value, encoding) { return encoding === undefined ? value : value.toString(encoding); }

function signingKind(record, options = {}) {
  if (record.kind === "rsa" && (options.padding === 6 || options.padding === "RSA_PKCS1_PSS_PADDING")) return "rsa-pss";
  return record.kind === "ec" ? "ecdsa" : record.kind;
}

function cipherInput(value, encoding) {
  if (typeof value === "string" && encoding === undefined) {
    const error = new TypeError("The argument 'inputEncoding' If inputEncoding is not provided then the data must be a Buffer. Received undefined");
    error.code = "ERR_INVALID_ARG_VALUE";
    throw error;
  }
  return inputBytes(value, encoding);
}

function cipherSpec(algorithm) {
  const match = String(algorithm).toLowerCase().match(/^aes-(128|192|256)-(gcm|cbc|ctr)$/);
  if (!match) throw new Error(`Unknown cipher: ${algorithm}`);
  return { name: `AES-${match[2].toUpperCase()}`, keyLength: Number(match[1]) / 8, mode: match[2] };
}

class CipherBase extends Transform {
  constructor(algorithm, key, iv, decrypt, token) {
    super();
    if (token !== cipherToken) illegalConstructor();
    const spec = cipherSpec(algorithm);
    const keyBytes = key instanceof KeyObject ? keyRecord(key).bytes : inputBytes(key);
    if (keyBytes.length !== spec.keyLength) throw new RangeError("Invalid key length");
    this.__state = { algorithm, spec, mode: spec.mode, key: Buffer.from(keyBytes), iv: Buffer.from(iv ?? []), aad: Buffer.alloc(0), authTag: null, chunks: [], tagLength: 16, decrypt, finalized: false };
    if (spec.mode === "cbc" && this.__state.iv.length !== 16) throw new TypeError("Invalid initialization vector");
    if (spec.mode === "ctr" && this.__state.iv.length !== 16) throw new TypeError("Invalid initialization vector");
  }
  update(value, inputEncoding, outputEncodingName) {
    const state = this.__state;
    if (state.finalized) throw finalizedError();
    state.chunks.push(cipherInput(value, inputEncoding));
    return outputEncoding(Buffer.alloc(0), outputEncodingName);
  }
  final(outputEncodingName) {
    const state = this.__state;
    if (state.finalized) throw finalizedError();
    state.finalized = true;
    let input = joinChunks(state.chunks);
    if (state.mode === "gcm" && state.decrypt) input = Buffer.concat([input, state.authTag ?? Buffer.alloc(0)]);
    const value = Buffer.from(cryptoAesGcm(!state.decrypt, state.key, state.iv, input, { tagLength: state.tagLength * 8, mode: state.spec.name, additionalData: state.aad }));
    if (state.mode === "gcm" && !state.decrypt) {
      state.authTag = Buffer.from(value.subarray(-state.tagLength));
      return outputEncoding(Buffer.from(value.subarray(0, -state.tagLength)), outputEncodingName);
    }
    return outputEncoding(value, outputEncodingName);
  }
  setAAD(value) { this.__state.aad = inputBytes(value); return this; }
  setAutoPadding(value = true) { this.__state.autoPadding = Boolean(value); return this; }
  getAuthTag() {
    const state = this.__state;
    if (!state.finalized || state.decrypt || !state.authTag) throw new Error("Invalid state for operation");
    return Buffer.from(state.authTag);
  }
  _transform(chunk, encoding, callback) { try { callback(null, this.update(chunk, encoding)); } catch (error) { callback(error); } }
  _flush(callback) { try { callback(null, this.final()); } catch (error) { callback(error); } }
}

export class Cipheriv extends CipherBase {
  constructor(algorithm, key, iv, token) { super(algorithm, key, iv, false, token); }
}

export class Decipheriv extends CipherBase {
  constructor(algorithm, key, iv, token) { super(algorithm, key, iv, true, token); }
  setAuthTag(value) { this.__state.authTag = inputBytes(value); this.__state.tagLength = this.__state.authTag.length; return this; }
}

export class Cipher extends Cipheriv {}
export class Decipher extends Decipheriv {}

function primeChecks(options) {
  const checks = Number(options?.checks ?? 0);
  if (!Number.isSafeInteger(checks) || checks < 0) {
    const error = new RangeError(`The value of "checks" is out of range. Received ${options?.checks}`);
    error.code = "ERR_OUT_OF_RANGE";
    throw error;
  }
  return checks;
}

export function checkPrimeSync(candidate, options = {}) {
  return cryptoCheckPrime(inputBytes(candidate), { checks: primeChecks(options) });
}

export function checkPrime(candidate, options, callback) {
  if (typeof options === "function") { callback = options; options = {}; }
  if (typeof callback !== "function") throw new TypeError("The \"callback\" argument must be of type function");
  try {
    const value = checkPrimeSync(candidate, options);
    queueMicrotask(() => callback(null, value));
  } catch (error) { queueMicrotask(() => callback(error)); }
}

function primeSize(size) {
  const bits = Number(size);
  if (!Number.isSafeInteger(bits) || bits < 2) {
    const error = new RangeError(`The value of "size" is out of range. Received ${size}`);
    error.code = "ERR_OUT_OF_RANGE";
    throw error;
  }
  return bits;
}

function primeBytes(size, options = {}) {
  const bits = primeSize(size);
  const value = cryptoGeneratePrime({
    bits,
    safe: Boolean(options.safe),
    add: options.add === undefined ? undefined : inputBytes(options.add),
    rem: options.rem === undefined ? undefined : inputBytes(options.rem),
  });
  return Buffer.from(value);
}

export function generatePrimeSync(size, options = {}) {
  const value = primeBytes(size, options);
  if (options.bigint) {
    let result = 0n;
    for (const byte of value) result = (result << 8n) | BigInt(byte);
    return result;
  }
  return Uint8Array.from(value).buffer;
}

export function generatePrime(size, options, callback) {
  if (typeof options === "function") { callback = options; options = {}; }
  if (typeof callback !== "function") throw new TypeError("The \"callback\" argument must be of type function");
  try {
    const value = generatePrimeSync(size, options);
    queueMicrotask(() => callback(null, value));
  } catch (error) { queueMicrotask(() => callback(error)); }
}

const cipherInfo = {
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

export function getCipherInfo(name, options = {}) {
  const info = cipherInfo[String(name).toLowerCase()];
  if (!info) return undefined;
  const keyLength = options.keyLength === undefined ? info.keyLength : Number(options.keyLength);
  const ivLength = options.ivLength === undefined ? info.ivLength : Number(options.ivLength);
  if (!Number.isSafeInteger(keyLength) || !Number.isSafeInteger(ivLength)) return undefined;
  return { ...info, keyLength, ivLength };
}

function integerBytes(value) {
  const number = Number(value);
  if (!Number.isSafeInteger(number) || number < 0) throw new RangeError("Invalid integer value");
  if (number === 0) return Buffer.from([0]);
  const bytes = [];
  for (let current = number; current > 0; current = Math.floor(current / 256)) bytes.unshift(current % 256);
  return Buffer.from(bytes);
}

function diffieHellmanArgs(prime, primeEncoding, generator, generatorEncoding) {
  if (typeof prime === "number") {
    const actualGenerator = typeof primeEncoding === "number" ? primeEncoding : generator ?? 2;
    const params = parseBundle(cryptoDhParams({ bits: prime, generator: actualGenerator }));
    return { prime: Buffer.from(params.public), generator: Buffer.from(params.private) };
  }
  if (typeof primeEncoding === "number" || ArrayBuffer.isView(primeEncoding) || primeEncoding instanceof ArrayBuffer) {
    generatorEncoding = undefined;
    generator = primeEncoding;
    primeEncoding = undefined;
  }
  return {
    prime: Buffer.from(inputBytes(prime, primeEncoding)),
    generator: generator === undefined
      ? integerBytes(2)
      : typeof generator === "number" ? integerBytes(generator) : Buffer.from(inputBytes(generator, generatorEncoding)),
  };
}

export function createDiffieHellman(prime, primeEncoding, generator, generatorEncoding) {
  const args = diffieHellmanArgs(prime, primeEncoding, generator, generatorEncoding);
  return new DiffieHellman(args.prime, args.generator, keyAgreementToken);
}

export function createDiffieHellmanGroup(name) {
  throw new Error(`The Diffie-Hellman group ${name} is not available in this runtime`);
}

export function createECDH(curve) { return new ECDH(curve, keyAgreementToken); }
export function createPrivateKey(input) {
  if (input instanceof KeyObject) {
    const record = keyRecord(input);
    if (record.type !== "private") throw new TypeError("The key is not private");
    return input;
  }
  const settings = input && typeof input === "object" && !(input instanceof ArrayBuffer) && !ArrayBuffer.isView(input) ? input : {};
  let value = settings.key ?? input;
  let format = settings.format;
  let type = settings.type === "pkcs8" ? "private" : undefined;
  if (format === "jwk") return keyFromJwk(value);
  if (typeof value === "string") {
    const pem = pemDecode(value);
    value = pem.bytes;
    format ??= pem.format;
    type ??= pem.type;
  }
  format ??= "der";
  const bytes = Buffer.from(value);
  const description = describeKey(bytes, format, type);
  return createAsymmetricObject(description);
}
export function createPublicKey(input) {
  if (input instanceof KeyObject) {
    const record = keyRecord(input);
    if (record.type === "public") return input;
    const bytes = hostKeyExport(record, "spki");
    return createAsymmetricObject({ ...record, type: "public", format: "spki", bytes, jwk: { ...record.jwk, d: undefined } });
  }
  const settings = input && typeof input === "object" && !(input instanceof ArrayBuffer) && !ArrayBuffer.isView(input) ? input : {};
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
  format ??= "der";
  return createAsymmetricObject(describeKey(Buffer.from(value), format, "public"));
}
export function createSecretKey(key, encoding) { return new SecretKeyObject(inputBytes(key, encoding), keyObjectToken); }
export function createCipheriv(algorithm, key, iv) { return new Cipheriv(algorithm, key, iv, cipherToken); }
export function createDecipheriv(algorithm, key, iv) { return new Decipheriv(algorithm, key, iv, cipherToken); }
function evpBytesToKey(password, keyLength, ivLength) {
  const output = [];
  let previous = new Uint8Array();
  while (output.reduce((length, value) => length + value.length, 0) < keyLength + ivLength) {
    previous = new Uint8Array(digest("MD5", Buffer.concat([previous, inputBytes(password)])));
    output.push(previous);
  }
  const bytes = Buffer.concat(output);
  return { key: bytes.subarray(0, keyLength), iv: bytes.subarray(keyLength, keyLength + ivLength) };
}
export function createCipher(algorithm, password) { const spec = cipherSpec(algorithm); const derived = evpBytesToKey(password, spec.keyLength, 16); return createCipheriv(algorithm, derived.key, derived.iv); }
export function createDecipher(algorithm, password) { const spec = cipherSpec(algorithm); const derived = evpBytesToKey(password, spec.keyLength, 16); return createDecipheriv(algorithm, derived.key, derived.iv); }
export function createSign(algorithm) { return new Sign(algorithm, signToken); }
export function createVerify(algorithm) { return new Verify(algorithm, signToken); }
export function generateKeyPairSync(type, options = {}) {
  const kind = type === "rsa" || type === "rsa-pss" || type === "rsa-oaep" ? "rsa" : type === "ec" ? "ec" : type;
  const generation = { kind };
  if (kind === "rsa") {
    generation.modulusLength = options.modulusLength ?? 2048;
    generation.publicExponent = inputBytes(options.publicExponent ?? new Uint8Array([1, 0, 1]));
  }
  if (kind === "ec") generation.curve = options.namedCurve === "prime256v1" ? "P-256" : options.namedCurve === "secp384r1" ? "P-384" : options.namedCurve === "secp521r1" ? "P-521" : options.namedCurve;
  const keys = parseBundle(cryptoGenerateKey(generation));
  const publicJwk = JSON.parse(new TextDecoder().decode(cryptoExportKey(new Uint8Array(), { format: "jwk", keyFormat: "spki", kind, key: keys.public })));
  const privateJwk = JSON.parse(new TextDecoder().decode(cryptoExportKey(new Uint8Array(), { format: "jwk", keyFormat: "pkcs8", kind, key: keys.private })));
  const publicKey = createAsymmetricObject({ type: "public", kind, format: "spki", bytes: keys.public, jwk: publicJwk });
  const privateKey = createAsymmetricObject({ type: "private", kind, format: "pkcs8", bytes: keys.private, jwk: privateJwk });
  if (options.publicKeyEncoding || options.privateKeyEncoding) return { publicKey: encodeKey(keyRecord(publicKey), options.publicKeyEncoding), privateKey: encodeKey(keyRecord(privateKey), options.privateKeyEncoding) };
  return { publicKey, privateKey };
}
export function generateKeyPair(type, options, callback) {
  if (typeof options === "function") { callback = options; options = {}; }
  if (typeof callback !== "function") throw new TypeError("The callback argument must be of type function");
  try { const result = generateKeyPairSync(type, options); queueMicrotask(() => callback(null, result.publicKey, result.privateKey)); }
  catch (error) { queueMicrotask(() => callback(error)); }
}
export function generateKeySync(type, options = {}) {
  if (type !== "aes" && type !== "hmac") throw new Error("Unsupported key type");
  const length = Number(options.length ?? (type === "aes" ? 256 : 512));
  if (!Number.isSafeInteger(length) || length < 1 || length % 8 !== 0) throw new RangeError("Invalid key length");
  return createSecretKey(randomBytesSync(Math.ceil(length / 8)));
}
export function generateKey(type, options, callback) {
  if (typeof options === "function") { callback = options; options = {}; }
  if (typeof callback !== "function") throw new TypeError("The callback argument must be of type function");
  try { const key = generateKeySync(type, options); queueMicrotask(() => callback(null, key)); }
  catch (error) { queueMicrotask(() => callback(error)); }
}
function hashByteLength(hash) {
  return { "SHA-1": 20, "SHA-256": 32, "SHA-384": 48, "SHA-512": 64 }[normalizeHash(hash)];
}

function derivationLength(value) {
  const length = Number(value);
  if (!Number.isSafeInteger(length) || length < 0) {
    const error = new RangeError(`The value of "keylen" is out of range. Received ${value}`);
    error.code = "ERR_OUT_OF_RANGE";
    throw error;
  }
  return length;
}

function hkdfBytes(hash, key, salt, info, length) {
  const normalized = normalizeHash(hash);
  const size = hashByteLength(normalized);
  if (size && length > size * 255) throw new RangeError("Invalid key length");
  return cryptoHkdf(normalized, key, salt, info, length);
}

export function hkdfSync(hash, key, salt, info, keylen) {
  return hkdfBytes(hash, inputBytes(key), inputBytes(salt), inputBytes(info), derivationLength(keylen));
}

export function hkdf(hash, key, salt, info, keylen, callback) {
  if (typeof callback !== "function") throw new TypeError("The \"callback\" argument must be of type function");
  try {
    const value = hkdfSync(hash, key, salt, info, keylen);
    queueMicrotask(() => callback(null, value));
  } catch (error) { queueMicrotask(() => callback(error)); }
}

function derivationIterations(value) {
  const iterations = Number(value);
  if (!Number.isSafeInteger(iterations) || iterations < 1) {
    const error = new RangeError(`The value of "iterations" is out of range. Received ${value}`);
    error.code = "ERR_OUT_OF_RANGE";
    throw error;
  }
  return iterations;
}

function pbkdf2Bytes(hash, password, salt, iterations, length) {
  return new Uint8Array(cryptoPbkdf2(hash, password, salt, iterations, length));
}

export function pbkdf2Sync(password, salt, iterations, keylen, hash) {
  return Buffer.from(pbkdf2Bytes(normalizeHash(hash), inputBytes(password), inputBytes(salt), derivationIterations(iterations), derivationLength(keylen)));
}

export function pbkdf2(password, salt, iterations, keylen, hash, callback) {
  if (typeof callback !== "function") throw new TypeError("The \"callback\" argument must be of type function");
  try {
    const value = pbkdf2Sync(password, salt, iterations, keylen, hash);
    queueMicrotask(() => callback(null, value));
  } catch (error) { queueMicrotask(() => callback(error)); }
}

function encryptionSettings(value) {
  if (value && typeof value === "object" && !(value instanceof KeyObject) && !(value instanceof ArrayBuffer) && !ArrayBuffer.isView(value) && Object.hasOwn(value, "key")) return value;
  return { key: value };
}

function rsaOperation(value, decrypt) {
  const settings = encryptionSettings(value);
  const key = decrypt ? createPrivateKey(settings.key) : createPublicKey(settings.key);
  const record = keyRecord(key);
  if (record.kind !== "rsa") throw new TypeError("An RSA key is required");
  const padding = settings.padding;
  if (padding !== undefined && padding !== 4 && padding !== "RSA_PKCS1_OAEP_PADDING") throw new Error("Only RSA-OAEP is supported");
  return { settings, record };
}

export function publicEncrypt(options, buffer) {
  const { settings, record } = rsaOperation(options, false);
  return Buffer.from(cryptoEncrypt(inputBytes(buffer), { kind: "rsa-oaep", format: record.format, key: record.bytes, hash: normalizeHash(settings.oaepHash ?? "sha1"), label: settings.oaepLabel ? inputBytes(settings.oaepLabel) : new Uint8Array() }));
}

export function privateDecrypt(options, buffer) {
  const { settings, record } = rsaOperation(options, true);
  return Buffer.from(cryptoDecrypt(inputBytes(buffer), { kind: "rsa-oaep", format: record.format, key: record.bytes, hash: normalizeHash(settings.oaepHash ?? "sha1"), label: settings.oaepLabel ? inputBytes(settings.oaepLabel) : new Uint8Array() }));
}

function legacyRsaPadding(value) {
  if (value === undefined || value === "RSA_PKCS1_PADDING") return 1;
  if (value === "RSA_NO_PADDING") return 3;
  if (value === "RSA_PKCS1_OAEP_PADDING") return 4;
  return value;
}

export function privateEncrypt(options, buffer) {
  const settings = encryptionSettings(options);
  const key = createPrivateKey(settings.key);
  const record = keyRecord(key);
  if (record.kind !== "rsa") throw new TypeError("An RSA key is required");
  return Buffer.from(cryptoRsaLegacyPrivateEncrypt(inputBytes(buffer), { kind: "rsa", format: record.format, key: record.bytes, padding: legacyRsaPadding(settings.padding) }));
}
export const pseudoRandomBytes = randomBytes;
export function publicDecrypt(options, buffer) {
  const settings = encryptionSettings(options);
  const key = createPublicKey(settings.key);
  const record = keyRecord(key);
  if (record.kind !== "rsa") throw new TypeError("An RSA key is required");
  return Buffer.from(cryptoRsaLegacyPublicDecrypt(inputBytes(buffer), { kind: "rsa", format: record.format, key: record.bytes, padding: legacyRsaPadding(settings.padding) }));
}

function scryptSettings(options, keylen) {
  const settings = options ?? {};
  const n = Number(settings.N ?? 16_384);
  const r = Number(settings.r ?? 8);
  const p = Number(settings.p ?? 1);
  const maxmem = Number(settings.maxmem ?? 32 * 1024 * 1024);
  if (!Number.isSafeInteger(n) || n < 2 || (n & (n - 1)) !== 0) throw new RangeError("Invalid scrypt N parameter");
  if (!Number.isSafeInteger(r) || r < 1) throw new RangeError("Invalid scrypt r parameter");
  if (!Number.isSafeInteger(p) || p < 1) throw new RangeError("Invalid scrypt p parameter");
  if (!Number.isSafeInteger(maxmem) || maxmem < 1) throw new RangeError("Invalid scrypt maxmem parameter");
  return { n, r, p, maxmem, keyLength: derivationLength(keylen) };
}

export function scryptSync(password, salt, keylen, options = {}) {
  const settings = scryptSettings(options, keylen);
  return Buffer.from(cryptoScrypt(inputBytes(password), { salt: inputBytes(salt), ...settings }));
}

export function scrypt(password, salt, keylen, options, callback) {
  if (typeof options === "function") { callback = options; options = {}; }
  if (typeof callback !== "function") throw new TypeError("The \"callback\" argument must be of type function");
  try {
    const value = scryptSync(password, salt, keylen, options);
    queueMicrotask(() => callback(null, value));
  } catch (error) { queueMicrotask(() => callback(error)); }
}
export const setEngine = unsupportedCrypto("setEngine");
export const setFips = unsupportedCrypto("setFips");
export function sign(algorithm, data, key, callback) {
  try {
    const value = createSign(algorithm).update(data).sign(key);
    if (typeof callback === "function") queueMicrotask(() => callback(null, value));
    return callback ? undefined : value;
  } catch (error) {
    if (typeof callback === "function") { queueMicrotask(() => callback(error)); return undefined; }
    throw error;
  }
}

export function verify(algorithm, data, key, signature, callback) {
  try {
    const value = createVerify(algorithm).update(data).verify(key, signature);
    if (typeof callback === "function") queueMicrotask(() => callback(null, value));
    return callback ? undefined : value;
  } catch (error) {
    if (typeof callback === "function") { queueMicrotask(() => callback(error)); return undefined; }
    throw error;
  }
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
