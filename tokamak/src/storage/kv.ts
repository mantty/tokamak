import { kvDelete, kvGet, kvGetMany, kvList, kvPut, kvValue, kvWrite, type KvEntry, type ValueWriter } from "tokamak:storage";
import { sharedBufferSourceBytes } from "../globals/conversions.ts";
import { propertyOf } from "../globals/objects.ts";
import { bodyStream } from "../network/fetch.ts";
import { TextDecoder, TextEncoder } from "../streams/text.ts";
import { ReadableStream, drainedChunkBytes } from "../streams/web.ts";

const RESPONSE_TYPES = "Possible types are \"text\", \"arrayBuffer\", \"json\", and \"stream\".";
const VALUE_TYPES = "KV put() accepts only strings, ArrayBuffers, ArrayBufferViews, and ReadableStreams as values.";

// A request the KV service refused, as workerd reports its status and message.
function failure(method: string, refusal: string): Error {
  return new Error(`KV ${method} failed: ${refusal}`);
}

// The result of the store's `work` for a `method` request, which fails as the service does.
async function service<T>(method: string, work: () => Promise<T>): Promise<T> {
  try { return await work(); }
  catch (error) { throw failure(method, `${propertyOf(error, "message")}`); }
}

// An option the binding takes as a 32-bit integer.
function integer(value: unknown): number | undefined {
  return value === undefined ? undefined : Math.trunc(Number(value)) | 0;
}

// A string option, empty when absent.
function optionalString(value: unknown): string {
  return value == null ? "" : String(value);
}

function validateKeyName(name: string): void {
  if (name === "") throw new TypeError("Key name cannot be empty.");
  if (name === ".") throw new TypeError("\".\" is not allowed as a key name.");
  if (name === "..") throw new TypeError("\"..\" is not allowed as a key name.");
}

function getOptions(options: unknown): { type: string | undefined; cacheTtl: number | undefined } {
  if (typeof options === "string") return { type: options, cacheTtl: undefined };
  if (options == null) return { type: undefined, cacheTtl: undefined };
  const type = propertyOf(options, "type");
  return { type: type === undefined ? undefined : String(type), cacheTtl: integer(propertyOf(options, "cacheTtl")) };
}

function decode(bytes: Uint8Array, type: string): unknown {
  switch (type) {
    case "text": return new TextDecoder().decode(bytes);
    case "json": return JSON.parse(new TextDecoder().decode(bytes));
    case "arrayBuffer": return bytes.buffer;
    case "stream": return bodyStream(bytes);
    default: throw new TypeError(`Unknown response type. ${RESPONSE_TYPES}`);
  }
}

// The store's value for `body`, written as the body is read.
async function readValue(body: unknown): Promise<ValueWriter> {
  const value = kvValue();
  if (body instanceof ReadableStream) {
    for await (const chunk of body) kvWrite(value, drainedChunkBytes(chunk));
    return value;
  }
  const bytes = typeof body !== "object" || body === null ? new TextEncoder().encode(String(body)) : sharedBufferSourceBytes(body);
  if (!bytes) throw new TypeError(VALUE_TYPES);
  kvWrite(value, bytes);
  return value;
}

// Whether a property name is an array index, which workerd leaves out of bulk results.
function isArrayIndex(name: string): boolean {
  const index = Number(name);
  return String(index >>> 0) === name && index !== 2 ** 32 - 1;
}

function parseMetadata(metadata: string | undefined): unknown {
  return metadata == null ? null : JSON.parse(metadata);
}

function bulkValue(entry: KvEntry | undefined, type: string | undefined, withMetadata: boolean): unknown {
  if (entry == null) return null;
  const text = new TextDecoder().decode(entry.value);
  let value: unknown = text;
  if (type === "json") {
    try { value = JSON.parse(text); } catch { throw failure("GET_BULK", "400 At least one of the requested keys corresponds to a non-json value"); }
  }
  return withMetadata ? { value, metadata: parseMetadata(entry.metadata) } : value;
}

export class KVNamespace {
  #binding: string;

  constructor(binding: string) {
    this.#binding = binding;
  }

  async get(name: unknown, options?: unknown): Promise<unknown> {
    if (Array.isArray(name)) return this.#getBulk(name, options, false);
    return (await this.#getOne(name, options)).value;
  }

  async getWithMetadata(name: unknown, options?: unknown): Promise<unknown> {
    if (Array.isArray(name)) return this.#getBulk(name, options, true);
    return this.#getOne(name, options);
  }

  async #getOne(name: unknown, options: unknown): Promise<{ value: unknown; metadata: unknown; cacheStatus: null }> {
    const key = String(name);
    validateKeyName(key);
    const { type = "text", cacheTtl } = getOptions(options);
    const entry = await service("GET", () => kvGet(this.#binding, key, cacheTtl));
    if (entry == null) return { value: null, metadata: null, cacheStatus: null };
    const value = decode(entry.value, type);
    return { value, metadata: parseMetadata(entry.metadata), cacheStatus: null };
  }

  async #getBulk(names: unknown[], options: unknown, withMetadata: boolean): Promise<Map<string, unknown>> {
    const keys = names.map(String);
    const { type, cacheTtl } = getOptions(options);
    if (type !== undefined && type !== "" && type !== "text" && type !== "json") {
      throw failure("GET_BULK", `400 "${type}" is not a valid type. Use "json" or "text"`);
    }
    const entries = await service("GET_BULK", () => kvGetMany(this.#binding, keys, cacheTtl));
    const values: Record<string, unknown> = {};
    keys.forEach((key, index) => { values[key] = bulkValue(entries[index], type, withMetadata); });
    return new Map(Object.keys(values).filter(key => !isArrayIndex(key)).map(key => [key, values[key]]));
  }

  async put(name: unknown, body: unknown, options?: unknown): Promise<void> {
    const key = String(name);
    validateKeyName(key);
    const expirationTtl = propertyOf(options, "expirationTtl");
    const expiration = propertyOf(options, "expiration");
    const metadata = propertyOf(options, "metadata");
    const json = metadata == null ? undefined : JSON.stringify(metadata);
    const value = await readValue(body);
    const settings = { expirationTtl: integer(expirationTtl), expiration: integer(expiration), metadata: json };
    await service("PUT", () => kvPut(this.#binding, key, value, settings));
  }

  async delete(name: unknown): Promise<void> {
    const key = String(name);
    validateKeyName(key);
    await service("DELETE", () => kvDelete(this.#binding, key));
  }

  async list(options?: unknown): Promise<unknown> {
    const limit = integer(propertyOf(options, "limit")) ?? 0;
    const prefix = optionalString(propertyOf(options, "prefix"));
    const cursor = optionalString(propertyOf(options, "cursor"));
    const page = await service("GET", () => kvList(this.#binding, prefix, cursor, limit));
    const keys = page.keys.map(key => (key.metadata === undefined ? key : { ...key, metadata: JSON.parse(key.metadata) }));
    return page.cursor === undefined
      ? { keys, list_complete: true, cacheStatus: null }
      : { keys, list_complete: false, cursor: page.cursor, cacheStatus: null };
  }
}
