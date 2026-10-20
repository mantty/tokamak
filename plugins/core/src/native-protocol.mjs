const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true });

/** A length-delimited routing header followed by an opaque endpoint payload. */
export function encodeMessage(header, payload = new Uint8Array()) {
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

export function decodeMessage(bytes) {
  if (bytes.byteLength < 8) throw new TypeError("Incomplete native message");
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const length = view.getUint32(0, true);
  const metadata = view.getUint32(4, true);
  if (length !== bytes.byteLength - 4 || metadata > length - 4) {
    throw new TypeError("Invalid native message length");
  }
  const header = JSON.parse(decoder.decode(bytes.subarray(8, 8 + metadata)));
  if (header === null || typeof header !== "object" || Array.isArray(header) || typeof header.type !== "string" ||
      (header.id !== undefined && (!Number.isSafeInteger(header.id) || header.id < 0)) ||
      ["plugin", "method", "name", "overflow"].some(key => header[key] !== undefined && typeof header[key] !== "string") ||
      ["failed", "done"].some(key => header[key] !== undefined && typeof header[key] !== "boolean")) {
    throw new TypeError("Invalid native routing header");
  }
  return { header, payload: bytes.subarray(8 + metadata) };
}

/** Reads messages independently of HTTP chunk boundaries. */
export async function* messages(body) {
  let prefix = new Uint8Array(4);
  let used = 0;
  let remaining = 0;
  let parts = [];
  for await (const chunk of body ?? []) {
    let offset = 0;
    while (offset < chunk.byteLength) {
      if (used < 4) {
        const count = Math.min(4 - used, chunk.byteLength - offset);
        prefix.set(chunk.subarray(offset, offset + count), used);
        used += count;
        offset += count;
        if (used < 4) continue;
        remaining = new DataView(prefix.buffer).getUint32(0, true);
        if (remaining < 4) throw new TypeError("Invalid native message length");
        parts.push(prefix);
      }
      const count = Math.min(remaining, chunk.byteLength - offset);
      parts.push(chunk.subarray(offset, offset + count));
      remaining -= count;
      offset += count;
      if (remaining) continue;
      const frame = new Uint8Array(parts.reduce((length, part) => length + part.byteLength, 0));
      let position = 0;
      for (const part of parts) { frame.set(part, position); position += part.byteLength; }
      yield decodeMessage(frame);
      prefix = new Uint8Array(4);
      used = 0;
      parts = [];
    }
  }
  if (used) throw new TypeError("Incomplete native message");
}
