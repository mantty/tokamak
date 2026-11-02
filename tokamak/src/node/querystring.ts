import { propertyKey, string } from "../globals/conversions.ts";
import { isFunction, isObject, objectOf, propertyOf } from "../globals/objects.ts";
import { TextDecoder } from "../streams/text.ts";
import { Buffer } from "./buffer.ts";
import { codedError } from "./errors.ts";

const hexDigit = /[0-9A-Fa-f]/;

function malformedURI(): Error {
  return Object.assign(codedError(Error, "URI malformed", "ERR_INVALID_URI"), { name: "URIError" });
}

// UTF-8 percent-encoding in which any surrogate code unit pairs with the unit after it.
function encodeString(value: string): string {
  if (value.isWellFormed()) return encodeURIComponent(value);
  let output = "";
  for (let index = 0; index < value.length; index += 1) {
    const unit = value.charCodeAt(index);
    if (unit < 0xd800 || unit >= 0xe000) {
      output += encodeURIComponent(value[index]);
      continue;
    }
    index += 1;
    if (index === value.length) throw malformedURI();
    output += encodeURIComponent(String.fromCodePoint(0x10000 + (((unit & 0x3ff) << 10) | (value.charCodeAt(index) & 0x3ff))));
  }
  return output;
}

// A function converts by concatenation, any other value by ToString.
function escape(input: unknown): string {
  return encodeString(isFunction(input) ? input + "" : string(input));
}

function decodeComponent(input: string, decodeSpaces: boolean): string {
  try { return decodeURIComponent(input); }
  catch { return new TextDecoder("utf-8", { ignoreBOM: true }).decode(unescapeBuffer(input, decodeSpaces)); }
}

// Decodes a key or value, as the default decoder or a custom one returns it.
type Decoder = (text: string) => unknown;

function decodeWith(decode: Decoder, text: string): unknown {
  try { return decode(text); }
  catch { return unescape(text, true); }
}

// The number of pairs parse reads; -1, never reached by counting down, reads all.
function maxPairs(options: unknown): number {
  const maxKeys = propertyOf(options, "maxKeys");
  if (typeof maxKeys !== "number") return 1000;
  return maxKeys > 0 ? maxKeys : -1;
}

// The state of the pair being read.
interface Pair {
  key: string;
  value: string;
  keyEncoded: boolean;
  valueEncoded: boolean;
  escapeProgress: number;
  pendingStart: number;
  separatorMatched: number;
  equalsMatched: number;
}

// The state of one parse.
interface Scan extends Pair {
  input: string;
  output: Record<PropertyKey, unknown>;
  decode: Decoder;
  custom: boolean;
  separator: string;
  equals: string;
  plus: string;
  pairs: number;
}

// A pair starting at `position`, encoded from the start when the decoder is custom.
function pairAt(position: number, custom: boolean): Pair {
  return { key: "", value: "", keyEncoded: custom, valueEncoded: custom, escapeProgress: 0, pendingStart: position, separatorMatched: 0, equalsMatched: 0 };
}

function startPair(scan: Scan, position: number): void {
  Object.assign(scan, pairAt(position, scan.custom));
}

// A key or value is decoded only when it holds a percent escape or the decoder is custom.
function addPair(scan: Scan): void {
  const key = propertyKey(scan.key.length > 0 && scan.keyEncoded ? decodeWith(scan.decode, scan.key) : scan.key);
  const value = scan.value.length > 0 && scan.valueEncoded ? decodeWith(scan.decode, scan.value) : scan.value;
  const current = scan.output[key];
  if (current === undefined) scan.output[key] = value;
  // Any value with `pop` collects repeats, whatever type a custom decoder returns.
  else if (isObject(current) && current.pop) current[propertyKey(current.length)] = value;
  else scan.output[key] = [current, value];
}

// Ends the pair before the separator ending at `index`; true when no more pairs may be read.
function endPair(scan: Scan, index: number): boolean {
  const pending = scan.input.slice(scan.pendingStart, index - scan.separatorMatched + 1);
  const inKey = scan.equalsMatched < scan.equals.length;
  if (inKey) scan.key += pending;
  else scan.value += pending;
  if (!inKey || scan.key !== "") addPair(scan);
  scan.pairs -= 1;
  if (scan.pairs === 0) return true;
  startPair(scan, index + 1);
  return false;
}

// Advances the percent-escape check by `character`; true while it may continue
// an escape, which is complete when `escapeProgress` reaches 3.
function trackEscape(scan: Scan, character: string): boolean {
  if (character === "%") {
    scan.escapeProgress = 1;
    return true;
  }
  if (scan.escapeProgress === 0) return false;
  if (hexDigit.test(character)) {
    scan.escapeProgress += 1;
    return true;
  }
  scan.escapeProgress = 0;
  return false;
}

// Appends the text before the "+" at `index`, and the "+" as a space, to the key or value.
function appendPlus(scan: Scan, part: "key" | "value", index: number): void {
  scan[part] += scan.input.slice(scan.pendingStart, index) + scan.plus;
  scan.pendingStart = index + 1;
}

// Reads a key character; false when the value's rules still apply to it.
function readKeyCharacter(scan: Scan, index: number): boolean {
  const character = scan.input[index];
  if (character === scan.equals[scan.equalsMatched]) {
    scan.equalsMatched += 1;
    if (scan.equalsMatched < scan.equals.length) return true;
    scan.key += scan.input.slice(scan.pendingStart, index - scan.equalsMatched + 1);
    scan.escapeProgress = 0;
    scan.pendingStart = index + 1;
    return true;
  }
  scan.equalsMatched = 0;
  if (!scan.keyEncoded && trackEscape(scan, character)) {
    scan.keyEncoded = scan.escapeProgress === 3;
    return true;
  }
  if (character !== "+") return false;
  appendPlus(scan, "key", index);
  return true;
}

function readValueCharacter(scan: Scan, index: number): void {
  const character = scan.input[index];
  if (character === "+") appendPlus(scan, "value", index);
  else if (!scan.valueEncoded && trackEscape(scan, character) && scan.escapeProgress === 3) scan.valueEncoded = true;
}

// Reads the character at `index`; true when no more pairs may be read.
function readCharacter(scan: Scan, index: number): boolean {
  if (scan.input[index] === scan.separator[scan.separatorMatched]) {
    scan.separatorMatched += 1;
    return scan.separatorMatched === scan.separator.length && endPair(scan, index);
  }
  scan.separatorMatched = 0;
  if (scan.equalsMatched < scan.equals.length && readKeyCharacter(scan, index)) return false;
  readValueCharacter(scan, index);
  return false;
}

// A single pass over `input`, matching separators one character at a time.
export function parse(input: unknown, separator?: unknown, equals?: unknown, options?: unknown): Record<PropertyKey, unknown> {
  const output: Record<PropertyKey, unknown> = { __proto__: null };
  if (typeof input !== "string" || input.length === 0) return output;
  const pairSeparator = separator ? String(separator) : "&";
  const keySeparator = equals ? String(equals) : "=";
  const pairs = maxPairs(options);
  const decoder = propertyOf(options, "decodeURIComponent");
  const decode: Decoder = isFunction(decoder) ? decoder : unescape;
  const custom = decode !== unescape;
  const scan: Scan = {
    input, output, decode, custom, separator: pairSeparator, equals: keySeparator, plus: custom ? "%20" : " ", pairs,
    ...pairAt(0, custom),
  };
  for (let index = 0; index < input.length; index += 1) {
    if (readCharacter(scan, index)) return output;
  }
  if (scan.pendingStart < input.length) {
    if (scan.equalsMatched < scan.equals.length) scan.key += input.slice(scan.pendingStart);
    else if (scan.separatorMatched < scan.separator.length) scan.value += input.slice(scan.pendingStart);
  } else if (scan.equalsMatched === 0 && scan.key.length === 0) return output;
  addPair(scan);
  return output;
}

export const decode = parse;

function stringifyPrimitive(value: unknown): string {
  if (typeof value === "string") return value;
  if ((typeof value === "number" && Number.isFinite(value)) || typeof value === "bigint") return String(value);
  if (typeof value === "boolean") return value ? "true" : "false";
  return "";
}

// Numbers from 1e21 up are written in exponent form, so escaped.
function encodeStringified(value: unknown): string {
  if (typeof value === "string") return value.length ? escape(value) : "";
  if (typeof value === "number" && Number.isFinite(value)) return Math.abs(value) < 1e21 ? String(value) : escape(String(value));
  return stringifyPrimitive(value);
}

export function stringify(object: unknown, separator?: unknown, equals?: unknown, options?: unknown): string {
  const pairSeparator = separator || "&";
  const keySeparator = equals || "=";
  const encoder = propertyOf(options, "encodeURIComponent");
  const encode = isFunction(encoder) ? encoder : escape;
  const convert = encode === escape ? encodeStringified : (value: unknown) => encode(stringifyPrimitive(value));
  if (object === null || typeof object !== "object") return "";
  let fields = "";
  for (const name of Object.keys(object)) {
    const value = objectOf(object)[name];
    let field = "";
    field += convert(name);
    field += keySeparator;
    const values: readonly unknown[] = Array.isArray(value) ? value : [value];
    if (values.length === 0) continue;
    if (fields) fields += pairSeparator;
    for (let index = 0; index < values.length; index += 1) {
      if (index) fields += pairSeparator;
      fields += field;
      fields += convert(values[index]);
    }
  }
  return fields;
}

export const encode = stringify;
export { escape };

export function unescape(value: unknown, decodeSpaces = false): string { return decodeComponent(string(value), decodeSpaces); }

// Each UTF-16 unit that is not a percent escape becomes its low byte.
export function unescapeBuffer(value: unknown, decodeSpaces = false): Buffer {
  const input = string(value);
  const bytes: number[] = [];
  for (let index = 0; index < input.length; index += 1) {
    const hex = input.slice(index + 1, index + 3);
    if (decodeSpaces && input[index] === "+") bytes.push(32);
    else if (input[index] === "%" && /^[0-9a-f]{2}$/i.test(hex)) {
      bytes.push(parseInt(hex, 16));
      index += 2;
    } else bytes.push(input.charCodeAt(index) & 0xff);
  }
  return Buffer.from(bytes);
}

export default { parse, decode, stringify, encode, escape, unescape, unescapeBuffer };
