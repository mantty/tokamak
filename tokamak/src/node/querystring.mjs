import { string } from "../globals/conversions.mjs";
import { TextDecoder } from "../streams/text.mjs";
import { Buffer } from "./buffer.mjs";

function escape(value) {
  return encodeURIComponent(string(value));
}

function decodeComponent(input, decodeSpaces) {
  try { return decodeURIComponent(input); }
  catch { return new TextDecoder("utf-8", { ignoreBOM: true }).decode(unescapeBuffer(input, decodeSpaces)); }
}

// A parsed key or value is decoded only when it holds a valid percent escape.
function decodeParsed(value) {
  const input = value.replaceAll("+", " ");
  return /%[0-9a-f]{2}/i.test(input) ? decodeComponent(input, true) : input;
}

function valueString(value) {
  if (value === null || value === undefined || typeof value === "object" || typeof value === "function" || typeof value === "symbol") return "";
  return string(value);
}

export function parse(input, separator = "&", equal = "=", options = {}) {
  const output = Object.create(null);
  const value = input == null ? "" : string(input);
  const pairSeparator = string(separator);
  const keyValueSeparator = string(equal);
  const decoder = typeof options?.decodeURIComponent === "function" ? options.decodeURIComponent : decodeParsed;
  const maxKeys = options?.maxKeys === undefined ? 1000 : Number(options.maxKeys);
  const limit = maxKeys === 0 ? Infinity : Math.max(0, Math.trunc(maxKeys));
  if (value === "" || limit === 0) return output;
  const parts = value.split(pairSeparator);
  let count = 0;
  for (const part of parts) {
    if (count >= limit) break;
    if (part === "") continue;
    const index = part.indexOf(keyValueSeparator);
    const key = decoder(index < 0 ? part : part.slice(0, index));
    const item = decoder(index < 0 ? "" : part.slice(index + keyValueSeparator.length));
    if (Object.hasOwn(output, key)) output[key] = Array.isArray(output[key]) ? [...output[key], item] : [output[key], item];
    else output[key] = item;
    count += 1;
  }
  return output;
}

export const decode = parse;

export function stringify(value, separator = "&", equal = "=", options = {}) {
  if (value == null) return "";
  const pairSeparator = string(separator);
  const keyValueSeparator = string(equal);
  const encoder = typeof options?.encodeURIComponent === "function" ? options.encodeURIComponent : escape;
  const pairs = [];
  for (const key of Object.keys(Object(value))) {
    const item = value[key];
    const values = Array.isArray(item) ? item : [item];
    for (const current of values) pairs.push(encoder(key) + keyValueSeparator + encoder(valueString(current)));
  }
  return pairs.join(pairSeparator);
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
