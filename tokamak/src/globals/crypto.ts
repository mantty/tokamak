import { isObject, markHostObject, propertyOf } from "./objects.ts";
import { randomBytes, digest, cryptoCreateDigest, cryptoAesGcm, cryptoPbkdf2, cryptoHkdf, cryptoGenerateKey, cryptoImportKey, cryptoExportKey, cryptoSign, cryptoVerify, cryptoEncrypt, cryptoDecrypt, cryptoDerive, cryptoTimingSafeEqual } from "tokamak:host";
import type { HostKey } from "tokamak:host";
import { bufferSourceBytes, sharedBufferSourceBytes, string, toNumber } from "./conversions.ts";
import { DOMException } from "./dom-exception.ts";
import { TextDecoder, TextEncoder } from "../streams/text.ts";
import { WritableStream } from "../streams/web.ts";

export function randomUUID(): string {
  const bytes = randomBytes(16);
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = [...bytes].map(value => value.toString(16).padStart(2, "0")).join("");
  return hex.slice(0, 8) + "-" + hex.slice(8, 12) + "-" + hex.slice(12, 16) + "-" + hex.slice(16, 20) + "-" + hex.slice(20);
}

type KeyType = "secret" | "public" | "private";

/** A CryptoKey's algorithm dictionary. */
interface KeyAlgorithm {
  name: string;
  hash?: { name: string };
  length?: number;
  namedCurve?: string;
  modulusLength?: number | undefined;
  publicExponent?: Uint8Array | undefined;
}

/** What a CryptoKey holds, with the kind of key the host knows it as. */
interface KeyRecord {
  algorithm: KeyAlgorithm;
  extractable: boolean;
  usages: string[];
  type: KeyType;
  data: Uint8Array;
  kind: string;
}

/** A JsonWebKey as workerd converts one, each absent member undefined. */
interface JsonWebKey {
  kty: string;
  use: string | undefined;
  key_ops: string[] | undefined;
  alg: string | undefined;
  ext: boolean | undefined;
  crv: string | undefined;
  x: string | undefined;
  y: string | undefined;
  d: string | undefined;
  n: string | undefined;
  e: string | undefined;
  p: string | undefined;
  q: string | undefined;
  dp: string | undefined;
  dq: string | undefined;
  qi: string | undefined;
  oth: unknown[] | undefined;
  k: string | undefined;
  pub: string | undefined;
  priv: string | undefined;
}

/** Key data to import: a JsonWebKey, or the bytes of a key in another format. */
type KeyData = { format: "jwk"; jwk: JsonWebKey } | { format: "raw" | "spki" | "pkcs8"; bytes: Uint8Array };

/** The converters of an algorithm dictionary's members. */
type Members = Record<string, (value: unknown, member: string) => unknown>;
/** An algorithm dictionary as workerd converts it: its name and its present members. */
type Dictionary<M extends Members> = { name: string } & { [K in keyof M]?: ReturnType<M[K]> };

/** The members an asymmetric algorithm is normalized from. */
interface AsymmetricParams {
  name: string;
  hash?: string;
  namedCurve?: string;
  modulusLength?: number;
  publicExponent?: Uint8Array;
}

/** An asymmetric algorithm, the kind of key the host knows it by, and the algorithm its keys report. */
interface Asymmetric {
  name: string;
  kind: string;
  namedCurve?: string;
  modulusLength?: number | undefined;
  publicExponent?: Uint8Array | undefined;
  keyAlgorithm: KeyAlgorithm;
}

const keyRecords = new WeakMap<CryptoKey, KeyRecord>();
const cryptoKeyBrand = {};
const hashNames = ["SHA-1", "SHA-256", "SHA-384", "SHA-512"];
const digestNames = ["MD5", ...hashNames];
const aesNames = ["AES-CTR", "AES-CBC", "AES-GCM", "AES-KW"];
const aesLengths = [128, 192, 256];
const derivationNames = ["HKDF", "PBKDF2"];
const hmacUsages = ["sign", "verify"];
const derivationUsages = ["deriveKey", "deriveBits"];
const ecCurves = ["P-256", "P-384", "P-521"];
// A CryptoKey's usages are a set, listed in this order.
const usageOrder = ["encrypt", "decrypt", "sign", "verify", "deriveKey", "deriveBits", "wrapKey", "unwrapKey"];
// The members of a JsonWebKey, in workerd's order.
const jwkMembers = ["kty", "use", "key_ops", "alg", "ext", "crv", "x", "y", "d", "n", "e", "p", "q", "dp", "dq", "qi", "oth", "k", "pub", "priv"];
// The converted members of each algorithm dictionary.
const importMembers = { hash: requestedName, length: int32, namedCurve: string };
const generateMembers = { hash: requestedName, modulusLength: int32, publicExponent: toBytes, length: int32, namedCurve: string };
const deriveMembers = { public: keyMember, salt: toBytes, iterations: int32, hash: requestedName, info: toBytes };
const encryptMembers = { iv: toBytes, additionalData: toBytes, tagLength: int32, counter: toBytes, length: int32, label: toBytes };
type ImportParams = Dictionary<typeof importMembers>;
type GenerateParams = Dictionary<typeof generateMembers>;
type DeriveParams = Dictionary<typeof deriveMembers>;
type EncryptParams = Dictionary<typeof encryptMembers>;

// Each method converts its arguments, throwing any TypeError, before it validates them.
export class SubtleCrypto {
  constructor() { markHostObject(this); }
  async digest(algorithm: unknown, data: unknown) {
    const name = requestedName(algorithm);
    const bytes = toBytes(data);
    return digest(digestName(name), bytes);
  }
  async importKey(format: unknown, keyData: unknown, algorithm: unknown, extractable: unknown, usages: unknown) {
    return importKeyData(string(format), keyMaterial(keyData), algorithmMembers(algorithm, importMembers), Boolean(extractable), normalizeUsages(usages));
  }
  async exportKey(format: unknown, key: unknown) {
    return exportKeyData(string(format), requireKey(key));
  }
  async generateKey(algorithm: unknown, extractable: unknown, usages: unknown) {
    const parameters = algorithmMembers(algorithm, generateMembers);
    const keyExtractable = Boolean(extractable);
    const keyUsages = normalizeUsages(usages);
    const name = upperAscii(parameters.name);
    if (name === "HMAC") return requireUsages(generateHmac(parameters, keyExtractable, keyUsages));
    if (aesNames.includes(name)) return requireUsages(generateAes(name, parameters, keyExtractable, keyUsages));
    if (derivationNames.includes(name)) throw notSupported(`Unrecognized key generation algorithm "${parameters.name}" requested.`);
    const asymmetric = normalizeAsymmetricAlgorithm(parameters);
    validateAsymmetricUsages(keyUsages, name, "pair");
    const key = cryptoGenerateKey({ kind: asymmetric.kind, curve: asymmetric.namedCurve, modulusLength: asymmetric.modulusLength, publicExponent: asymmetric.publicExponent });
    const allowed = asymmetricUsages(name);
    const publicKey = asymmetricKey(asymmetric, keyExtractable, keyUsages.filter(usage => allowed.public.includes(usage)), key, false);
    const privateKey = asymmetricKey(asymmetric, keyExtractable, keyUsages.filter(usage => allowed.private.includes(usage)), key, true);
    return { publicKey, privateKey };
  }
  async decrypt(algorithm: unknown, key: unknown, data: unknown) {
    return cryptOperation(false, algorithm, key, data);
  }
  async encrypt(algorithm: unknown, key: unknown, data: unknown) {
    return cryptOperation(true, algorithm, key, data);
  }
  async sign(algorithm: unknown, key: unknown, data: unknown) {
    const name = requestedName(algorithm);
    const record = requireKey(key);
    const bytes = toBytes(data);
    validateOperation(record, name, "sign");
    if (record.algorithm.name === "HMAC") return hmac(record, bytes);
    const hash = signatureHash(record, algorithm);
    return cryptoSign(bytes, { kind: record.kind, format: derFormat(record), key: record.data, hash, saltLength: saltLength(record, algorithm) });
  }
  async verify(algorithm: unknown, key: unknown, signature: unknown, data: unknown) {
    const name = requestedName(algorithm);
    const record = requireKey(key);
    const actual = toBytes(signature);
    const bytes = toBytes(data);
    validateOperation(record, name, "verify");
    if (record.algorithm.name === "HMAC") {
      const expected = new Uint8Array(hmac(record, bytes));
      if (expected.byteLength !== actual.byteLength) return false;
      return cryptoTimingSafeEqual(expected, actual);
    }
    const hash = signatureHash(record, algorithm);
    return cryptoVerify(actual, bytes, { kind: record.kind, format: derFormat(record), key: record.data, hash, saltLength: saltLength(record, algorithm) });
  }
  timingSafeEqual(left: unknown, right: unknown) { return cryptoTimingSafeEqual(toBytes(left), toBytes(right)); }
  async deriveBits(algorithm: unknown, baseKey: unknown, length: unknown) {
    const parameters = algorithmMembers(algorithm, deriveMembers);
    const record = requireKey(baseKey);
    const bitLength = deriveBitsLength(length);
    validateOperation(record, parameters.name, "deriveBits");
    return derivedBits(record, parameters, bitLength);
  }
  async deriveKey(algorithm: unknown, baseKey: unknown, derivedKeyAlgorithm: unknown, extractable: unknown, usages: unknown) {
    const parameters = algorithmMembers(algorithm, deriveMembers);
    const record = requireKey(baseKey);
    const derived = algorithmMembers(derivedKeyAlgorithm, importMembers);
    const keyExtractable = Boolean(extractable);
    const keyUsages = normalizeUsages(usages);
    validateOperation(record, parameters.name, "deriveKey");
    const bits = derivedBits(record, parameters, derivedKeyLength(derived));
    return importKeyData("raw", new Uint8Array(bits), derived, keyExtractable, keyUsages);
  }
  async wrapKey(format: unknown, key: unknown, wrappingKey: unknown, wrapAlgorithm: unknown) {
    const formatName = string(format);
    const record = requireKey(key);
    const wrapping = requireKey(wrappingKey);
    const parameters = algorithmMembers(wrapAlgorithm, encryptMembers);
    validateOperation(wrapping, parameters.name, "wrapKey");
    const exported = exportKeyData(formatName, record);
    const bytes = exported instanceof ArrayBuffer ? new Uint8Array(exported) : new TextEncoder().encode(JSON.stringify(exported));
    return crypt(true, wrapping, parameters, bytes);
  }
  async unwrapKey(format: unknown, wrappedKey: unknown, unwrappingKey: unknown, unwrapAlgorithm: unknown, unwrappedKeyAlgorithm: unknown, extractable: unknown, usages: unknown) {
    const formatName = string(format);
    const wrapped = toBytes(wrappedKey);
    const unwrapping = requireKey(unwrappingKey);
    const parameters = algorithmMembers(unwrapAlgorithm, encryptMembers);
    const algorithm = algorithmMembers(unwrappedKeyAlgorithm, importMembers);
    const keyExtractable = Boolean(extractable);
    const keyUsages = normalizeUsages(usages);
    validateOperation(unwrapping, parameters.name, "unwrapKey");
    const bytes = new Uint8Array(crypt(false, unwrapping, parameters, wrapped));
    return importKeyData(formatName, formatName === "jwk" ? unwrappedJwk(bytes) : bytes, algorithm, keyExtractable, keyUsages);
  }
}
const subtle = new SubtleCrypto();

export class CryptoKey {
  declare readonly usages: string[];
  declare readonly algorithm: KeyAlgorithm;
  declare readonly extractable: boolean;
  declare readonly type: KeyType;
  declare [Symbol.toStringTag]: string;

  constructor(algorithm: KeyAlgorithm, extractable: boolean, usages: string[], type: KeyType, data: Uint8Array, brand: unknown, kind: string) {
    markHostObject(this);
    if (brand !== cryptoKeyBrand) throw new TypeError("Illegal constructor");
    const record = { algorithm: cloneAlgorithm(algorithm), extractable: Boolean(extractable), usages: [...usages], type, data: data.slice(), kind };
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

function notSupported(message = "The requested cryptographic algorithm is not supported.") { return new DOMException(message, "NotSupportedError"); }
function unsupportedFormat(format: string) { return notSupported(`Unrecognized key import format "${format}".`); }
function invalidAccess(message = "The requested key cannot be used with this operation") { return new DOMException(message, "InvalidAccessError"); }
function dataError(message: string) { return new DOMException(message, "DataError"); }
function operationError(message: string) { return new DOMException(message, "OperationError"); }
function syntaxError(message: string) { return new DOMException(message, "SyntaxError"); }
// A copy of `algorithm` that shares no object with it.
function cloneAlgorithm(algorithm: KeyAlgorithm): KeyAlgorithm {
  const { hash, publicExponent } = algorithm;
  return { ...algorithm, ...(hash && { hash: { ...hash } }), ...(publicExponent && { publicExponent: publicExponent.slice() }) };
}
// Algorithm names compare without regard to ASCII case.
function upperAscii(text: string) { return text.replace(/[a-z]/g, letter => letter.toUpperCase()); }
// The name an algorithm identifier, a string or a dictionary, requests.
function requestedName(algorithm: unknown) {
  if (algorithm === undefined || algorithm === null) throw new TypeError("An algorithm is required");
  return string(isObject(algorithm) ? algorithm.name : algorithm);
}
function algorithmName(algorithm: unknown) { return upperAscii(requestedName(algorithm)); }
// `algorithm` as workerd's dictionary of it: the name and the present `members`, converted.
function algorithmMembers<M extends Members>(algorithm: unknown, members: M): Dictionary<M> {
  const converted: Record<string, unknown> = { name: requestedName(algorithm) };
  if (isObject(algorithm)) {
    for (const [member, convert] of Object.entries(members)) {
      const value = algorithm[member];
      if (value !== undefined) converted[member] = convert(value, member);
    }
  }
  // The members converted are those of `members`, each by its converter.
  return converted as Dictionary<M>;
}
// A WebIDL long, with NaN and the infinities as 0.
function int32(value: unknown) {
  const number = Math.trunc(toNumber(value));
  if (!Number.isFinite(number)) return 0;
  if (number < -2147483648 || number > 2147483647) throw new TypeError("Value out of range. Must be between -2147483648 and 2147483647 (inclusive).");
  return number | 0;
}
function keyMember(value: unknown, member: string) {
  if (!(value instanceof CryptoKey)) throw new TypeError(`The ${member} member must be a CryptoKey`);
  return value;
}
// A converted member, which is never null, or else a TypeError.
function requiredMember<T, K extends keyof T & string>(algorithm: T, member: K, dictionary = "algorithm"): T[K] & {} {
  const value = algorithm[member];
  if (value == null) throw new TypeError(`Missing field "${member}" in "${dictionary}".`);
  return value;
}
// The canonical name of the digest `name` identifies.
function digestName(name: string) {
  const canonical = upperAscii(name);
  if (!digestNames.includes(canonical)) throw notSupported("Unrecognized or unimplemented digest algorithm requested.");
  return canonical;
}
function normalizeDigest(algorithm: unknown) { return digestName(requestedName(algorithm)); }
function algorithmHash(algorithm: unknown) {
  const value = typeof algorithm === "object" && algorithm !== null ? propertyOf(algorithm, "hash") : undefined;
  const name = algorithmName(value);
  if (!hashNames.includes(name)) throw notSupported();
  return name;
}
// The name of the hash `algorithm` names, as an HMAC or RSA key's algorithm does.
function hashName(algorithm: KeyAlgorithm) {
  if (algorithm.hash === undefined) throw notSupported();
  return algorithm.hash.name;
}
// `usages` as a usage set; names no algorithm allows are kept for validation to reject.
function normalizeUsages(usages: unknown) {
  if (!Array.isArray(usages)) throw new TypeError("Key usages must be an array");
  const rank = (usage: string) => (usageOrder.includes(usage) ? usageOrder.indexOf(usage) : usageOrder.length);
  return [...new Set(Array.from(usages, string))].sort((left, right) => rank(left) - rank(right));
}
function validateUsages(operation: string, name: string, usages: string[], allowed: string[]) {
  const invalid = usages.find(usage => !allowed.includes(usage));
  if (invalid !== undefined) throw syntaxError(`Attempt to ${operation} ${name} key with invalid usage "${invalid}".`);
}
// A secret or private key needs at least one usage.
function requireUsages(key: CryptoKey) {
  const record = requireKey(key);
  if (record.type !== "public" && record.usages.length === 0) throw syntaxError("Secret/private CryptoKeys must have at least one usage.");
  return key;
}
function aesUsages(name: string) { return name === "AES-KW" ? ["wrapKey", "unwrapKey"] : ["encrypt", "decrypt", "wrapKey", "unwrapKey"]; }
// The usages each key of an asymmetric algorithm's pair allows.
function asymmetricUsages(name: string): { public: string[]; private: string[] } {
  if (name === "RSA-OAEP") return { public: ["encrypt"], private: ["decrypt"] };
  if (name === "ECDH" || name === "X25519") return { public: [], private: ["deriveBits", "deriveKey"] };
  return { public: ["verify"], private: ["sign"] };
}
function validateAsymmetricUsages(usages: string[], name: string, type: "public" | "private" | "pair") {
  const allowed = asymmetricUsages(name);
  validateUsages(type === "pair" ? "generate" : "import", name, usages, type === "pair" ? [...allowed.public, ...allowed.private] : allowed[type]);
}
function requireKey(key: unknown) {
  const record = key instanceof CryptoKey ? keyRecords.get(key) : undefined;
  if (!record) throw new TypeError("Expected a CryptoKey");
  return record;
}
// The algorithm an operation requests must be the key's, and the key must allow `usage`.
function validateOperation(record: KeyRecord, name: string, usage: string) {
  const keyName = record.algorithm.name;
  if (upperAscii(name) !== upperAscii(keyName)) throw invalidAccess(`Requested algorithm "${name}" does not match this CryptoKey's algorithm "${keyName}".`);
  if (!record.usages.includes(usage)) throw invalidAccess(`Requested key usage "${usage}" does not match any usage listed in this CryptoKey.`);
}
function toBytes(value: unknown) {
  const bytes = bufferSourceBytes(value);
  if (!bytes) throw new TypeError("Data must be an ArrayBuffer or ArrayBufferView");
  return bytes.slice();
}

// Key data as workerd converts it: the bytes of a BufferSource, or else a JsonWebKey.
function keyMaterial(data: unknown) {
  const bytes = bufferSourceBytes(data);
  if (bytes) return bytes.slice();
  if (!isObject(data)) throw new TypeError("Key data must be a BufferSource or a JsonWebKey");
  return jsonWebKey(data);
}
// `value` as workerd's JsonWebKey: every member converted, absent ones undefined.
function jsonWebKey(value: Record<PropertyKey, unknown>): JsonWebKey {
  const text = (member: string) => optionalString(value[member]);
  return {
    kty: string(value.kty), use: text("use"), key_ops: keyOperations(value.key_ops), alg: text("alg"),
    ext: value.ext === undefined ? undefined : Boolean(value.ext), crv: text("crv"), x: text("x"), y: text("y"), d: text("d"),
    n: text("n"), e: text("e"), p: text("p"), q: text("q"), dp: text("dp"), dq: text("dq"), qi: text("qi"),
    oth: jwkArray(value.oth, "oth"), k: text("k"), pub: text("pub"), priv: text("priv"),
  };
}
function optionalString(value: unknown) { return value === undefined ? undefined : string(value); }
function keyOperations(value: unknown) {
  const operations = jwkArray(value, "key_ops");
  return operations === undefined ? undefined : Array.from(operations, string);
}
function jwkArray(value: unknown, member: string): unknown[] | undefined {
  if (value === undefined) return undefined;
  if (!Array.isArray(value)) throw new TypeError(`The JsonWebKey ${member} member must be an array`);
  return value;
}
// An exported JsonWebKey lists every member, in workerd's order.
function exportedJwk(value: Record<PropertyKey, unknown>) {
  return Object.fromEntries(jwkMembers.map(member => [member, value[member]]));
}
// The JsonWebKey an unwrapped key's JSON text holds.
function unwrappedJwk(bytes: Uint8Array) {
  const value: unknown = JSON.parse(new TextDecoder("utf-8", { ignoreBOM: true }).decode(bytes));
  if (value === null) throw new TypeError("An unwrapped JsonWebKey cannot be null");
  if (!isObject(value)) throw dataError('Missing "kty" field or corrupt JSON unwrapping key?');
  return jsonWebKey(value);
}
// The JsonWebKey the host exports as JSON text.
export function hostJwk(output: ArrayBuffer) {
  const value: unknown = JSON.parse(new TextDecoder().decode(output));
  if (!isObject(value)) throw new TypeError("The exported JsonWebKey is not an object");
  return value;
}

// importKey once its arguments are converted.
function importKeyData(format: string, data: Uint8Array | JsonWebKey, algorithm: ImportParams, extractable: boolean, usages: string[]) {
  const key = keyData(format, data, algorithm.name, extractable);
  const name = upperAscii(algorithm.name);
  if (name === "HMAC") return requireUsages(importHmac(key, algorithm, extractable, usages));
  if (aesNames.includes(name)) return requireUsages(importAes(name, key, extractable, usages));
  if (derivationNames.includes(name)) return requireUsages(importDerivationKey(name, key, extractable, usages));
  return requireUsages(importAsymmetric(key, algorithm, extractable, usages));
}
// `data` as the key data `format` imports, which it must be.
function keyData(format: string, data: Uint8Array | JsonWebKey, name: string, extractable: boolean): KeyData {
  if (format === "jwk") {
    if (data instanceof Uint8Array) throw new TypeError('Import data provided for "jwk" import format must be a JsonWebKey.');
    if (data.ext === false && extractable) throw dataError(`JWK ext field for "${name}" is set to false but extractable is true`);
    return { format, jwk: data };
  }
  if (format !== "raw" && format !== "spki" && format !== "pkcs8") throw unsupportedFormat(format);
  if (!(data instanceof Uint8Array)) throw new TypeError('Import data provided for "raw", "pkcs8", or "spki" import formats must be a buffer source.');
  return { format, bytes: data };
}
function importHmac(key: KeyData, algorithm: ImportParams, extractable: boolean, usages: string[]) {
  validateUsages("import secret", "HMAC", usages, hmacUsages);
  const hash = requiredMember(algorithm, "hash");
  const bytes = hmacKeyBytes(key, hash);
  const keySize = bytes.byteLength * 8;
  const length = algorithm.length ?? keySize;
  if (length === 0 || length > keySize || length <= keySize - 8) throw dataError(`Imported HMAC key length (${length}) must be a non-zero value up to 7 bits less than, and no greater than, the bit length of the raw key data (${keySize}).`);
  return hmacKey(digestName(hash), length, bytes, extractable, usages);
}
function hmacKeyBytes(key: KeyData, hash: string) {
  if (key.format === "raw") return key.bytes;
  if (key.format !== "jwk") throw unsupportedFormat(key.format);
  const { jwk } = key;
  if (jwk.kty !== "oct") throw dataError(`HMAC "jwk" key import requires a JSON Web Key with Key Type parameter ("kty") equal to "oct" (encountered "${jwk.kty}").`);
  const bytes = base64urlDecode(jwk.k);
  if (!bytes) throw dataError('HMAC "jwk" key import requires a base64Url encoding of the key');
  if (jwk.alg !== undefined) validateHmacJwkAlg(jwk.alg, hash);
  return bytes;
}
// A JWK alg is compared with a hash only when the hash is spelt "SHA-...".
function validateHmacJwkAlg(alg: string, hash: string) {
  if (!hash.startsWith("SHA-")) throw notSupported(`Unrecognized or unimplemented hash algorithm requested${alg}`);
  const expected = `HS${hash.slice(4)}`;
  if (alg !== expected) throw dataError(`HMAC "jwk" key import specifies "alg" that is incompatible with the hash name (encountered "${alg}", expected "${expected}").`);
}
function generateHmac(algorithm: GenerateParams, extractable: boolean, usages: string[]) {
  const hash = digestName(requiredMember(algorithm, "hash"));
  validateUsages("generate", "HMAC", usages, hmacUsages);
  const length = algorithm.length ?? hmacBlockLength(hash);
  if (length <= 0) throw operationError(`HMAC key length must be a non-zero unsigned long integer (requested ${length}).`);
  return hmacKey(hash, length, randomKeyBytes(Math.ceil(length / 8)), extractable, usages);
}
// An HMAC key of `length` bits keeps the bytes that hold them, with the bits past `length` zero.
// Its algorithm reports the length in 16 bits, as workerd's does.
function hmacKey(hash: string, length: number, bytes: Uint8Array, extractable: boolean, usages: string[]) {
  const unusedBits = bytes.byteLength * 8 - length;
  if (unusedBits > 0) bytes[bytes.byteLength - 1] &= 0xff << unusedBits;
  return secretKey({ name: "HMAC", hash: { name: hash }, length: length % 65536 }, extractable, usages, bytes);
}
// `length` random bytes, gathered within the host's limit of 65536 a call.
function randomKeyBytes(length: number) {
  const bytes = new Uint8Array(length);
  for (let offset = 0; offset < length; offset += 65536) bytes.set(randomBytes(Math.min(65536, length - offset)), offset);
  return bytes;
}
function hmacBlockLength(hash: string) { return hash === "SHA-384" || hash === "SHA-512" ? 1024 : 512; }
function importAes(name: string, key: KeyData, extractable: boolean, usages: string[]) {
  validateUsages("import secret", name, usages, aesUsages(name));
  if (key.format !== "raw" && key.format !== "jwk") throw unsupportedFormat(key.format);
  const bytes = key.format === "jwk" ? aesJwkBytes(name, key.jwk, usages) : key.bytes;
  validateAesKeyLength(bytes);
  return secretKey({ name, length: bytes.byteLength * 8 }, extractable, usages, bytes);
}
function aesJwkBytes(name: string, jwk: JsonWebKey, usages: string[]) {
  if (jwk.kty !== "oct") throw dataError(`Symmetric "jwk" key import requires a JSON Web Key with Key Type parameter "kty" equal to "oct" (encountered "${jwk.kty}").`);
  const bytes = base64urlDecode(jwk.k);
  if (!bytes) throw dataError('Symmetric "jwk" key import requires a base64Url encoding of the key.');
  validateAesKeyLength(bytes);
  const alg = `A${bytes.byteLength * 8}${name.slice(4)}`;
  if (jwk.alg !== undefined && jwk.alg !== alg) throw dataError(`Symmetric "jwk" key contains invalid "alg" value "${jwk.alg}", expected "${alg}".`);
  if (usages.length > 0 && jwk.use !== undefined && jwk.use !== "enc") throw dataError(`Symmetric "jwk" key must have a "use" of "enc", not "${jwk.use}".`);
  validateKeyOperations(jwk.key_ops, usages);
  return bytes;
}
function validateAesKeyLength(bytes: Uint8Array) {
  if (!aesLengths.includes(bytes.byteLength * 8)) throw dataError(`Imported AES key length must be 128, 192, or 256 bits but provided ${bytes.byteLength * 8}.`);
}
// A JWK's key_ops, when present, list each requested usage, and no operation twice.
function validateKeyOperations(operations: string[] | undefined, usages: string[]) {
  if (operations === undefined) return;
  const duplicate = operations.find((operation, index) => operations.indexOf(operation) !== index);
  if (duplicate !== undefined) throw dataError(`Symmetric "jwk" key contains duplicate value "${duplicate}", in "key_op".`);
  const missing = usages.find(usage => !operations.includes(usage));
  if (missing !== undefined) throw dataError(`"jwk" key missing usage "${missing}", in "key_ops".`);
}
function generateAes(name: string, algorithm: GenerateParams, extractable: boolean, usages: string[]) {
  validateUsages("generate", name, usages, aesUsages(name));
  const length = requiredMember(algorithm, "length");
  if (!aesLengths.includes(length)) throw operationError(`Generated AES key length must be 128, 192, or 256 bits but requested ${length}.`);
  return secretKey({ name, length }, extractable, usages, randomBytes(length / 8));
}
// HKDF and PBKDF2 keys are raw secrets that are never extractable.
function importDerivationKey(name: string, key: KeyData, extractable: boolean, usages: string[]) {
  validateUsages("import secret", name, usages, derivationUsages);
  if (extractable) throw syntaxError(`${name} key cannot be extractable.`);
  if (key.format !== "raw") throw notSupported(`${name} key must be imported in "raw" format (requested "${key.format}").`);
  return secretKey({ name }, false, usages, key.bytes);
}
function importAsymmetric(key: KeyData, algorithm: AsymmetricParams, extractable: boolean, usages: string[]) {
  const asymmetric = normalizeAsymmetricAlgorithm(algorithm);
  if (key.format === "jwk") return importJwk(key.jwk, asymmetric, extractable, usages);
  if (key.format === "raw" && !["ECDSA", "ECDH", "Ed25519", "X25519", "NODE-ED25519"].includes(asymmetric.name)) throw notSupported();
  const type = key.format === "pkcs8" ? "private" : "public";
  validateAsymmetricUsages(usages, asymmetric.name, type);
  const imported = cryptoImportKey({ format: key.format, kind: asymmetric.kind, curve: asymmetric.namedCurve, key: key.bytes });
  return asymmetricKey(importedAlgorithm(asymmetric, imported), extractable, usages, imported, type === "private");
}

// exportKey once its arguments are converted.
function exportKeyData(format: string, record: KeyRecord) {
  if (!record.extractable) throw invalidAccess(`Attempt to export non-extractable ${record.algorithm.name} key.`);
  if (record.type === "secret") return exportSecretKey(format, record);
  if (format === "raw" && record.type !== "public") throw invalidAccess("The key is not public");
  if (!["raw", "spki", "pkcs8", "jwk"].includes(format)) throw notSupported();
  const output = cryptoExportKey({ format, keyFormat: derFormat(record), kind: record.kind, key: record.data });
  if (format === "jwk") return addJwkMetadata(hostJwk(output), record);
  return output;
}
function exportSecretKey(format: string, record: KeyRecord) {
  if (format === "raw") return record.data.slice().buffer;
  if (format !== "jwk") throw notSupported(`Unimplemented key export format "${format}".`);
  return exportedJwk({ kty: "oct", key_ops: record.usages.slice(), alg: secretJwkAlg(record.algorithm), ext: true, k: base64urlEncode(record.data) });
}
function secretJwkAlg(algorithm: KeyAlgorithm) {
  const { name, length } = algorithm;
  if (name !== "HMAC") return `A${length}${name.slice(4)}`;
  const hash = hashName(algorithm);
  if (!hash.startsWith("SHA-")) throw notSupported(`Unimplemented JWK key export format for key algorithm "${hash}".`);
  return `HS${hash.slice(4)}`;
}

// The deriveBits length, which may be null.
function deriveBitsLength(length: unknown) {
  if (length === undefined || length === null) return null;
  const value = int32(length);
  if (value < 0) throw new TypeError("deriveBits length must be an unsigned long integer.");
  return value;
}
// `length` bits derived from the key `record`, which the operation's checks have matched to `algorithm`.
function derivedBits(record: KeyRecord, algorithm: DeriveParams, length: number | null) {
  if (record.algorithm.name === "HKDF") return hkdfBits(record, algorithm, length);
  if (record.algorithm.name === "PBKDF2") return pbkdf2Bits(record, algorithm, length);
  return agreedBits(record, algorithm, length);
}
function hkdfBits(record: KeyRecord, algorithm: DeriveParams, length: number | null) {
  const hash = digestName(requiredMember(algorithm, "hash"));
  const salt = requiredMember(algorithm, "salt");
  const info = requiredMember(algorithm, "info");
  return cryptoHkdf(hash, record.data, salt, info, derivedByteLength("HKDF", length));
}
function pbkdf2Bits(record: KeyRecord, algorithm: DeriveParams, length: number | null) {
  const hash = digestName(requiredMember(algorithm, "hash"));
  const salt = requiredMember(algorithm, "salt");
  const iterations = requiredMember(algorithm, "iterations");
  const byteLength = derivedByteLength("PBKDF2", length);
  if (iterations <= 0) throw operationError(`PBKDF2 requires a positive iteration count (requested ${iterations}).`);
  return cryptoPbkdf2(hash, record.data, salt, iterations, byteLength);
}
function derivedByteLength(name: string, length: number | null) {
  if (length === null) throw operationError(`${name} cannot derive a key with null length.`);
  if (length % 8 !== 0) throw operationError(`${name} requires a derived key length that is a multiple of eight (requested ${length}).`);
  return length / 8;
}
// ECDH or X25519 agreement between the private key `record` and the public key `algorithm` names.
function agreedBits(record: KeyRecord, algorithm: DeriveParams, length: number | null) {
  const name = record.algorithm.name;
  if (!length || length % 8 !== 0) throw operationError("Derived length must be a non-zero multiple of 8");
  if (record.type !== "private") throw invalidAccess();
  const peer = algorithm.public;
  if (!(peer instanceof CryptoKey) || peer.type !== "public" || peer.algorithm.name !== name) throw invalidAccess();
  if (name === "ECDH" && record.algorithm.namedCurve !== peer.algorithm.namedCurve) throw invalidAccess();
  const output = cryptoDerive({ kind: name === "X25519" ? "x25519" : "ec", format: "pkcs8", key: record.data, peer: requireKey(peer).data });
  if (length > output.byteLength * 8) throw operationError("Derived secret is shorter than requested");
  return output.slice(0, length / 8);
}
// The bit length deriveKey derives for a key of `algorithm`; null for HKDF and PBKDF2 keys, which cannot be derived.
function derivedKeyLength(algorithm: ImportParams) {
  const name = upperAscii(algorithm.name);
  if (aesNames.includes(name)) {
    const length = requiredMember(algorithm, "length", "derivedKeyParams");
    if (!aesLengths.includes(length)) throw operationError(`Derived AES key must be 128, 192, or 256 bits in length but provided ${length}.`);
    return length;
  }
  if (name === "HMAC") {
    if (algorithm.length === undefined) return hmacBlockLength(digestName(requiredMember(algorithm, "hash", "derivedKeyParams")));
    if (algorithm.length > 0) return algorithm.length;
    throw new TypeError("HMAC key length must be a non-zero unsigned long integer.");
  }
  if (derivationNames.includes(name)) return null;
  throw notSupported(`Unrecognized derived key type "${algorithm.name}" requested.`);
}

function cryptOperation(encrypt: boolean, algorithm: unknown, key: unknown, data: unknown) {
  const parameters = algorithmMembers(algorithm, encryptMembers);
  const record = requireKey(key);
  const bytes = toBytes(data);
  validateOperation(record, parameters.name, encrypt ? "encrypt" : "decrypt");
  return crypt(encrypt, record, parameters, bytes);
}
// Encryption or decryption of `bytes` with the key `record`, which the operation's checks have matched to `algorithm`.
function crypt(encrypt: boolean, record: KeyRecord, algorithm: EncryptParams, bytes: Uint8Array) {
  const name = record.algorithm.name;
  if (name === "AES-KW") return cryptoAesGcm(encrypt, record.data, new Uint8Array(), bytes, { tagLength: 128, mode: name });
  if (aesNames.includes(name)) return aesCrypt(encrypt, record, algorithm, bytes);
  if (name === "RSA-OAEP") return rsaOaep(encrypt, record, algorithm, bytes);
  throw notSupported();
}
// AES-CTR, AES-CBC or AES-GCM.
function aesCrypt(encrypt: boolean, record: KeyRecord, algorithm: EncryptParams, bytes: Uint8Array) {
  const mode = record.algorithm.name;
  const parameter = requiredMember(algorithm, mode === "AES-CTR" ? "counter" : "iv");
  const options = { mode, tagLength: aesBitLength(mode, algorithm), additionalData: algorithm.additionalData ?? new Uint8Array() };
  return cryptoAesGcm(encrypt, record.data, parameter, bytes, options);
}
// The counter length of AES-CTR or the tag length of AES-GCM; AES-CBC has neither.
function aesBitLength(mode: string, algorithm: EncryptParams) {
  if (mode === "AES-CBC") return 128;
  if (mode === "AES-CTR") {
    const length = requiredMember(algorithm, "length");
    if (length < 1 || length > 128) throw operationError(`Invalid counter of ${length} bits length provided.`);
    return length;
  }
  const tagLength = algorithm.tagLength ?? 128;
  if (![32, 64, 96, 104, 112, 120, 128].includes(tagLength)) throw operationError(`Invalid AES-GCM tag length ${tagLength}.`);
  return tagLength;
}
function rsaOaep(encrypt: boolean, record: KeyRecord, algorithm: EncryptParams, bytes: Uint8Array) {
  const options = { format: derFormat(record), key: record.data, hash: hashName(record.algorithm), label: algorithm.label ?? new Uint8Array() };
  return encrypt ? cryptoEncrypt(bytes, options) : cryptoDecrypt(bytes, options);
}
function hmac(record: KeyRecord, data: Uint8Array) {
  const state = cryptoCreateDigest(hashName(record.algorithm), record.data);
  state.update(data);
  return state.finish();
}
function signatureHash(record: KeyRecord, algorithm: unknown) {
  return record.algorithm.hash?.name ?? (["Ed25519", "NODE-ED25519"].includes(record.algorithm.name) ? "SHA-256" : algorithmHash(algorithm));
}
function saltLength(record: KeyRecord, algorithm: unknown) {
  if (record.algorithm.name !== "RSA-PSS") return undefined;
  if (algorithm === null || typeof algorithm !== "object") throw new TypeError("RSA-PSS saltLength is required");
  const length = integer(propertyOf(algorithm, "saltLength"), "saltLength");
  if (length > 0x7fffffff) throw new TypeError("RSA-PSS saltLength is too large");
  return length;
}
function createKey(algorithm: KeyAlgorithm, extractable: boolean, usages: string[], type: KeyType, data: Uint8Array, kind: string) { return new CryptoKey(algorithm, extractable, usages, type, data, cryptoKeyBrand, kind); }
function secretKey(algorithm: KeyAlgorithm, extractable: boolean, usages: string[], data: Uint8Array) { return createKey(algorithm, extractable, usages, "secret", data, "secret"); }
function asymmetricKey(algorithm: Asymmetric, extractable: boolean, usages: string[], key: HostKey, isPrivate: boolean) {
  const data = isPrivate ? key.private : key.public;
  if (data === undefined) throw dataError("The key is not private");
  return createKey(algorithm.keyAlgorithm, extractable, usages, isPrivate ? "private" : "public", data, algorithm.kind);
}
// The DER structure an asymmetric key's data holds.
function derFormat(record: KeyRecord) { return record.type === "private" ? "pkcs8" : "spki"; }
function normalizeAsymmetricAlgorithm(algorithm: AsymmetricParams): Asymmetric {
  const name = algorithmName(algorithm);
  if (["RSASSA-PKCS1-V1_5", "RSA-PSS", "RSA-OAEP"].includes(name)) {
    const modulusLength = algorithm.modulusLength === undefined ? undefined : integer(algorithm.modulusLength, "modulusLength");
    const publicExponent = algorithm.publicExponent === undefined ? undefined : toBytes(algorithm.publicExponent);
    const hash = algorithmHash(algorithm);
    return { name, kind: name === "RSA-PSS" ? "rsa-pss" : name === "RSA-OAEP" ? "rsa-oaep" : "rsa", modulusLength, publicExponent, keyAlgorithm: { name: canonicalAlgorithmName(name), modulusLength, publicExponent: publicExponent?.slice(), hash: { name: hash } } };
  }
  if (name === "ECDSA" || name === "ECDH") {
    const namedCurve = String(algorithm.namedCurve ?? "").toUpperCase();
    if (!ecCurves.includes(namedCurve)) throw notSupported();
    return { name, kind: "ec", namedCurve, keyAlgorithm: { name, namedCurve } };
  }
  if (name === "ED25519") return { name: "Ed25519", kind: "ed25519", keyAlgorithm: { name: "Ed25519" } };
  if (name === "X25519") return { name, kind: "x25519", keyAlgorithm: { name } };
  if (name === "NODE-ED25519") {
    if (String(algorithm.namedCurve ?? "").toUpperCase() !== "NODE-ED25519") throw new TypeError("namedCurve is required");
    return { name, kind: "ed25519", namedCurve: "NODE-ED25519", keyAlgorithm: { name, namedCurve: "NODE-ED25519" } };
  }
  throw notSupported();
}
function integer(value: unknown, name: string) { const number = Number(value); if (!Number.isSafeInteger(number) || number < 0) throw new TypeError(`${name} must be an integer`); return number; }

function importJwk(jwk: JsonWebKey, asymmetric: Asymmetric, extractable: boolean, usages: string[]) {
  if (jwk.key_ops?.some(value => !usages.includes(value))) throw dataError("JWK key operations do not match usages");
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
function addJwkMetadata(value: Record<PropertyKey, unknown>, record: KeyRecord) {
  value.key_ops = record.usages.slice();
  value.ext = record.extractable;
  const name = algorithmName(record.algorithm);
  if (name === "RSASSA-PKCS1-V1_5") value.alg = `RS${hashName(record.algorithm).slice(4)}`;
  else if (name === "RSA-PSS") value.alg = `PS${hashName(record.algorithm).slice(4)}`;
  else if (name === "RSA-OAEP") value.alg = hashName(record.algorithm) === "SHA-1" ? "RSA-OAEP" : `RSA-OAEP-${hashName(record.algorithm).slice(4)}`;
  else if (name === "Ed25519" || name === "NODE-ED25519") value.alg = "EdDSA";
  return exportedJwk(value);
}

function canonicalAlgorithmName(name: string) {
  return name === "RSASSA-PKCS1-V1_5" ? "RSASSA-PKCS1-v1_5" : name === "ED25519" ? "Ed25519" : name;
}
// An RSA key algorithm describes the imported key's modulus and exponent.
function importedAlgorithm(asymmetric: Asymmetric, key: HostKey) {
  if (key.modulusLength === undefined) return asymmetric;
  return { ...asymmetric, keyAlgorithm: { ...asymmetric.keyAlgorithm, modulusLength: key.modulusLength, publicExponent: key.publicExponent } };
}
function base64urlEncode(bytes: Uint8Array) { return bytes.toBase64({ alphabet: "base64url", omitPadding: true }); }
// The bytes of a JWK member in base64url; as in workerd, base64's alphabet and padding are also accepted.
function base64urlDecode(value: string | undefined) {
  if (value === undefined) return undefined;
  try { return Uint8Array.fromBase64(value.replaceAll("-", "+").replaceAll("_", "/")); }
  catch { return undefined; }
}

export class DigestStream extends WritableStream {
  #digest: Promise<ArrayBuffer>;
  #written: { bytes: bigint };

  constructor(algorithm: unknown) {
    const name = normalizeDigest(algorithm);
    const digest = Promise.withResolvers<ArrayBuffer>();
    const written = { bytes: 0n };
    const state = cryptoCreateDigest(name);
    const update = (chunk: unknown) => {
      const bytes = toBytes(chunk);
      state.update(bytes);
      written.bytes += BigInt(bytes.byteLength);
    };
    super({
      write(chunk) {
        try { update(chunk); }
        catch (error) { digest.reject(error); throw error; }
      },
      close() {
        try { digest.resolve(state.finish()); }
        catch (error) { digest.reject(error); throw error; }
      },
      abort: reason => digest.reject(reason),
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
  getRandomValues<T>(value: T): T {
    if (!ArrayBuffer.isView(value)) throw new TypeError("Expected an integer typed array");
    if (value instanceof DataView || value instanceof Float32Array || value instanceof Float64Array || (typeof Float16Array !== "undefined" && value instanceof Float16Array)) throw new DOMException("The provided value is not an integer typed array.", "TypeMismatchError");
    if (value.byteLength > 65536) throw new DOMException("The requested length exceeds 65,536 bytes.", "QuotaExceededError");
    sharedBufferSourceBytes(value)?.set(randomBytes(value.byteLength));
    return value;
  }
  randomUUID() { return randomUUID(); }
}

export const crypto = new Crypto();
