import { string } from "../globals/conversions.mjs";
import { TextDecoder } from "../streams/text.mjs";
import { Buffer } from "./buffer.mjs";
import { codedError } from "./errors.mjs";

const hexDigit = /[0-9A-Fa-f]/;

function malformedURI() {
  return Object.assign(codedError(Error, "URI malformed", "ERR_INVALID_URI"), { name: "URIError" });
}

// UTF-8 percent-encoding in which any surrogate code unit pairs with the unit after it.
function encodeString(value) {
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
function escape(input) {
  return encodeString(typeof input === "function" ? input + "" : string(input));
}

function decodeComponent(input, decodeSpaces) {
  try { return decodeURIComponent(input); }
  catch { return new TextDecoder("utf-8", { ignoreBOM: true }).decode(unescapeBuffer(input, decodeSpaces)); }
}

function decodeWith(decode, text) {
  try { return decode(text); }
  catch { return unescape(text, true); }
}

// The number of pairs parse reads; -1, never reached by counting down, reads all.
function maxPairs(options) {
  if (typeof options?.maxKeys !== "number") return 1000;
  return options.maxKeys > 0 ? options.maxKeys : -1;
}

function startPair(scan, position) {
  Object.assign(scan, { key: "", value: "", keyEncoded: scan.custom, valueEncoded: scan.custom, escapeProgress: 0, pendingStart: position, separatorMatched: 0, equalsMatched: 0 });
}

// A key or value is decoded only when it holds a percent escape or the decoder is custom.
function addPair(scan) {
  const key = scan.key.length > 0 && scan.keyEncoded ? decodeWith(scan.decode, scan.key) : scan.key;
  const value = scan.value.length > 0 && scan.valueEncoded ? decodeWith(scan.decode, scan.value) : scan.value;
  const current = scan.output[key];
  if (current === undefined) scan.output[key] = value;
  // Any value with `pop` collects repeats, whatever type a custom decoder returns.
  else if (current.pop) current[current.length] = value;
  else scan.output[key] = [current, value];
}

// Ends the pair before the separator ending at `index`; true when no more pairs may be read.
function endPair(scan, index) {
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
function trackEscape(scan, character) {
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
function appendPlus(scan, part, index) {
  scan[part] += scan.input.slice(scan.pendingStart, index) + scan.plus;
  scan.pendingStart = index + 1;
}

// Reads a key character; false when the value's rules still apply to it.
function readKeyCharacter(scan, index) {
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

function readValueCharacter(scan, index) {
  const character = scan.input[index];
  if (character === "+") appendPlus(scan, "value", index);
  else if (!scan.valueEncoded && trackEscape(scan, character) && scan.escapeProgress === 3) scan.valueEncoded = true;
}

// Reads the character at `index`; true when no more pairs may be read.
function readCharacter(scan, index) {
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
export function parse(input, separator, equals, options) {
  const output = { __proto__: null };
  if (typeof input !== "string" || input.length === 0) return output;
  const pairSeparator = separator ? String(separator) : "&";
  const keySeparator = equals ? String(equals) : "=";
  const pairs = maxPairs(options);
  const decode = typeof options?.decodeURIComponent === "function" ? options.decodeURIComponent : unescape;
  const custom = decode !== unescape;
  const scan = { input, output, decode, custom, separator: pairSeparator, equals: keySeparator, plus: custom ? "%20" : " ", pairs };
  startPair(scan, 0);
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

function stringifyPrimitive(value) {
  if (typeof value === "string") return value;
  if ((typeof value === "number" && Number.isFinite(value)) || typeof value === "bigint") return String(value);
  if (typeof value === "boolean") return value ? "true" : "false";
  return "";
}

// Numbers from 1e21 up are written in exponent form, so escaped.
function encodeStringified(value) {
  if (typeof value === "string") return value.length ? escape(value) : "";
  if (typeof value === "number" && Number.isFinite(value)) return Math.abs(value) < 1e21 ? String(value) : escape(String(value));
  return stringifyPrimitive(value);
}

export function stringify(object, separator, equals, options) {
  const pairSeparator = separator || "&";
  const keySeparator = equals || "=";
  const encode = typeof options?.encodeURIComponent === "function" ? options.encodeURIComponent : escape;
  const convert = encode === escape ? encodeStringified : value => encode(stringifyPrimitive(value));
  if (object === null || typeof object !== "object") return "";
  let fields = "";
  for (const name of Object.keys(object)) {
    const value = object[name];
    const field = convert(name) + keySeparator;
    const values = Array.isArray(value) ? value : [value];
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

export function unescape(value, decodeSpaces = false) { return decodeComponent(string(value), decodeSpaces); }

// Each UTF-16 unit that is not a percent escape becomes its low byte.
export function unescapeBuffer(value, decodeSpaces = false) {
  const input = string(value);
  const bytes = [];
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
