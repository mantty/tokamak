import { createDecoder } from "tokamak:host";
import { TextDecoderStream, TextEncoderStream, TransformStream } from "./web.mjs";

export { TextDecoderStream, TextEncoderStream };

function string(value) {
  if (typeof value === "symbol") throw new TypeError("Cannot convert a Symbol to a string");
  return String(value);
}

function utf8(code) {
  if (code >= 0xd800 && code <= 0xdfff) code = 0xfffd;
  if (code < 0x80) return [code];
  if (code < 0x800) return [0xc0 | (code >> 6), 0x80 | (code & 0x3f)];
  if (code < 0x10000) return [0xe0 | (code >> 12), 0x80 | ((code >> 6) & 0x3f), 0x80 | (code & 0x3f)];
  return [0xf0 | (code >> 18), 0x80 | ((code >> 12) & 0x3f), 0x80 | ((code >> 6) & 0x3f), 0x80 | (code & 0x3f)];
}

export class TextEncoder {
  get encoding() { return "utf-8"; }
  encode(value = "") {
    const bytes = [];
    for (const character of string(value)) bytes.push(...utf8(character.codePointAt(0)));
    return new Uint8Array(bytes);
  }
  encodeInto(source, destination) {
    source = string(source);
    if (!(destination instanceof Uint8Array)) throw new TypeError("Destination must be a Uint8Array");
    let read = 0, written = 0;
    for (const character of source) {
      const bytes = utf8(character.codePointAt(0));
      if (written + bytes.length > destination.byteLength) break;
      destination.set(bytes, written);
      written += bytes.length;
      read += character.length;
    }
    return { read, written };
  }
}

export class TextDecoder {
  #decoder;
  #fatal;
  #ignoreBOM;
  constructor(label = "utf-8", options = {}) {
    label = string(label);
    if (options === null || (typeof options !== "object" && typeof options !== "function")) throw new TypeError("Options must be an object");
    this.#fatal = Boolean(options.fatal);
    this.#ignoreBOM = Boolean(options.ignoreBOM);
    this.#decoder = createDecoder(label, this.#fatal, this.#ignoreBOM);
  }
  get encoding() { return this.#decoder.encoding; }
  get fatal() { return this.#fatal; }
  get ignoreBOM() { return this.#ignoreBOM; }
  decode(input = new Uint8Array(), options = {}) {
    let bytes;
    if (input instanceof ArrayBuffer) bytes = input.byteLength ? new Uint8Array(input) : new Uint8Array();
    else if (ArrayBuffer.isView(input)) bytes = input.buffer.byteLength ? new Uint8Array(input.buffer, input.byteOffset, input.byteLength) : new Uint8Array();
    else throw new TypeError("Input must be an ArrayBuffer or an ArrayBuffer view");
    if (options === null || (typeof options !== "object" && typeof options !== "function")) throw new TypeError("Options must be an object");
    return this.#decoder.decode(bytes, Boolean(options.stream));
  }
}
