import { kvDelete, kvGet, kvGetMany, kvList, kvPut } from "tokamak:host";

const MAX_KEY_LENGTH = 512;
const MAX_VALUE_LENGTH = 25 * 1024 * 1024;
const MAX_METADATA_LENGTH = 1024;
const MAX_LIST_KEYS = 1000;
const MAX_BULK_KEYS = 100;
const MIN_CACHE_TTL = 30;
const MIN_EXPIRATION_TTL = 60;
const RESPONSE_TYPES = "Possible types are \"text\", \"arrayBuffer\", \"json\", and \"stream\".";
const VALUE_TYPES = "KV put() accepts only strings, ArrayBuffers, ArrayBufferViews, and ReadableStreams as values.";

const encoder = new TextEncoder();

function byteLength(text) {
  return encoder.encode(text).byteLength;
}

// A failed request to the KV service, as workerd reports its status.
function failure(method, status, message) {
  return new Error(`KV ${method} failed: ${status} ${message}`);
}

// An option the binding takes as a 32-bit integer.
function integer(value) {
  return Math.trunc(Number(value)) | 0;
}

function validateKeyLength(method, key) {
  const length = byteLength(key);
  if (length > MAX_KEY_LENGTH) {
    throw failure(method, 414, `UTF-8 encoded length of ${length} exceeds key length limit of ${MAX_KEY_LENGTH}.`);
  }
}

function validateKeyName(method, name) {
  if (name === "") throw new TypeError("Key name cannot be empty.");
  if (name === ".") throw new TypeError("\".\" is not allowed as a key name.");
  if (name === "..") throw new TypeError("\"..\" is not allowed as a key name.");
  validateKeyLength(method, name);
}

// The service's checks of a key in a bulk request.
function validateBulkKey(key) {
  if (key === "") throw failure("GET_BULK", 400, "Key names must not be empty");
  if (key === "." || key === "..") throw failure("GET_BULK", 400, `Illegal key name "${key}". Please use a different name.`);
  validateKeyLength("GET_BULK", key);
}

function validateCacheTtl(method, cacheTtl) {
  if (cacheTtl !== undefined && (Number.isNaN(cacheTtl) || cacheTtl < MIN_CACHE_TTL)) {
    throw failure(method, 400, `Invalid cache_ttl of ${cacheTtl}. Cache TTL must be at least ${MIN_CACHE_TTL}.`);
  }
}

function getOptions(options) {
  if (typeof options === "string") return { type: options, cacheTtl: undefined };
  if (options === undefined || options === null) return { type: undefined, cacheTtl: undefined };
  return {
    type: options.type === undefined ? undefined : String(options.type),
    cacheTtl: options.cacheTtl === undefined ? undefined : integer(options.cacheTtl),
  };
}

function decode(bytes, type) {
  switch (type) {
    case "text": return new TextDecoder().decode(bytes);
    case "json": return JSON.parse(new TextDecoder().decode(bytes));
    case "arrayBuffer": return bytes.buffer;
    case "stream": return new ReadableStream({ type: "bytes", start(controller) { controller.enqueue(bytes); controller.close(); } });
    default: throw new TypeError(`Unknown response type. ${RESPONSE_TYPES}`);
  }
}

function copy(bytes) {
  return { bytes: bytes.slice(), length: bytes.byteLength };
}

// A value's length, and its bytes unless it exceeds the value limit.
async function readValue(body) {
  if (typeof body !== "object" || body === null) return copy(encoder.encode(String(body)));
  if (body instanceof ArrayBuffer) return copy(new Uint8Array(body));
  if (ArrayBuffer.isView(body)) return copy(new Uint8Array(body.buffer, body.byteOffset, body.byteLength));
  if (body instanceof ReadableStream) return readStream(body);
  throw new TypeError(VALUE_TYPES);
}

async function readStream(stream) {
  const chunks = [];
  let length = 0;
  for await (const chunk of stream) {
    const bytes = ArrayBuffer.isView(chunk) ? new Uint8Array(chunk.buffer, chunk.byteOffset, chunk.byteLength) : new Uint8Array(chunk);
    length += bytes.byteLength;
    if (length <= MAX_VALUE_LENGTH) chunks.push(bytes);
  }
  if (length > MAX_VALUE_LENGTH) return { bytes: null, length };
  const bytes = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return { bytes, length };
}

// The expiration, in seconds since the epoch, `put` options ask for.
function expiration(options, now) {
  if (options.expirationTtl !== undefined) {
    const ttl = integer(options.expirationTtl);
    if (ttl <= 0) throw failure("PUT", 400, `Invalid expiration_ttl of ${ttl}. Please specify integer greater than 0.`);
    if (ttl < MIN_EXPIRATION_TTL) {
      throw failure("PUT", 400, `Invalid expiration_ttl of ${ttl}. Expiration TTL must be at least ${MIN_EXPIRATION_TTL}.`);
    }
    return now + ttl;
  }
  if (options.expiration !== undefined) {
    const at = integer(options.expiration);
    if (at <= now) {
      throw failure("PUT", 400, `Invalid expiration of ${at}. Please specify integer greater than the current number of seconds since the UNIX epoch.`);
    }
    if (at < now + MIN_EXPIRATION_TTL) {
      throw failure("PUT", 400, `Invalid expiration of ${at}. Expiration times must be at least ${MIN_EXPIRATION_TTL} seconds in the future.`);
    }
    return at;
  }
  return null;
}

// Whether a property name is an array index, which workerd leaves out of bulk results.
function isArrayIndex(name) {
  const index = Number(name);
  return String(index >>> 0) === name && index !== 2 ** 32 - 1;
}

function parseMetadata(metadata) {
  return metadata == null ? null : JSON.parse(metadata);
}

function bulkValue(entry, type, withMetadata) {
  if (entry == null) return { value: null, size: 0 };
  const text = new TextDecoder().decode(entry.value);
  let value = text;
  if (type === "json") {
    try { value = JSON.parse(text); } catch { throw failure("GET_BULK", 400, "At least one of the requested keys corresponds to a non-json value"); }
  }
  if (withMetadata) value = { value, metadata: parseMetadata(entry.metadata) };
  return { value, size: text.length };
}

export class KVNamespace {
  #binding;

  constructor(binding) {
    this.#binding = binding;
  }

  async get(name, options) {
    if (Array.isArray(name)) return this.#getBulk(name, options, false);
    return (await this.#getOne(name, options)).value;
  }

  async getWithMetadata(name, options) {
    if (Array.isArray(name)) return this.#getBulk(name, options, true);
    return this.#getOne(name, options);
  }

  async #getOne(name, options) {
    const key = String(name);
    validateKeyName("GET", key);
    const { type = "text", cacheTtl } = getOptions(options);
    validateCacheTtl("GET", cacheTtl);
    const entry = await kvGet(this.#binding, key);
    if (entry == null) return { value: null, metadata: null, cacheStatus: null };
    const value = decode(entry.value, type);
    return { value, metadata: parseMetadata(entry.metadata), cacheStatus: null };
  }

  async #getBulk(names, options, withMetadata) {
    const keys = names.map(String);
    const { type, cacheTtl } = getOptions(options);
    if (type !== undefined && type !== "" && type !== "text" && type !== "json") {
      throw failure("GET_BULK", 400, `"${type}" is not a valid type. Use "json" or "text"`);
    }
    if (keys.length > MAX_BULK_KEYS) throw failure("GET_BULK", 400, `You can request a maximum of ${MAX_BULK_KEYS} keys`);
    if (keys.length < 1) throw failure("GET_BULK", 400, "You must request a minimum of 1 key");
    for (const key of keys) {
      validateBulkKey(key);
      validateCacheTtl("GET_BULK", cacheTtl);
    }
    const entries = await kvGetMany(this.#binding, keys);
    const values = {};
    let size = 0;
    keys.forEach((key, index) => {
      const result = bulkValue(entries[index], type, withMetadata);
      size += result.size;
      values[key] = result.value;
    });
    if (size > MAX_VALUE_LENGTH) throw failure("GET_BULK", 413, "Total size of request exceeds the limit of 25MB");
    return new Map(Object.keys(values).filter(key => !isArrayIndex(key)).map(key => [key, values[key]]));
  }

  async put(name, body, options) {
    const key = String(name);
    validateKeyName("PUT", key);
    const settings = options ?? {};
    const metadata = settings.metadata === undefined || settings.metadata === null ? null : JSON.stringify(settings.metadata);
    const { bytes, length } = await readValue(body);
    const expires = expiration(settings, Math.floor(Date.now() / 1000));
    const metadataLength = metadata === null ? 0 : byteLength(metadata);
    if (metadataLength > MAX_METADATA_LENGTH) {
      throw failure("PUT", 413, `Metadata length of ${metadataLength} exceeds limit of ${MAX_METADATA_LENGTH}.`);
    }
    if (length > MAX_VALUE_LENGTH) throw failure("PUT", 413, `Value length of ${length} exceeds limit of ${MAX_VALUE_LENGTH}.`);
    await kvPut(this.#binding, key, bytes, expires, metadata);
  }

  async delete(name) {
    const key = String(name);
    validateKeyName("DELETE", key);
    await kvDelete(this.#binding, key);
  }

  async list(options) {
    const limit = options?.limit === undefined ? 0 : integer(options.limit);
    const prefix = options?.prefix ?? null;
    const cursor = options?.cursor ?? null;
    const count = limit > 0 ? limit : MAX_LIST_KEYS;
    if (count > MAX_LIST_KEYS) {
      throw failure("GET", 400, `Invalid key_count_limit of ${count}. Please specify an integer less than ${MAX_LIST_KEYS}.`);
    }
    if (prefix !== null) validateKeyLength("GET", String(prefix));
    const page = JSON.parse(await kvList(this.#binding, prefix === null ? "" : String(prefix), cursor === null ? "" : String(cursor), count));
    const keys = page.keys.map(key => (key.metadata === undefined ? key : { ...key, metadata: JSON.parse(key.metadata) }));
    return page.cursor === undefined
      ? { keys, list_complete: true, cacheStatus: null }
      : { keys, list_complete: false, cursor: page.cursor, cacheStatus: null };
  }
}
