// Conversions of JavaScript values to strings and bytes.

// ToString, refusing a Symbol as WebIDL does.
export function string(value) {
  if (typeof value === "symbol") throw new TypeError("Cannot convert a Symbol value to a string");
  return String(value);
}

// ToString with lone surrogates replaced, as WebIDL's USVString does.
export function usvString(value) {
  return string(value).toWellFormed();
}

// The bytes of an ArrayBuffer, SharedArrayBuffer or view, without a copy;
// undefined for any other value.
export function bufferBytes(value) {
  if (ArrayBuffer.isView(value)) return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  if (value instanceof ArrayBuffer || value instanceof SharedArrayBuffer) return new Uint8Array(value);
}

// The bytes of an [AllowShared] BufferSource, none once detached; undefined
// for any other value.
export function sharedBufferSourceBytes(value) {
  const buffer = ArrayBuffer.isView(value) ? value.buffer : value;
  if (buffer instanceof ArrayBuffer && buffer.detached) return new Uint8Array();
  return bufferBytes(value);
}

// The bytes of a BufferSource, which a SharedArrayBuffer itself is not.
export function bufferSourceBytes(value) {
  if (!(value instanceof SharedArrayBuffer)) return sharedBufferSourceBytes(value);
}

// The bytes of `chunks`, joined.
export function joinBytes(chunks) {
  const joined = new Uint8Array(chunks.reduce((length, chunk) => length + chunk.byteLength, 0));
  let offset = 0;
  for (const chunk of chunks) {
    joined.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return joined;
}
