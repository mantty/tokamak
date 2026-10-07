import { kvDelete, kvGet, kvGetMany, kvList, kvPut } from "tokamak:storage";
import { joinBytes, sharedBufferSourceBytes } from "../globals/conversions.mjs";
import { bodyStream } from "../network/fetch.mjs";
import { TextDecoder, TextEncoder } from "../streams/text.mjs";
import { ReadableStream, drainedChunkBytes } from "../streams/web.mjs";

const RESPONSE_TYPES = "Possible types are \"text\", \"arrayBuffer\", \"json\", and \"stream\".";
const VALUE_TYPES = "KV put() accepts only strings, ArrayBuffers, ArrayBufferViews, and ReadableStreams as values.";

// A request the KV service refused, as workerd reports its status and message.
function failure(method, refusal) {
  return new Error(`KV ${method} failed: ${refusal}`);
}

// The result of the store's `work` for a `method` request, which fails as the service does.
async function service(method, work) {
  try { return await work(); }
  catch (error) { throw failure(method, error.message); }
}

// An option the binding takes as a 32-bit integer.
function integer(value) {
  return value === undefined ? undefined : Math.trunc(Number(value)) | 0;
}

function validateKeyName(name) {
  if (name === "") throw new TypeError("Key name cannot be empty.");
  if (name === ".") throw new TypeError("\".\" is not allowed as a key name.");
  if (name === "..") throw new TypeError("\"..\" is not allowed as a key name.");
}

function getOptions(options) {
  if (typeof options === "string") return { type: options, cacheTtl: undefined };
  if (options == null) return { type: undefined, cacheTtl: undefined };
  return { type: options.type === undefined ? undefined : String(options.type), cacheTtl: integer(options.cacheTtl) };
}

function decode(bytes, type) {
  switch (type) {
    case "text": return new TextDecoder().decode(bytes);
    case "json": return JSON.parse(new TextDecoder().decode(bytes));
    case "arrayBuffer": return bytes.buffer;
    case "stream": return bodyStream(bytes);
    default: throw new TypeError(`Unknown response type. ${RESPONSE_TYPES}`);
  }
}

// A copy of a value's bytes.
async function readValue(body) {
  if (body instanceof ReadableStream) return joinBytes(await Array.fromAsync(body, drainedChunkBytes));
  if (typeof body !== "object" || body === null) return new TextEncoder().encode(String(body));
  const bytes = sharedBufferSourceBytes(body);
  if (!bytes) throw new TypeError(VALUE_TYPES);
  return bytes.slice();
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
  if (entry == null) return null;
  let value = new TextDecoder().decode(entry.value);
  if (type === "json") {
    try { value = JSON.parse(value); } catch { throw failure("GET_BULK", "400 At least one of the requested keys corresponds to a non-json value"); }
  }
  return withMetadata ? { value, metadata: parseMetadata(entry.metadata) } : value;
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
    validateKeyName(key);
    const { type = "text", cacheTtl } = getOptions(options);
    const entry = await service("GET", () => kvGet(this.#binding, key, cacheTtl));
    if (entry == null) return { value: null, metadata: null, cacheStatus: null };
    const value = decode(entry.value, type);
    return { value, metadata: parseMetadata(entry.metadata), cacheStatus: null };
  }

  async #getBulk(names, options, withMetadata) {
    const keys = names.map(String);
    const { type, cacheTtl } = getOptions(options);
    if (type !== undefined && type !== "" && type !== "text" && type !== "json") {
      throw failure("GET_BULK", `400 "${type}" is not a valid type. Use "json" or "text"`);
    }
    const entries = await service("GET_BULK", () => kvGetMany(this.#binding, keys, cacheTtl));
    const values = {};
    keys.forEach((key, index) => { values[key] = bulkValue(entries[index], type, withMetadata); });
    return new Map(Object.keys(values).filter(key => !isArrayIndex(key)).map(key => [key, values[key]]));
  }

  async put(name, body, options) {
    const key = String(name);
    validateKeyName(key);
    const { expirationTtl, expiration, metadata } = options ?? {};
    const json = metadata == null ? undefined : JSON.stringify(metadata);
    const value = await readValue(body);
    const settings = JSON.stringify({ expirationTtl: integer(expirationTtl), expiration: integer(expiration), metadata: json });
    await service("PUT", () => kvPut(this.#binding, key, value, settings));
  }

  async delete(name) {
    const key = String(name);
    validateKeyName(key);
    await service("DELETE", () => kvDelete(this.#binding, key));
  }

  async list(options) {
    const limit = integer(options?.limit) ?? 0;
    const prefix = options?.prefix == null ? "" : String(options.prefix);
    const cursor = options?.cursor == null ? "" : String(options.cursor);
    const page = JSON.parse(await service("GET", () => kvList(this.#binding, prefix, cursor, limit)));
    const keys = page.keys.map(key => (key.metadata === undefined ? key : { ...key, metadata: JSON.parse(key.metadata) }));
    return page.cursor === undefined
      ? { keys, list_complete: true, cacheStatus: null }
      : { keys, list_complete: false, cursor: page.cursor, cacheStatus: null };
  }
}
