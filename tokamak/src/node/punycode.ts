import { codePoints, toNumber } from "../globals/conversions.ts";
import { isIterable } from "../globals/objects.ts";

const base = 36;
const tMin = 1;
const tMax = 26;
const skew = 38;
const damp = 700;
const initialBias = 72;
const initialN = 128;
const delimiter = "-";

function digit(value: number): string { return value < 26 ? String.fromCharCode(97 + value) : String.fromCharCode(22 + value); }
function value(character: string): number { const code = character.toLowerCase().charCodeAt(0); return code >= 97 ? code - 97 : code - 22; }
function threshold(k: number, bias: number): number { return k <= bias ? tMin : k >= bias + tMax ? tMax : k - bias; }

function adapt(delta: number, points: number, first: boolean): number {
  delta = first ? Math.floor(delta / damp) : Math.floor(delta / 2);
  delta += Math.floor(delta / points);
  let k = 0;
  while (delta > Math.floor(((base - tMin) * tMax) / 2)) { delta = Math.floor(delta / (base - tMin)); k += base; }
  return k + Math.floor(((base - tMin + 1) * delta) / (delta + skew));
}

export function encode(input: unknown): string {
  const points = codePoints(String(input));
  let output = points.filter(point => point < initialN).map(point => String.fromCodePoint(point)).join("");
  let basic = output.length;
  let handled = basic;
  if (basic > 0) output += delimiter;
  let n = initialN;
  let delta = 0;
  let bias = initialBias;
  while (handled < points.length) {
    const next = Math.min(...points.filter(point => point >= n));
    delta += (next - n) * (handled + 1);
    n = next;
    for (const point of points) {
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

export function decode(input: unknown): string {
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

function mapLabels(value: unknown, transform: (label: string) => string): string { return String(value).split(".").map(label => transform(label)).join("."); }
export function toASCII(value: unknown): string { return mapLabels(value, label => { const lower = label.toLowerCase(); return codePoints(lower).some(point => point >= 128) ? `xn--${encode(lower)}` : lower; }); }
export function toUnicode(value: unknown): string { return mapLabels(value, label => label.toLowerCase().startsWith("xn--") ? decode(label.slice(4)) : label.toLowerCase()); }
export const ucs2 = {
  decode: (value: unknown) => codePoints(String(value)),
  encode: (value: unknown) => {
    if (!isIterable(value)) throw new TypeError("value is not iterable");
    return String.fromCodePoint(...Array.from(value, toNumber));
  },
};
export const version = "5.1.1";
export default { encode, decode, toASCII, toUnicode, ucs2, version };
