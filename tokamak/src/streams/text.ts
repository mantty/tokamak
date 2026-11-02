import { isObject, markHostObject } from "../globals/objects.ts";
import { createDecoder, type TextDecoderState } from "tokamak:host";
import { codePoints, sharedBufferSourceBytes, string } from "../globals/conversions.ts";

function utf8(code: number): number[] {
  if (code >= 0xd800 && code <= 0xdfff) code = 0xfffd;
  if (code < 0x80) return [code];
  if (code < 0x800) return [0xc0 | (code >> 6), 0x80 | (code & 0x3f)];
  if (code < 0x10000) return [0xe0 | (code >> 12), 0x80 | ((code >> 6) & 0x3f), 0x80 | (code & 0x3f)];
  return [0xf0 | (code >> 18), 0x80 | ((code >> 12) & 0x3f), 0x80 | ((code >> 6) & 0x3f), 0x80 | (code & 0x3f)];
}

export class TextEncoder {
  constructor() { markHostObject(this); }
  get encoding() { return "utf-8"; }
  encode(value: unknown = ""): Uint8Array { return new Uint8Array(codePoints(string(value)).flatMap(code => utf8(code))); }
  encodeInto(source: unknown, destination: unknown): { read: number; written: number } {
    const text = string(source);
    if (!(destination instanceof Uint8Array)) throw new TypeError("Destination must be a Uint8Array");
    let read = 0, written = 0;
    for (const code of codePoints(text)) {
      const bytes = utf8(code);
      if (written + bytes.length > destination.byteLength) break;
      destination.set(bytes, written);
      written += bytes.length;
      read += code > 0xffff ? 2 : 1;
    }
    return { read, written };
  }
}

export class TextDecoder {
  #decoder: TextDecoderState;
  #fatal: boolean;
  #ignoreBOM: boolean;
  constructor(label: unknown = "utf-8", options: unknown = {}) {
    markHostObject(this);
    const encoding = string(label);
    if (!isObject(options)) throw new TypeError("Options must be an object");
    this.#fatal = Boolean(options.fatal);
    this.#ignoreBOM = Boolean(options.ignoreBOM);
    this.#decoder = createDecoder(encoding, this.#fatal, this.#ignoreBOM);
  }
  get encoding() { return this.#decoder.encoding; }
  get fatal() { return this.#fatal; }
  get ignoreBOM() { return this.#ignoreBOM; }
  decode(input: unknown = new Uint8Array(), options: unknown = {}): string {
    const bytes = sharedBufferSourceBytes(input);
    if (!bytes) throw new TypeError("Failed to execute 'decode' on 'TextDecoder': parameter 1 is not of type 'Array'.");
    if (!isObject(options)) throw new TypeError("Options must be an object");
    return this.#decoder.decode(bytes, Boolean(options.stream));
  }
}
