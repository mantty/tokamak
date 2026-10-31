import { atob as globalAtob, btoa as globalBtoa } from "../globals/base64.ts";
import { bufferBytes } from "../globals/conversions.ts";
import { isObject, isTypedArray, objectOf, type TypedArray } from "../globals/objects.ts";
import { Blob, File } from "../network/fetch.ts";
import { TextDecoder, TextEncoder } from "../streams/text.ts";
import { codedError } from "./errors.ts";
import { invalidArgType } from "./validators.ts";

type DataViewSetter = "setInt8" | "setUint8" | "setInt16" | "setUint16" | "setInt32" | "setUint32" | "setFloat32" | "setFloat64";

const encodingNames = new Set(["utf8", "utf-8", "ascii", "latin1", "binary", "base64", "base64url", "hex", "ucs2", "ucs-2", "utf16le", "utf-16le"]);

function normalizeEncoding(encoding: unknown): string {
  if (encoding === undefined || encoding === null) return "utf8";
  const name = String(encoding).toLowerCase();
  if (!encodingNames.has(name)) throw unknownEncoding(name);
  return name;
}

function unknownEncoding(name: string): Error {
  return codedError(TypeError, `Unknown encoding: ${name}`, "ERR_UNKNOWN_ENCODING");
}

function isArrayBufferLike(value: unknown): value is ArrayBufferLike {
  return value instanceof ArrayBuffer || value instanceof SharedArrayBuffer;
}

// Whether `value` is an array or has a numeric length, as Node reads an
// array-like object's bytes.
function isArrayLike(value: unknown): value is ArrayLike<unknown> {
  return Array.isArray(value) || (Boolean(value) && typeof objectOf(Object(value)).length === "number");
}

// ToNumber, which refuses a BigInt as assigning a typed array element does.
function toNumber(value: unknown): number {
  if (typeof value === "bigint") throw new TypeError("Cannot convert a BigInt value to a number");
  return Number(value);
}

// A copy of an array-like object's elements, each converted to a byte.
function arrayLikeBytes(value: ArrayLike<unknown>): Uint8Array {
  return Uint8Array.from(value, toNumber);
}

function hexByte(value: number): string { return value.toString(16).padStart(2, "0"); }

function stringBytes(value: unknown, encoding: unknown): Uint8Array {
  const name = normalizeEncoding(encoding);
  if (name === "hex") return decodeHex(String(value));
  if (name === "base64" || name === "base64url") return decodeBase64Leniently(String(value));
  if (name === "ascii" || name === "latin1" || name === "binary") {
    const input = String(value);
    return Uint8Array.from({ length: input.length }, (_, index) => input.charCodeAt(index));
  }
  if (name === "ucs2" || name === "ucs-2" || name === "utf16le" || name === "utf-16le") {
    const input = String(value);
    const bytes = new Uint8Array(input.length * 2);
    for (let index = 0; index < input.length; index += 1) {
      const code = input.charCodeAt(index);
      bytes[index * 2] = code & 0xff;
      bytes[index * 2 + 1] = code >> 8;
    }
    return bytes;
  }
  return new TextEncoder().encode(String(value));
}

// Node decodes hex pairs up to the first that is not one.
function decodeHex(value: string): Uint8Array {
  const length = value.search(/[^0-9a-f]|$/i);
  return Uint8Array.fromHex(value.slice(0, length - (length % 2)));
}

// Node decodes either base64 alphabet up to the first "=", skipping other characters.
function decodeBase64Leniently(value: string): Uint8Array {
  const digits = value.split("=", 1)[0].replace(/[^A-Za-z0-9+/_-]/g, "").replace(/-/g, "+").replace(/_/g, "/");
  return Uint8Array.fromBase64(digits.length % 4 === 1 ? digits.slice(0, -1) : digits);
}

function decode(bytes: Uint8Array, encoding: unknown): string {
  const name = normalizeEncoding(encoding);
  if (name === "hex") return bytes.toHex();
  if (name === "base64") return bytes.toBase64();
  if (name === "base64url") return bytes.toBase64({ alphabet: "base64url", omitPadding: true });
  if (name === "ascii") return Array.from(bytes, byte => String.fromCharCode(byte & 0x7f)).join("");
  if (name === "latin1" || name === "binary") return Array.from(bytes, byte => String.fromCharCode(byte)).join("");
  if (name === "ucs2" || name === "ucs-2" || name === "utf16le" || name === "utf-16le") return utf16Units(bytes);
  return new TextDecoder("utf-8", { ignoreBOM: true }).decode(bytes);
}

// UTF-16LE code units, unpaired surrogates kept and an odd last byte left out.
function utf16Units(bytes: Uint8Array): string {
  let text = "";
  for (let index = 0; index + 1 < bytes.length; index += 2) text += String.fromCharCode(bytes[index] | (bytes[index + 1] << 8));
  return text;
}

function integer(value: unknown, fallback = 0): number {
  const number = Number(value);
  if (Number.isNaN(number) || number === 0) return fallback;
  return number < 0 ? Math.ceil(number) : Math.floor(number);
}

function index(value: unknown, length: number): number {
  const number = integer(value);
  if (number < 0) return Math.max(length + number, 0);
  return Math.min(number, length);
}

// The view of `bytes` from `start` to `end`, which may be any values, converted as `subarray` converts them.
function view(bytes: Uint8Array, start: unknown = 0, end: unknown = bytes.length): Uint8Array {
  return bytes.subarray(index(start, bytes.length), index(end, bytes.length));
}

function rangeError(name: string, value: unknown, min: number | bigint, max: number | bigint): Error {
  return codedError(RangeError, `The value of "${name}" is out of range. It must be >= ${min} && <= ${max}. Received ${value}`, "ERR_OUT_OF_RANGE");
}

function checkOffset(buffer: Uint8Array, offset: number, size: number): number {
  if (!Number.isInteger(offset) || offset < 0 || offset + size > buffer.length) throw rangeError("offset", offset, 0, buffer.length - size);
  return offset;
}

function checkByteLength(size: unknown): asserts size is number {
  if (typeof size !== "number" || !Number.isInteger(size) || size < 1 || size > 6) throw new RangeError("byteLength must be between 1 and 6");
}

function fillBytes<T extends Uint8Array>(buffer: T, value: unknown, start: number, end: number, encoding: unknown): T {
  const first = pattern(value, encoding);
  if (first.length === 0) return buffer;
  for (let index = start; index < end; index += 1) buffer[index] = first[(index - start) % first.length];
  return buffer;
}

// A copy of a typed array's elements as bytes, which a BigInt array's cannot become.
function elementBytes(array: TypedArray): Uint8Array {
  if (array instanceof BigInt64Array || array instanceof BigUint64Array) throw new TypeError("Cannot convert a BigInt value to a number");
  return Uint8Array.from(array);
}

function toBytes(value: unknown, allowString = true, encoding?: unknown): Uint8Array {
  if (allowString && typeof value === "string") return stringBytes(value, encoding);
  if (value instanceof Buffer) return value;
  if (isTypedArray(value)) return elementBytes(value);
  const bytes = bufferBytes(value);
  if (bytes) return bytes;
  if (isArrayLike(value)) return arrayLikeBytes(value);
  throw new TypeError("The value must be a string, Buffer, TypedArray, DataView, ArrayBuffer, or Array-like Object");
}

function pattern(value: unknown, encoding: unknown): Uint8Array {
  if (typeof value === "number") return Uint8Array.of(value & 0xff);
  return toBytes(value, true, encoding);
}

// workerd's `fromArrayBuffer`.
function fromArrayBuffer(buffer: ArrayBufferLike, byteOffset: unknown, length: unknown): Buffer {
  let offset = byteOffset === undefined ? 0 : toNumber(byteOffset);
  if (Number.isNaN(offset)) offset = 0;
  const maxLength = buffer.byteLength - offset;
  if (maxLength < 0) throw outOfBufferBounds("offset");
  let size = maxLength;
  if (length !== undefined) {
    size = toNumber(length);
    if (!(size > 0)) size = 0;
    else if (size > maxLength) throw outOfBufferBounds("length");
  }
  return new Buffer(buffer, offset, size);
}

function outOfBufferBounds(name: string): Error {
  return codedError(RangeError, `"${name}" is outside of buffer bounds`, "ERR_BUFFER_OUT_OF_BOUNDS");
}

// Uint8Array's constructor without its static methods, which Buffer replaces
// with Node's. Its arrays are typed as over an ArrayBuffer, though a Buffer
// can be over a SharedArrayBuffer, because Buffer's `slice` returns a view
// where Uint8Array's returns a copy in a new ArrayBuffer.
const ByteArray: {
  new (length: number): Uint8Array<ArrayBuffer>;
  new (bytes: ArrayLike<number> | ArrayBufferLike, byteOffset?: number, length?: number): Uint8Array<ArrayBuffer>;
} = Uint8Array;

export class Buffer extends ByteArray {
  declare static poolSize: number;

  static from(value: unknown, encodingOrOffset?: unknown, length?: unknown): Buffer {
    // Like workerd's, a string with an encoding that is not a non-empty string is utf8.
    if (typeof value === "string") return new Buffer(stringBytes(value, typeof encodingOrOffset === "string" && encodingOrOffset !== "" ? encodingOrOffset : "utf8"));
    if (isObject(value) && value.type === "Buffer" && Array.isArray(value.data)) return new Buffer(arrayLikeBytes(value.data));
    if (isArrayBufferLike(value)) return fromArrayBuffer(value, encodingOrOffset, length);
    if (ArrayBuffer.isView(value)) return isTypedArray(value) ? new Buffer(elementBytes(value)) : new Buffer(value.buffer, value.byteOffset, value.byteLength);
    if (isArrayLike(value)) return new Buffer(arrayLikeBytes(value));
    throw invalidArgType("first argument", ["string", "Buffer", "TypedArray", "ArrayBuffer", "SharedArrayBuffer", "Array", "Array-like Object"], value);
  }

  static alloc(size: unknown, fill: unknown = 0, encoding?: unknown): Buffer {
    const buffer = new Buffer(normalizeSize(size));
    return fill === 0 || fill === undefined ? buffer : buffer.fill(fill, 0, buffer.length, encoding);
  }

  static allocUnsafe(size: unknown): Buffer { return new Buffer(normalizeSize(size)); }
  static allocUnsafeSlow(size: unknown): Buffer { return Buffer.allocUnsafe(size); }

  static byteLength(value: unknown, encoding?: unknown): number {
    // Node estimates a base64 string's length from its padding.
    if (typeof value === "string" && ["base64", "base64url"].includes(normalizeEncoding(encoding))) return Math.floor(value.replace(/={1,2}$/, "").length * 3 / 4);
    if (typeof value === "string") return stringBytes(value, encoding).byteLength;
    if (isArrayBufferLike(value) || ArrayBuffer.isView(value)) return value.byteLength;
    return stringBytes(String(value), encoding).byteLength;
  }

  static compare(left: unknown, right: unknown): number {
    const a = toBytes(left, false);
    const b = toBytes(right, false);
    const length = Math.min(a.length, b.length);
    for (let index = 0; index < length; index += 1) if (a[index] !== b[index]) return a[index] < b[index] ? -1 : 1;
    return a.length === b.length ? 0 : a.length < b.length ? -1 : 1;
  }

  static concat(list: unknown, totalLength?: unknown): Buffer {
    if (!Array.isArray(list)) throw new TypeError("The first argument must be an array of Buffers or Uint8Arrays");
    const buffers = list.map((value: unknown) => toBytes(value, false));
    const length = totalLength === undefined ? buffers.reduce((sum, value) => sum + value.length, 0) : Math.max(0, integer(totalLength));
    const output = new Buffer(length);
    let offset = 0;
    for (const value of buffers) {
      if (offset >= length) break;
      const count = Math.min(value.length, length - offset);
      output.set(value.subarray(0, count), offset);
      offset += count;
    }
    return output;
  }

  static isBuffer(value: unknown): value is Buffer { return value instanceof Buffer; }
  static isEncoding(value: unknown): boolean { return typeof value === "string" && encodingNames.has(value.toLowerCase()); }
  static of(...values: unknown[]): Buffer { return new Buffer(arrayLikeBytes(values)); }

  get parent() { return this.buffer; }
  get offset() { return this.byteOffset; }

  toString(encoding: unknown = "utf8", start: unknown = 0, end: unknown = this.length): string {
    const first = index(start, this.length);
    const last = Math.max(first, index(end, this.length));
    return decode(this.subarray(first, last), encoding);
  }

  toJSON(): { type: "Buffer"; data: number[] } { return { type: "Buffer", data: [...this] }; }
  inspect(): string { return `<Buffer ${[...this].slice(0, 50).map(hexByte).join(" ")}${this.length > 50 ? " …" : ""}>`; }
  toLocaleString(encoding?: unknown, start?: unknown, end?: unknown): string { return this.toString(encoding, start, end); }
  slice(start: unknown = 0, end: unknown = this.length): Uint8Array<ArrayBuffer> { return this.subarray(index(start, this.length), index(end, this.length)); }
  equals(other: unknown): boolean { return Buffer.compare(this, other) === 0; }
  compare(other: unknown, targetStart?: unknown, targetEnd?: unknown, sourceStart?: unknown, sourceEnd?: unknown): number {
    return Buffer.compare(view(this, sourceStart, sourceEnd), view(toBytes(other, false), targetStart, targetEnd));
  }
  copy(target: unknown, targetStart: unknown = 0, sourceStart: unknown = 0, sourceEnd: unknown = this.length): number {
    if (!(target instanceof Uint8Array)) throw new TypeError("The target argument must be a Buffer or Uint8Array");
    const start = index(sourceStart, this.length);
    const end = index(sourceEnd, this.length);
    const destination = Math.max(0, integer(targetStart));
    const count = Math.min(end - start, target.length - destination);
    if (count > 0) target.set(this.subarray(start, start + count), destination);
    return Math.max(0, count);
  }
  fill(value: unknown, start: unknown = 0, end: unknown = this.length, encoding?: unknown): this {
    const first = index(start, this.length);
    const last = index(end, this.length);
    return fillBytes(this, value, first, last, encoding);
  }
  write(value: unknown, offset: unknown = 0, length?: unknown, encoding?: unknown): number {
    if (typeof offset === "string") { encoding = offset; offset = 0; length = this.length; }
    else if (typeof length === "string") { encoding = length; length = undefined; }
    if (length === undefined) length = this.length - toNumber(offset);
    const start = integer(offset);
    if (start < 0 || start > this.length) throw rangeError("offset", start, 0, this.length);
    const count = Math.max(0, Math.min(integer(length), this.length - start));
    const bytes = stringBytes(value, encoding).subarray(0, count);
    this.set(bytes, start);
    return bytes.length;
  }

  indexOf(value: unknown, byteOffset: unknown = 0, encoding?: unknown): number { return find(this, pattern(value, encoding), byteOffset, false); }
  lastIndexOf(value: unknown, byteOffset: unknown = this.length - 1, encoding?: unknown): number { return find(this, pattern(value, encoding), byteOffset, true); }
  includes(value: unknown, byteOffset: unknown = 0, encoding?: unknown): boolean { return this.indexOf(value, byteOffset, encoding) !== -1; }
  swap16(): this { return swap(this, 2); }
  swap32(): this { return swap(this, 4); }
  swap64(): this { return swap(this, 8); }

  readUInt8(offset: unknown = 0): number { return this[checkOffset(this, integer(offset), 1)]; }
  readUint8(offset: unknown = 0): number { return this.readUInt8(offset); }
  readInt8(offset: unknown = 0): number { const value = this.readUInt8(offset); return value > 127 ? value - 256 : value; }
  readUInt16LE(offset: unknown = 0): number { return dataView(this, offset, 2).getUint16(0, true); }
  readUint16LE(offset: unknown = 0): number { return this.readUInt16LE(offset); }
  readUInt16BE(offset: unknown = 0): number { return dataView(this, offset, 2).getUint16(0, false); }
  readUint16BE(offset: unknown = 0): number { return this.readUInt16BE(offset); }
  readInt16LE(offset: unknown = 0): number { return dataView(this, offset, 2).getInt16(0, true); }
  readInt16BE(offset: unknown = 0): number { return dataView(this, offset, 2).getInt16(0, false); }
  readUInt32LE(offset: unknown = 0): number { return dataView(this, offset, 4).getUint32(0, true); }
  readUint32LE(offset: unknown = 0): number { return this.readUInt32LE(offset); }
  readUInt32BE(offset: unknown = 0): number { return dataView(this, offset, 4).getUint32(0, false); }
  readUint32BE(offset: unknown = 0): number { return this.readUInt32BE(offset); }
  readInt32LE(offset: unknown = 0): number { return dataView(this, offset, 4).getInt32(0, true); }
  readInt32BE(offset: unknown = 0): number { return dataView(this, offset, 4).getInt32(0, false); }
  readUIntLE(offset: unknown, byteLength: unknown): number { return readInteger(this, offset, byteLength, false, false); }
  readUintLE(offset: unknown, byteLength: unknown): number { return this.readUIntLE(offset, byteLength); }
  readUIntBE(offset: unknown, byteLength: unknown): number { return readInteger(this, offset, byteLength, false, true); }
  readUintBE(offset: unknown, byteLength: unknown): number { return this.readUIntBE(offset, byteLength); }
  readIntLE(offset: unknown, byteLength: unknown): number { return readInteger(this, offset, byteLength, true, false); }
  readIntBE(offset: unknown, byteLength: unknown): number { return readInteger(this, offset, byteLength, true, true); }
  readFloatLE(offset: unknown = 0): number { return dataView(this, offset, 4).getFloat32(0, true); }
  readFloatBE(offset: unknown = 0): number { return dataView(this, offset, 4).getFloat32(0, false); }
  readDoubleLE(offset: unknown = 0): number { return dataView(this, offset, 8).getFloat64(0, true); }
  readDoubleBE(offset: unknown = 0): number { return dataView(this, offset, 8).getFloat64(0, false); }
  readBigUInt64LE(offset: unknown = 0): bigint { return dataView(this, offset, 8).getBigUint64(0, true); }
  readBigUint64LE(offset: unknown = 0): bigint { return this.readBigUInt64LE(offset); }
  readBigUInt64BE(offset: unknown = 0): bigint { return dataView(this, offset, 8).getBigUint64(0, false); }
  readBigUint64BE(offset: unknown = 0): bigint { return this.readBigUInt64BE(offset); }
  readBigInt64LE(offset: unknown = 0): bigint { return dataView(this, offset, 8).getBigInt64(0, true); }
  readBigInt64BE(offset: unknown = 0): bigint { return dataView(this, offset, 8).getBigInt64(0, false); }

  writeUInt8(value: unknown, offset: unknown = 0): number { return writeData(this, offset, 1, value, "setUint8", undefined, 0, 255); }
  writeUint8(value: unknown, offset: unknown = 0): number { return this.writeUInt8(value, offset); }
  writeInt8(value: unknown, offset: unknown = 0): number { return writeData(this, offset, 1, value, "setInt8", undefined, -128, 127); }
  writeUInt16LE(value: unknown, offset: unknown = 0): number { return writeData(this, offset, 2, value, "setUint16", true, 0, 65535); }
  writeUint16LE(value: unknown, offset: unknown = 0): number { return this.writeUInt16LE(value, offset); }
  writeUInt16BE(value: unknown, offset: unknown = 0): number { return writeData(this, offset, 2, value, "setUint16", false, 0, 65535); }
  writeUint16BE(value: unknown, offset: unknown = 0): number { return this.writeUInt16BE(value, offset); }
  writeInt16LE(value: unknown, offset: unknown = 0): number { return writeData(this, offset, 2, value, "setInt16", true, -32768, 32767); }
  writeInt16BE(value: unknown, offset: unknown = 0): number { return writeData(this, offset, 2, value, "setInt16", false, -32768, 32767); }
  writeUInt32LE(value: unknown, offset: unknown = 0): number { return writeData(this, offset, 4, value, "setUint32", true, 0, 0xffffffff); }
  writeUint32LE(value: unknown, offset: unknown = 0): number { return this.writeUInt32LE(value, offset); }
  writeUInt32BE(value: unknown, offset: unknown = 0): number { return writeData(this, offset, 4, value, "setUint32", false, 0, 0xffffffff); }
  writeUint32BE(value: unknown, offset: unknown = 0): number { return this.writeUInt32BE(value, offset); }
  writeInt32LE(value: unknown, offset: unknown = 0): number { return writeData(this, offset, 4, value, "setInt32", true, -0x80000000, 0x7fffffff); }
  writeInt32BE(value: unknown, offset: unknown = 0): number { return writeData(this, offset, 4, value, "setInt32", false, -0x80000000, 0x7fffffff); }
  writeUIntLE(value: unknown, offset: unknown, byteLength: unknown): number { return writeInteger(this, value, offset, byteLength, false, false); }
  writeUintLE(value: unknown, offset: unknown, byteLength: unknown): number { return this.writeUIntLE(value, offset, byteLength); }
  writeUIntBE(value: unknown, offset: unknown, byteLength: unknown): number { return writeInteger(this, value, offset, byteLength, false, true); }
  writeUintBE(value: unknown, offset: unknown, byteLength: unknown): number { return this.writeUIntBE(value, offset, byteLength); }
  writeIntLE(value: unknown, offset: unknown, byteLength: unknown): number { return writeInteger(this, value, offset, byteLength, true, false); }
  writeIntBE(value: unknown, offset: unknown, byteLength: unknown): number { return writeInteger(this, value, offset, byteLength, true, true); }
  writeFloatLE(value: unknown, offset: unknown = 0): number { return writeData(this, offset, 4, value, "setFloat32", true, -Infinity, Infinity, false); }
  writeFloatBE(value: unknown, offset: unknown = 0): number { return writeData(this, offset, 4, value, "setFloat32", false, -Infinity, Infinity, false); }
  writeDoubleLE(value: unknown, offset: unknown = 0): number { return writeData(this, offset, 8, value, "setFloat64", true, -Infinity, Infinity, false); }
  writeDoubleBE(value: unknown, offset: unknown = 0): number { return writeData(this, offset, 8, value, "setFloat64", false, -Infinity, Infinity, false); }
  writeBigUInt64LE(value: unknown, offset: unknown = 0): number { return writeBig(this, value, offset, false, false); }
  writeBigUint64LE(value: unknown, offset: unknown = 0): number { return this.writeBigUInt64LE(value, offset); }
  writeBigUInt64BE(value: unknown, offset: unknown = 0): number { return writeBig(this, value, offset, false, true); }
  writeBigUint64BE(value: unknown, offset: unknown = 0): number { return this.writeBigUInt64BE(value, offset); }
  writeBigInt64LE(value: unknown, offset: unknown = 0): number { return writeBig(this, value, offset, true, false); }
  writeBigInt64BE(value: unknown, offset: unknown = 0): number { return writeBig(this, value, offset, true, true); }

  asciiSlice(start?: unknown, end?: unknown): string { return this.toString("ascii", start, end); }
  latin1Slice(start?: unknown, end?: unknown): string { return this.toString("latin1", start, end); }
  binarySlice(start?: unknown, end?: unknown): string { return this.toString("latin1", start, end); }
  base64Slice(start?: unknown, end?: unknown): string { return this.toString("base64", start, end); }
  base64urlSlice(start?: unknown, end?: unknown): string { return this.toString("base64url", start, end); }
  hexSlice(start?: unknown, end?: unknown): string { return this.toString("hex", start, end); }
  utf8Slice(start?: unknown, end?: unknown): string { return this.toString("utf8", start, end); }
  ucs2Slice(start?: unknown, end?: unknown): string { return this.toString("utf16le", start, end); }
  utf16leSlice(start?: unknown, end?: unknown): string { return this.toString("utf16le", start, end); }
  asciiWrite(value: unknown, offset?: unknown, length?: unknown): number { return this.write(value, offset, length, "ascii"); }
  latin1Write(value: unknown, offset?: unknown, length?: unknown): number { return this.write(value, offset, length, "latin1"); }
  binaryWrite(value: unknown, offset?: unknown, length?: unknown): number { return this.write(value, offset, length, "latin1"); }
  base64Write(value: unknown, offset?: unknown, length?: unknown): number { return this.write(value, offset, length, "base64"); }
  base64urlWrite(value: unknown, offset?: unknown, length?: unknown): number { return this.write(value, offset, length, "base64url"); }
  hexWrite(value: unknown, offset?: unknown, length?: unknown): number { return this.write(value, offset, length, "hex"); }
  utf8Write(value: unknown, offset?: unknown, length?: unknown): number { return this.write(value, offset, length, "utf8"); }
  ucs2Write(value: unknown, offset?: unknown, length?: unknown): number { return this.write(value, offset, length, "utf16le"); }
  utf16leWrite(value: unknown, offset?: unknown, length?: unknown): number { return this.write(value, offset, length, "utf16le"); }
}

function normalizeSize(size: unknown): number {
  const value = integer(size);
  if (value < 0 || !Number.isSafeInteger(value)) throw new RangeError(`Invalid typed array length: ${size}`);
  return value;
}

function dataView(buffer: Uint8Array, offset: unknown, size: number): DataView {
  return new DataView(buffer.buffer, buffer.byteOffset + checkOffset(buffer, integer(offset), size), size);
}

function find(buffer: Uint8Array, needle: Uint8Array, byteOffset: unknown, reverse: boolean): number {
  const start = reverse ? Math.min(index(byteOffset, buffer.length), buffer.length - needle.length) : index(byteOffset, buffer.length);
  if (needle.length === 0) return start;
  const step = reverse ? -1 : 1;
  const limit = reverse ? -1 : buffer.length - needle.length + 1;
  for (let offset = start; reverse ? offset >= 0 : offset < limit; offset += step) {
    let match = true;
    for (let index = 0; index < needle.length; index += 1) if (buffer[offset + index] !== needle[index]) { match = false; break; }
    if (match) return offset;
  }
  return -1;
}

function swap<T extends Uint8Array>(buffer: T, size: number): T {
  if (buffer.length % size !== 0) throw codedError(RangeError, `Buffer size must be a multiple of ${size * 8}-bits`, "ERR_INVALID_BUFFER_SIZE");
  for (let offset = 0; offset < buffer.length; offset += size) for (let left = 0; left < size / 2; left += 1) {
    const right = size - left - 1;
    [buffer[offset + left], buffer[offset + right]] = [buffer[offset + right], buffer[offset + left]];
  }
  return buffer;
}

function readInteger(buffer: Uint8Array, offset: unknown, byteLength: unknown, signed: boolean, reverse: boolean): number {
  checkByteLength(byteLength);
  checkOffset(buffer, integer(offset), byteLength);
  let value = 0;
  for (let index = 0; index < byteLength; index += 1) value += buffer[integer(offset) + (reverse ? byteLength - index - 1 : index)] * 2 ** (index * 8);
  if (signed && value >= 2 ** (byteLength * 8 - 1)) value -= 2 ** (byteLength * 8);
  return value;
}

function writeData(buffer: Uint8Array, offset: unknown, size: number, value: unknown, setter: DataViewSetter, littleEndian: boolean | undefined, min: number, max: number, validate = true): number {
  const position = checkOffset(buffer, integer(offset), size);
  const number = Number(value);
  if (validate && (!Number.isFinite(number) || !Number.isInteger(number) || number < min || number > max)) throw rangeError("value", value, min, max);
  const view = new DataView(buffer.buffer, buffer.byteOffset + position, size);
  if (setter === "setInt8" || setter === "setUint8") view[setter](0, number);
  else view[setter](0, number, littleEndian);
  return position + size;
}

function writeInteger(buffer: Uint8Array, value: unknown, offset: unknown, byteLength: unknown, signed: boolean, reverse: boolean): number {
  checkByteLength(byteLength);
  const bits = byteLength * 8;
  const min = signed ? -(2 ** (bits - 1)) : 0;
  const max = signed ? 2 ** (bits - 1) - 1 : 2 ** bits - 1;
  const number = Number(value);
  if (!Number.isSafeInteger(number) || number < min || number > max) throw rangeError("value", value, min, max);
  const position = checkOffset(buffer, integer(offset), byteLength);
  let unsigned = signed && number < 0 ? 2 ** bits + number : number;
  for (let index = 0; index < byteLength; index += 1) {
    const shift = reverse ? index : byteLength - index - 1;
    buffer[position + index] = Math.floor(unsigned / 2 ** (shift * 8)) & 0xff;
  }
  return position + byteLength;
}

function writeBig(buffer: Uint8Array, value: unknown, offset: unknown, signed: boolean, reverse: boolean): number {
  if (typeof value !== "bigint") throw new TypeError("Cannot mix BigInt and other types, use explicit conversions");
  const [min, max] = signed ? [-(2n ** 63n), 2n ** 63n - 1n] : [0n, 2n ** 64n - 1n];
  if (value < min || value > max) throw rangeError("value", value, min, max);
  const position = checkOffset(buffer, integer(offset), 8);
  const view = new DataView(buffer.buffer, buffer.byteOffset + position, 8);
  if (signed) view.setBigInt64(0, value, !reverse);
  else view.setBigUint64(0, value, !reverse);
  return position + 8;
}

export const SlowBuffer = class SlowBuffer extends Buffer {};
export const kMaxLength = 2 ** 31 - 1;
export const kStringMaxLength = 536870888;
export const INSPECT_MAX_BYTES = 50;
export const constants = { MAX_LENGTH: kMaxLength, MAX_STRING_LENGTH: kStringMaxLength };
export function isAscii(value: unknown): boolean { return toBytes(value).every(byte => byte < 128); }
export function isUtf8(value: unknown): boolean { try { new TextDecoder("utf-8", { fatal: true }).decode(toBytes(value)); return true; } catch { return false; } }
export function transcode(value: unknown, fromEncoding: unknown = "utf8", toEncoding: unknown = "utf8"): Buffer {
  return Buffer.from(Buffer.from(value, fromEncoding).toString(toEncoding), toEncoding);
}
// The globals, as bound functions of their own.
export const atob = globalAtob.bind(globalThis);
export const btoa = globalBtoa.bind(globalThis);
export function resolveObjectURL(): never { throw new Error("Blob URLs are not available in the Tokamak runtime"); }

Object.defineProperty(Buffer, "length", { configurable: true, value: 3 });
Buffer.poolSize = 0;

export { Blob, File };
export default { Buffer, SlowBuffer, Blob, File, atob, btoa, constants, isAscii, isUtf8, kMaxLength, kStringMaxLength, INSPECT_MAX_BYTES, resolveObjectURL, transcode };
