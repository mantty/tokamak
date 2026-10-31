// Conversions of JavaScript values to strings and bytes.

// workerd's TypeError for an argument of the wrong type.
export function parameterTypeError(method: string, owner: string, index: number, type: string): TypeError {
  return new TypeError(`Failed to execute '${method}' on '${owner}': parameter ${index} is not of type '${type}'.`);
}

// workerd's TypeError for a dictionary member of the wrong type.
export function fieldTypeError(field: string, struct: string, type: string): TypeError {
  return new TypeError(`Incorrect type for the '${field}' field on '${struct}': the provided value is not of type '${type}'.`);
}

// ToNumber, which refuses a BigInt where Number() would convert one.
export function toNumber(value: unknown): number {
  if (typeof value === "bigint") throw new TypeError("Cannot convert a BigInt value to a number");
  return Number(value);
}

// ToString, refusing a Symbol as WebIDL does.
export function string(value: unknown): string {
  if (typeof value === "symbol") throw new TypeError("Cannot convert a Symbol value to a string");
  return String(value);
}

// ToPropertyKey: a symbol itself, any other value's string.
export function propertyKey(value: unknown): PropertyKey {
  return typeof value === "symbol" ? value : String(value);
}

// ToString with lone surrogates replaced, as WebIDL's USVString does.
export function usvString(value: unknown): string {
  return string(value).toWellFormed();
}

// The code points of `text`'s characters, each of which has one.
export function codePoints(text: string): number[] {
  return Array.from(text, character => character.codePointAt(0) ?? 0);
}

// The bytes a view covers, without a copy.
export function viewBytes(view: ArrayBufferView): Uint8Array {
  return new Uint8Array(view.buffer, view.byteOffset, view.byteLength);
}

// The bytes of an ArrayBuffer, SharedArrayBuffer or view, without a copy;
// undefined for any other value.
export function bufferBytes(value: unknown): Uint8Array | undefined {
  if (ArrayBuffer.isView(value)) return viewBytes(value);
  if (value instanceof ArrayBuffer || value instanceof SharedArrayBuffer) return new Uint8Array(value);
}

// The bytes of an [AllowShared] BufferSource, none once detached; undefined
// for any other value.
export function sharedBufferSourceBytes(value: unknown): Uint8Array | undefined {
  const buffer = ArrayBuffer.isView(value) ? value.buffer : value;
  if (buffer instanceof ArrayBuffer && buffer.detached) return new Uint8Array();
  return bufferBytes(value);
}

// The bytes of a BufferSource, which a SharedArrayBuffer itself is not.
export function bufferSourceBytes(value: unknown): Uint8Array | undefined {
  if (!(value instanceof SharedArrayBuffer)) return sharedBufferSourceBytes(value);
}

// The bytes of `chunks`, joined.
export function joinBytes(chunks: readonly Uint8Array[]): Uint8Array<ArrayBuffer> {
  const joined = new Uint8Array(chunks.reduce((length, chunk) => length + chunk.byteLength, 0));
  let offset = 0;
  for (const chunk of chunks) {
    joined.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return joined;
}
