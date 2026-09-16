import { Blob, File } from "../network/fetch.mjs";

const BASE64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const encodingNames = new Set(["utf8", "utf-8", "ascii", "latin1", "binary", "base64", "base64url", "hex", "ucs2", "ucs-2", "utf16le", "utf-16le"]);

function normalizeEncoding(encoding) {
  if (encoding === undefined || encoding === null) return "utf8";
  const name = String(encoding).toLowerCase();
  if (!encodingNames.has(name)) throw unknownEncoding(name);
  return name;
}

function unknownEncoding(name) {
  const error = new TypeError(`Unknown encoding: ${name}`);
  error.code = "ERR_UNKNOWN_ENCODING";
  return error;
}

function stringBytes(value, encoding) {
  const name = normalizeEncoding(encoding);
  if (name === "hex") return decodeHex(String(value));
  if (name === "base64" || name === "base64url") return decodeBase64(String(value));
  if (name === "ascii" || name === "latin1" || name === "binary") {
    const input = String(value);
    const bytes = new Uint8Array(input.length);
    for (let index = 0; index < input.length; index += 1) bytes[index] = input.charCodeAt(index) & 0xff;
    return bytes;
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

function decodeHex(value) {
  const bytes = [];
  for (let index = 0; index + 1 < value.length; index += 2) {
    const pair = value.slice(index, index + 2);
    if (!/^[0-9a-f]{2}$/i.test(pair)) break;
    bytes.push(parseInt(pair, 16));
  }
  return new Uint8Array(bytes);
}

function decodeBase64(value) {
  const input = value.replace(/[\t\n\f\r ]/g, "").replace(/-/g, "+").replace(/_/g, "/");
  const bytes = [];
  let accumulator = 0;
  let bits = 0;
  for (const character of input) {
    if (character === "=") break;
    const digit = BASE64.indexOf(character);
    if (digit < 0) continue;
    accumulator = (accumulator << 6) | digit;
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      bytes.push((accumulator >> bits) & 0xff);
    }
  }
  return new Uint8Array(bytes);
}

function encodeBase64(bytes) {
  let output = "";
  for (let index = 0; index < bytes.length; index += 3) {
    const first = bytes[index];
    const second = bytes[index + 1];
    const third = bytes[index + 2];
    output += BASE64[first >> 2];
    output += BASE64[((first & 3) << 4) | (second === undefined ? 0 : second >> 4)];
    output += second === undefined ? "=" : BASE64[((second & 15) << 2) | (third === undefined ? 0 : third >> 6)];
    output += third === undefined ? "=" : BASE64[third & 63];
  }
  return output;
}

function decode(bytes, encoding) {
  const name = normalizeEncoding(encoding);
  if (name === "hex") return [...bytes].map(value => value.toString(16).padStart(2, "0")).join("");
  if (name === "base64" || name === "base64url") {
    const value = encodeBase64(bytes);
    return name === "base64url" ? value.replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "") : value;
  }
  if (name === "ascii") return String.fromCharCode(...bytes.map(value => value & 0x7f));
  if (name === "latin1" || name === "binary") return String.fromCharCode(...bytes);
  if (name === "ucs2" || name === "ucs-2" || name === "utf16le" || name === "utf-16le") return new TextDecoder("utf-16le").decode(bytes);
  return new TextDecoder().decode(bytes);
}

function integer(value, fallback = 0) {
  const number = Number(value);
  if (Number.isNaN(number) || number === 0) return fallback;
  return number < 0 ? Math.ceil(number) : Math.floor(number);
}

function index(value, length) {
  const number = integer(value);
  if (number < 0) return Math.max(length + number, 0);
  return Math.min(number, length);
}

function rangeError(name, value, min, max) {
  const error = new RangeError(`The value of "${name}" is out of range. It must be >= ${min} && <= ${max}. Received ${value}`);
  error.code = "ERR_OUT_OF_RANGE";
  return error;
}

function checkOffset(buffer, offset, size) {
  if (!Number.isInteger(offset) || offset < 0 || offset + size > buffer.length) throw rangeError("offset", offset, 0, buffer.length - size);
  return offset;
}

function checkByteLength(size, max = 6) {
  if (!Number.isInteger(size) || size < 1 || size > max) throw new RangeError(`byteLength must be between 1 and ${max}`);
}

function fillBytes(buffer, value, start, end, encoding) {
  const first = typeof value === "number" ? Uint8Array.of(value & 0xff) : typeof value === "string" ? stringBytes(value, encoding) : toBytes(value, false);
  if (first.length === 0) return buffer;
  for (let index = start; index < end; index += 1) buffer[index] = first[(index - start) % first.length];
  return buffer;
}

function toBytes(value, allowString = true, encoding) {
  if (allowString && typeof value === "string") return stringBytes(value, encoding);
  if (value instanceof Buffer) return value;
  if (value instanceof ArrayBuffer || (typeof SharedArrayBuffer !== "undefined" && value instanceof SharedArrayBuffer)) return new Uint8Array(value);
  if (ArrayBuffer.isView(value)) {
    if (value instanceof DataView) return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
    return Uint8Array.from(value);
  }
  if (Array.isArray(value) || (value && typeof value.length === "number")) return Uint8Array.from(value);
  throw new TypeError("The value must be a string, Buffer, TypedArray, DataView, ArrayBuffer, or Array-like Object");
}

function pattern(value, encoding) {
  if (typeof value === "number") return Uint8Array.of(value & 0xff);
  return toBytes(value, true, encoding);
}

export class Buffer extends Uint8Array {
  static from(value, encodingOrOffset, length) {
    if (typeof value === "string") return new Buffer(stringBytes(value, encodingOrOffset));
    if (value && value.type === "Buffer" && Array.isArray(value.data)) return new Buffer(value.data);
    if (value instanceof ArrayBuffer || (typeof SharedArrayBuffer !== "undefined" && value instanceof SharedArrayBuffer)) {
      const offset = encodingOrOffset === undefined ? 0 : integer(encodingOrOffset);
      const available = value.byteLength - offset;
      const size = length === undefined ? available : integer(length);
      if (offset < 0 || size < 0 || offset + size > value.byteLength) throw rangeError("offset", offset, 0, value.byteLength);
      return new Buffer(value, offset, size);
    }
    if (ArrayBuffer.isView(value)) return value instanceof DataView ? new Buffer(value.buffer, value.byteOffset, value.byteLength) : new Buffer(Uint8Array.from(value));
    if (Array.isArray(value) || (value && typeof value.length === "number")) return new Buffer(value);
    throw new TypeError("The first argument must be a string or an ArrayBuffer");
  }

  static alloc(size, fill = 0, encoding) {
    const buffer = new Buffer(normalizeSize(size));
    return fill === 0 || fill === undefined ? buffer : buffer.fill(fill, 0, buffer.length, encoding);
  }

  static allocUnsafe(size) { return new Buffer(normalizeSize(size)); }
  static allocUnsafeSlow(size) { return Buffer.allocUnsafe(size); }

  static byteLength(value, encoding) {
    if (typeof value === "string") return stringBytes(value, encoding).byteLength;
    if (value instanceof ArrayBuffer || (typeof SharedArrayBuffer !== "undefined" && value instanceof SharedArrayBuffer)) return value.byteLength;
    if (ArrayBuffer.isView(value)) return value.byteLength;
    return stringBytes(String(value), encoding).byteLength;
  }

  static compare(left, right) {
    const a = toBytes(left, false);
    const b = toBytes(right, false);
    const length = Math.min(a.length, b.length);
    for (let index = 0; index < length; index += 1) if (a[index] !== b[index]) return a[index] < b[index] ? -1 : 1;
    return a.length === b.length ? 0 : a.length < b.length ? -1 : 1;
  }

  static concat(list, totalLength) {
    if (!Array.isArray(list)) throw new TypeError("The first argument must be an array of Buffers or Uint8Arrays");
    const buffers = list.map(value => toBytes(value, false));
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

  static isBuffer(value) { return value instanceof Buffer; }
  static isEncoding(value) { return typeof value === "string" && encodingNames.has(value.toLowerCase()); }
  static of(...values) { return new Buffer(values); }

  get parent() { return this.buffer; }
  get offset() { return this.byteOffset; }

  toString(encoding = "utf8", start = 0, end = this.length) {
    const first = index(start, this.length);
    const last = Math.max(first, index(end, this.length));
    return decode(this.subarray(first, last), encoding);
  }

  toJSON() { return { type: "Buffer", data: [...this] }; }
  inspect() { return `<Buffer ${[...this].slice(0, 50).map(value => value.toString(16).padStart(2, "0")).join(" ")}${this.length > 50 ? " …" : ""}>`; }
  toLocaleString(encoding, start, end) { return this.toString(encoding, start, end); }
  slice(start = 0, end = this.length) { return this.subarray(index(start, this.length), index(end, this.length)); }
  equals(other) { return Buffer.compare(this, other) === 0; }
  compare(other, targetStart, targetEnd, sourceStart, sourceEnd) {
    return Buffer.compare(this.subarray(sourceStart, sourceEnd), toBytes(other, false).subarray(targetStart, targetEnd));
  }
  copy(target, targetStart = 0, sourceStart = 0, sourceEnd = this.length) {
    if (!(target instanceof Uint8Array)) throw new TypeError("The target argument must be a Buffer or Uint8Array");
    const start = index(sourceStart, this.length);
    const end = index(sourceEnd, this.length);
    const destination = Math.max(0, integer(targetStart));
    const count = Math.min(end - start, target.length - destination);
    if (count > 0) target.set(this.subarray(start, start + count), destination);
    return Math.max(0, count);
  }
  fill(value, start = 0, end = this.length, encoding) {
    const first = index(start, this.length);
    const last = index(end, this.length);
    return fillBytes(this, value, first, last, encoding);
  }
  write(value, offset = 0, length = this.length - offset, encoding) {
    if (typeof offset === "string") { encoding = offset; offset = 0; length = this.length; }
    else if (typeof length === "string") { encoding = length; length = this.length - offset; }
    const start = integer(offset);
    if (start < 0 || start > this.length) throw rangeError("offset", start, 0, this.length);
    const count = Math.max(0, Math.min(integer(length), this.length - start));
    const bytes = stringBytes(value, encoding).subarray(0, count);
    this.set(bytes, start);
    return bytes.length;
  }

  indexOf(value, byteOffset = 0, encoding) { return find(this, pattern(value, encoding), byteOffset, false); }
  lastIndexOf(value, byteOffset = this.length - 1, encoding) { return find(this, pattern(value, encoding), byteOffset, true); }
  includes(value, byteOffset = 0, encoding) { return this.indexOf(value, byteOffset, encoding) !== -1; }
  swap16() { return swap(this, 2); }
  swap32() { return swap(this, 4); }
  swap64() { return swap(this, 8); }

  readUInt8(offset = 0) { return this[checkOffset(this, integer(offset), 1)]; }
  readUint8(offset = 0) { return this.readUInt8(offset); }
  readInt8(offset = 0) { const value = this.readUInt8(offset); return value > 127 ? value - 256 : value; }
  readUInt16LE(offset = 0) { return dataView(this, offset, 2).getUint16(0, true); }
  readUint16LE(offset = 0) { return this.readUInt16LE(offset); }
  readUInt16BE(offset = 0) { return dataView(this, offset, 2).getUint16(0, false); }
  readUint16BE(offset = 0) { return this.readUInt16BE(offset); }
  readInt16LE(offset = 0) { return dataView(this, offset, 2).getInt16(0, true); }
  readInt16BE(offset = 0) { return dataView(this, offset, 2).getInt16(0, false); }
  readUInt32LE(offset = 0) { return dataView(this, offset, 4).getUint32(0, true); }
  readUint32LE(offset = 0) { return this.readUInt32LE(offset); }
  readUInt32BE(offset = 0) { return dataView(this, offset, 4).getUint32(0, false); }
  readUint32BE(offset = 0) { return this.readUInt32BE(offset); }
  readInt32LE(offset = 0) { return dataView(this, offset, 4).getInt32(0, true); }
  readInt32BE(offset = 0) { return dataView(this, offset, 4).getInt32(0, false); }
  readUIntLE(offset, byteLength) { return readInteger(this, offset, byteLength, false, false); }
  readUintLE(offset, byteLength) { return this.readUIntLE(offset, byteLength); }
  readUIntBE(offset, byteLength) { return readInteger(this, offset, byteLength, false, true); }
  readUintBE(offset, byteLength) { return this.readUIntBE(offset, byteLength); }
  readIntLE(offset, byteLength) { return readInteger(this, offset, byteLength, true, false); }
  readIntBE(offset, byteLength) { return readInteger(this, offset, byteLength, true, true); }
  readFloatLE(offset = 0) { return dataView(this, offset, 4).getFloat32(0, true); }
  readFloatBE(offset = 0) { return dataView(this, offset, 4).getFloat32(0, false); }
  readDoubleLE(offset = 0) { return dataView(this, offset, 8).getFloat64(0, true); }
  readDoubleBE(offset = 0) { return dataView(this, offset, 8).getFloat64(0, false); }
  readBigUInt64LE(offset = 0) { return dataView(this, offset, 8).getBigUint64(0, true); }
  readBigUint64LE(offset = 0) { return this.readBigUInt64LE(offset); }
  readBigUInt64BE(offset = 0) { return dataView(this, offset, 8).getBigUint64(0, false); }
  readBigUint64BE(offset = 0) { return this.readBigUInt64BE(offset); }
  readBigInt64LE(offset = 0) { return dataView(this, offset, 8).getBigInt64(0, true); }
  readBigInt64BE(offset = 0) { return dataView(this, offset, 8).getBigInt64(0, false); }

  writeUInt8(value, offset = 0) { return writeData(this, offset, 1, value, (view, number) => view.setUint8(0, number), 0, 255); }
  writeUint8(value, offset = 0) { return this.writeUInt8(value, offset); }
  writeInt8(value, offset = 0) { return writeData(this, offset, 1, value, (view, number) => view.setInt8(0, number), -128, 127); }
  writeUInt16LE(value, offset = 0) { return writeData(this, offset, 2, value, (view, number) => view.setUint16(0, number, true), 0, 65535); }
  writeUint16LE(value, offset = 0) { return this.writeUInt16LE(value, offset); }
  writeUInt16BE(value, offset = 0) { return writeData(this, offset, 2, value, (view, number) => view.setUint16(0, number, false), 0, 65535); }
  writeUint16BE(value, offset = 0) { return this.writeUInt16BE(value, offset); }
  writeInt16LE(value, offset = 0) { return writeData(this, offset, 2, value, (view, number) => view.setInt16(0, number, true), -32768, 32767); }
  writeInt16BE(value, offset = 0) { return writeData(this, offset, 2, value, (view, number) => view.setInt16(0, number, false), -32768, 32767); }
  writeUInt32LE(value, offset = 0) { return writeData(this, offset, 4, value, (view, number) => view.setUint32(0, number, true), 0, 0xffffffff); }
  writeUint32LE(value, offset = 0) { return this.writeUInt32LE(value, offset); }
  writeUInt32BE(value, offset = 0) { return writeData(this, offset, 4, value, (view, number) => view.setUint32(0, number, false), 0, 0xffffffff); }
  writeUint32BE(value, offset = 0) { return this.writeUInt32BE(value, offset); }
  writeInt32LE(value, offset = 0) { return writeData(this, offset, 4, value, (view, number) => view.setInt32(0, number, true), -0x80000000, 0x7fffffff); }
  writeInt32BE(value, offset = 0) { return writeData(this, offset, 4, value, (view, number) => view.setInt32(0, number, false), -0x80000000, 0x7fffffff); }
  writeUIntLE(value, offset, byteLength) { return writeInteger(this, value, offset, byteLength, false, false); }
  writeUintLE(value, offset, byteLength) { return this.writeUIntLE(value, offset, byteLength); }
  writeUIntBE(value, offset, byteLength) { return writeInteger(this, value, offset, byteLength, false, true); }
  writeUintBE(value, offset, byteLength) { return this.writeUIntBE(value, offset, byteLength); }
  writeIntLE(value, offset, byteLength) { return writeInteger(this, value, offset, byteLength, true, false); }
  writeIntBE(value, offset, byteLength) { return writeInteger(this, value, offset, byteLength, true, true); }
  writeFloatLE(value, offset = 0) { return writeData(this, offset, 4, value, (view, number) => view.setFloat32(0, number, true), -Infinity, Infinity, false); }
  writeFloatBE(value, offset = 0) { return writeData(this, offset, 4, value, (view, number) => view.setFloat32(0, number, false), -Infinity, Infinity, false); }
  writeDoubleLE(value, offset = 0) { return writeData(this, offset, 8, value, (view, number) => view.setFloat64(0, number, true), -Infinity, Infinity, false); }
  writeDoubleBE(value, offset = 0) { return writeData(this, offset, 8, value, (view, number) => view.setFloat64(0, number, false), -Infinity, Infinity, false); }
  writeBigUInt64LE(value, offset = 0) { return writeBig(this, value, offset, false, false); }
  writeBigUint64LE(value, offset = 0) { return this.writeBigUInt64LE(value, offset); }
  writeBigUInt64BE(value, offset = 0) { return writeBig(this, value, offset, false, true); }
  writeBigUint64BE(value, offset = 0) { return this.writeBigUInt64BE(value, offset); }
  writeBigInt64LE(value, offset = 0) { return writeBig(this, value, offset, true, false); }
  writeBigInt64BE(value, offset = 0) { return writeBig(this, value, offset, true, true); }

  asciiSlice(start, end) { return this.toString("ascii", start, end); }
  latin1Slice(start, end) { return this.toString("latin1", start, end); }
  binarySlice(start, end) { return this.toString("latin1", start, end); }
  base64Slice(start, end) { return this.toString("base64", start, end); }
  base64urlSlice(start, end) { return this.toString("base64url", start, end); }
  hexSlice(start, end) { return this.toString("hex", start, end); }
  utf8Slice(start, end) { return this.toString("utf8", start, end); }
  ucs2Slice(start, end) { return this.toString("utf16le", start, end); }
  utf16leSlice(start, end) { return this.toString("utf16le", start, end); }
  asciiWrite(value, offset, length) { return this.write(value, offset, length, "ascii"); }
  latin1Write(value, offset, length) { return this.write(value, offset, length, "latin1"); }
  binaryWrite(value, offset, length) { return this.write(value, offset, length, "latin1"); }
  base64Write(value, offset, length) { return this.write(value, offset, length, "base64"); }
  base64urlWrite(value, offset, length) { return this.write(value, offset, length, "base64url"); }
  hexWrite(value, offset, length) { return this.write(value, offset, length, "hex"); }
  utf8Write(value, offset, length) { return this.write(value, offset, length, "utf8"); }
  ucs2Write(value, offset, length) { return this.write(value, offset, length, "utf16le"); }
  utf16leWrite(value, offset, length) { return this.write(value, offset, length, "utf16le"); }
}

function normalizeSize(size) {
  const value = integer(size);
  if (value < 0 || !Number.isSafeInteger(value)) throw new RangeError(`Invalid typed array length: ${size}`);
  return value;
}

function dataView(buffer, offset, size) {
  return new DataView(buffer.buffer, buffer.byteOffset + checkOffset(buffer, integer(offset), size), size);
}

function find(buffer, needle, byteOffset, reverse) {
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

function swap(buffer, size) {
  if (buffer.length % size !== 0) {
    const bits = size * 8;
    const error = new RangeError(`Buffer size must be a multiple of ${bits}-bits`);
    error.code = "ERR_INVALID_BUFFER_SIZE";
    throw error;
  }
  for (let offset = 0; offset < buffer.length; offset += size) for (let left = 0; left < size / 2; left += 1) {
    const right = size - left - 1;
    [buffer[offset + left], buffer[offset + right]] = [buffer[offset + right], buffer[offset + left]];
  }
  return buffer;
}

function readInteger(buffer, offset, byteLength, signed, reverse) {
  checkByteLength(byteLength);
  checkOffset(buffer, integer(offset), byteLength);
  let value = 0;
  for (let index = 0; index < byteLength; index += 1) value += buffer[integer(offset) + (reverse ? byteLength - index - 1 : index)] * 2 ** (index * 8);
  if (signed && value >= 2 ** (byteLength * 8 - 1)) value -= 2 ** (byteLength * 8);
  return value;
}

function writeData(buffer, offset, size, value, writer, min, max, validate = true) {
  const position = checkOffset(buffer, integer(offset), size);
  const number = Number(value);
  if (validate && (!Number.isFinite(number) || !Number.isInteger(number) || number < min || number > max)) throw rangeError("value", value, min, max);
  writer(new DataView(buffer.buffer, buffer.byteOffset + position, size), number);
  return position + size;
}

function writeInteger(buffer, value, offset, byteLength, signed, reverse) {
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

function writeBig(buffer, value, offset, signed, reverse) {
  const position = checkOffset(buffer, integer(offset), 8);
  const view = new DataView(buffer.buffer, buffer.byteOffset + position, 8);
  const big = BigInt(value);
  if (signed) view.setBigInt64(0, big, !reverse);
  else view.setBigUint64(0, big, !reverse);
  return position + 8;
}

export const SlowBuffer = class SlowBuffer extends Buffer {};
export const kMaxLength = 2 ** 31 - 1;
export const kStringMaxLength = 536870888;
export const INSPECT_MAX_BYTES = 50;
export const constants = { MAX_LENGTH: kMaxLength, MAX_STRING_LENGTH: kStringMaxLength };
export function isAscii(value) { return toBytes(value).every(byte => byte < 128); }
export function isUtf8(value) { try { new TextDecoder("utf-8", { fatal: true }).decode(toBytes(value)); return true; } catch { return false; } }
export function transcode(value, fromEncoding = "utf8", toEncoding = "utf8") {
  return Buffer.from(Buffer.from(value, fromEncoding).toString(toEncoding), toEncoding);
}
export function atob(value) { return Buffer.from(decodeBase64(String(value))).toString("latin1"); }
export function btoa(value) { return encodeBase64(stringBytes(String(value), "latin1")); }
export const atobBuffer = value => Buffer.from(atob(value), "latin1");
export const btoaBuffer = value => btoa(Buffer.from(value).toString("latin1"));
export function resolveObjectURL() { throw new Error("Blob URLs are not available in the Tokamak runtime"); }

Object.defineProperty(Buffer, "length", { configurable: true, value: 3 });
Buffer.poolSize = 0;
if (typeof globalThis.Buffer !== "function") globalThis.Buffer = Buffer;

export { Blob, File };
export default { Buffer, SlowBuffer, Blob, File, atob, btoa, constants, isAscii, isUtf8, kMaxLength, kStringMaxLength, INSPECT_MAX_BYTES, resolveObjectURL, transcode };
