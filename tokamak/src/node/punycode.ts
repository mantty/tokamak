const base = 36;
const tMin = 1;
const tMax = 26;
const skew = 38;
const damp = 700;
const initialBias = 72;
const initialN = 128;
const delimiter = "-";

function digit(value) { return value < 26 ? String.fromCharCode(97 + value) : String.fromCharCode(22 + value); }
function value(character) { const code = character.toLowerCase().charCodeAt(0); return code >= 97 ? code - 97 : code - 22; }
function threshold(k, bias) { return k <= bias ? tMin : k >= bias + tMax ? tMax : k - bias; }

function adapt(delta, points, first) {
  delta = first ? Math.floor(delta / damp) : Math.floor(delta / 2);
  delta += Math.floor(delta / points);
  let k = 0;
  while (delta > Math.floor(((base - tMin) * tMax) / 2)) { delta = Math.floor(delta / (base - tMin)); k += base; }
  return k + Math.floor(((base - tMin + 1) * delta) / (delta + skew));
}

export function encode(input) {
  const codePoints = [...String(input)].map(character => character.codePointAt(0));
  let output = codePoints.filter(point => point < initialN).map(point => String.fromCodePoint(point)).join("");
  let basic = output.length;
  let handled = basic;
  if (basic > 0) output += delimiter;
  let n = initialN;
  let delta = 0;
  let bias = initialBias;
  while (handled < codePoints.length) {
    const next = Math.min(...codePoints.filter(point => point >= n));
    delta += (next - n) * (handled + 1);
    n = next;
    for (const point of codePoints) {
      if (point < n) delta += 1;
      if (point !== n) continue;
      let number = delta;
      for (let k = base;; k += base) {
        const limit = threshold(k, bias);
        if (number < limit) break;
        output += digit(limit + ((number - limit) % (base - limit)));
        number = Math.floor((number - limit) / (base - limit));
      }
      output += digit(number);
      bias = adapt(delta, handled + 1, handled === basic);
      delta = 0;
      handled += 1;
    }
    delta += 1;
    n += 1;
  }
  return output.replace(/-$/, "");
}

export function decode(input) {
  const valueInput = String(input);
  const delimiterIndex = valueInput.lastIndexOf(delimiter);
  const output = delimiterIndex < 0 ? [] : [...valueInput.slice(0, delimiterIndex)].map(character => character.charCodeAt(0));
  let index = delimiterIndex < 0 ? 0 : delimiterIndex + 1;
  let n = initialN;
  let bias = initialBias;
  let insertion = 0;
  while (index < valueInput.length) {
    const oldInsertion = insertion;
    let weight = 1;
    let number = insertion;
    for (let k = base;; k += base) {
      if (index >= valueInput.length) throw new RangeError("Invalid input");
      const current = value(valueInput[index++]);
      number += current * weight;
      const limit = threshold(k, bias);
      if (current < limit) break;
      weight *= base - limit;
    }
    const points = output.length + 1;
    bias = adapt(number - oldInsertion, points, oldInsertion === 0);
    n += Math.floor(number / points);
    insertion = number % points;
    output.splice(insertion, 0, n);
    insertion += 1;
  }
  return String.fromCodePoint(...output);
}

function mapLabels(value, transform) { return String(value).split(".").map(transform).join("."); }
export function toASCII(value) { return mapLabels(value, label => { const lower = label.toLowerCase(); return [...lower].some(character => character.codePointAt(0) >= 128) ? `xn--${encode(lower)}` : lower; }); }
export function toUnicode(value) { return mapLabels(value, label => label.toLowerCase().startsWith("xn--") ? decode(label.slice(4)) : label.toLowerCase()); }
export const ucs2 = { decode: value => [...String(value)].map(character => character.codePointAt(0)), encode: value => String.fromCodePoint(...value) };
export const version = "5.1.1";
export default { encode, decode, toASCII, toUnicode, ucs2, version };
