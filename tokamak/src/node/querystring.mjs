function string(value) {
  if (typeof value === "symbol") throw new TypeError("Cannot convert a Symbol value to a string");
  return String(value);
}

function escape(value) {
  return encodeURIComponent(string(value));
}

function decodeComponent(value, plusAsSpace) {
  const input = string(value).replace(plusAsSpace ? /\+/g : /$^/, " ");
  try { return decodeURIComponent(input); }
  catch { return decodeInvalid(input); }
}

function decodeInvalid(value) {
  const bytes = [];
  let output = "";
  for (let index = 0; index < value.length;) {
    if (value[index] === "%" && /^[0-9a-f]{2}$/i.test(value.slice(index + 1, index + 3))) {
      bytes.push(parseInt(value.slice(index + 1, index + 3), 16));
      index += 3;
    } else {
      if (bytes.length) { output += new TextDecoder().decode(new Uint8Array(bytes)); bytes.length = 0; }
      const character = value[index];
      output += character;
      index += character.length;
    }
  }
  if (bytes.length) output += new TextDecoder().decode(new Uint8Array(bytes));
  return output;
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
  const decoder = typeof options?.decodeURIComponent === "function" ? options.decodeURIComponent : value => decodeComponent(value, true);
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

export function unescape(value) { return decodeComponent(value, false); }

export function unescapeBuffer(value, decodeSpaces = false) {
  const input = string(value);
  const bytes = [];
  for (let index = 0; index < input.length;) {
    if (decodeSpaces && input[index] === "+") { bytes.push(32); index += 1; continue; }
    if (input[index] === "%" && /^[0-9a-f]{2}$/i.test(input.slice(index + 1, index + 3))) {
      bytes.push(parseInt(input.slice(index + 1, index + 3), 16));
      index += 3;
    } else {
      const character = String.fromCodePoint(input.codePointAt(index));
      bytes.push(...new TextEncoder().encode(character));
      index += character.length;
    }
  }
  return new Uint8Array(bytes);
}

export default { parse, decode, stringify, encode, escape, unescape, unescapeBuffer };
