import type { MessageHeader } from "./contracts.generated.js";

export type { MessageHeader };

const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true, ignoreBOM: false });

/** The global through which the native host names its platform before the Worker's modules load. */
export const platformGlobal = "__tokamak_platform";

/**
 * A message's header: a `MessageHeader` on the device's link, or one without an `id` on the
 * development Worker's link to the dev socket, where each request is one call.
 */
export type RoutingHeader = Omit<MessageHeader, "id"> & Partial<Pick<MessageHeader, "id">>;

export interface NativeMessage {
  header: RoutingHeader;
  payload: Uint8Array;
}

/** A failed endpoint payload. */
export interface NativeFailure {
  name: string;
  message: string;
  cause?: unknown;
}

/** A length-delimited routing header followed by an opaque endpoint payload. */
export function encodeMessage(header: RoutingHeader, payload: Uint8Array = new Uint8Array()): Uint8Array<ArrayBuffer> {
  const metadata = encoder.encode(JSON.stringify(header));
  const length = 4 + metadata.byteLength + payload.byteLength;
  if (length > 0xffffffff) throw new RangeError("Native message exceeds 4 GiB");
  const result = new Uint8Array(4 + length);
  const view = new DataView(result.buffer);
  view.setUint32(0, length, true);
  view.setUint32(4, metadata.byteLength, true);
  result.set(metadata, 8);
  result.set(payload, 8 + metadata.byteLength);
  return result;
}

export function decodeMessage(bytes: Uint8Array): NativeMessage {
  if (bytes.byteLength < 8) throw new TypeError("Incomplete native message");
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const length = view.getUint32(0, true);
  const metadata = view.getUint32(4, true);
  if (length !== bytes.byteLength - 4 || metadata > length - 4) {
    throw new TypeError("Invalid native message length");
  }
  const header: unknown = JSON.parse(decoder.decode(bytes.subarray(8, 8 + metadata)));
  if (!isRoutingHeader(header)) throw new TypeError("Invalid native routing header");
  return { header, payload: bytes.subarray(8 + metadata) };
}

/** Whether `header`, parsed from JSON, has a routing header's fields. */
function isRoutingHeader(header: unknown): header is RoutingHeader {
  if (header === null || typeof header !== "object" || Array.isArray(header)) return false;
  const fields = header as Record<string, unknown>;
  const { id } = fields;
  return typeof fields.type === "string" &&
    (id === undefined || (typeof id === "number" && Number.isSafeInteger(id) && id >= 0)) &&
    ["plugin", "method", "name", "overflow"].every(key => fields[key] === undefined || typeof fields[key] === "string") &&
    ["failed", "done"].every(key => fields[key] === undefined || typeof fields[key] === "boolean");
}

/** Reads messages independently of HTTP chunk boundaries. */
export async function* messages(body: AsyncIterable<Uint8Array> | Iterable<Uint8Array> | null): AsyncGenerator<NativeMessage> {
  const reader = new MessageReader();
  for await (const chunk of body ?? []) yield* reader.read(chunk);
  if (reader.started) throw new TypeError("Incomplete native message");
}

/** The messages in a sequence of chunks, however the chunks split them. */
class MessageReader {
  #prefix = new Uint8Array(4);
  #used = 0;
  #remaining = 0;
  #parts: Uint8Array[] = [];

  /** Whether a message has begun and not ended. */
  get started(): boolean {
    return this.#used > 0;
  }

  /** The messages that `chunk`, the next chunk, ends. */
  *read(chunk: Uint8Array): Generator<NativeMessage> {
    let offset = 0;
    while (offset < chunk.byteLength) {
      offset = this.#readPrefix(chunk, offset);
      const count = Math.min(this.#remaining, chunk.byteLength - offset);
      if (count === 0) continue;
      this.#parts.push(chunk.subarray(offset, offset + count));
      this.#remaining -= count;
      offset += count;
      if (this.#remaining === 0) yield this.#message();
    }
  }

  /** Reads the rest of the length prefix from `chunk` at `offset`, and returns the offset after it. */
  #readPrefix(chunk: Uint8Array, offset: number): number {
    if (this.#used === 4) return offset;
    const count = Math.min(4 - this.#used, chunk.byteLength - offset);
    this.#prefix.set(chunk.subarray(offset, offset + count), this.#used);
    this.#used += count;
    if (this.#used === 4) {
      this.#remaining = new DataView(this.#prefix.buffer).getUint32(0, true);
      if (this.#remaining < 4) throw new TypeError("Invalid native message length");
      this.#parts.push(this.#prefix);
    }
    return offset + count;
  }

  /** The message the parts read make up; the next part starts another. */
  #message(): NativeMessage {
    const frame = new Uint8Array(this.#parts.reduce((length, part) => length + part.byteLength, 0));
    let position = 0;
    for (const part of this.#parts) {
      frame.set(part, position);
      position += part.byteLength;
    }
    this.#prefix = new Uint8Array(4);
    this.#used = 0;
    this.#parts = [];
    return decodeMessage(frame);
  }
}

/** The error a failed endpoint payload describes, with its details as the `cause`. */
export function nativeError({ name, message, cause }: NativeFailure): DOMException {
  const error = new DOMException(message, name);
  if (cause !== undefined) Object.defineProperty(error, "cause", { value: cause, writable: true, configurable: true });
  return error;
}
