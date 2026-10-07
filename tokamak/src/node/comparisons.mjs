import { objectClass } from "tokamak:host";

// Deep equality as node:assert and node:util define it. Partial equality asks
// only that `actual` hold `expected`'s keys, entries and leading elements.
export function isDeepStrictEqual(actual, expected) { return equal(actual, expected, false, new Paths()); }
export function isPartialDeepStrictEqual(actual, expected) { return equal(actual, expected, true, new Paths()); }

const toStringTag = Object.prototype.toString;
const propertyIsEnumerable = Object.prototype.propertyIsEnumerable;
const getTime = Date.prototype.getTime;
const primitiveValues = {
  Number: Number.prototype.valueOf, String: String.prototype.valueOf, Boolean: Boolean.prototype.valueOf,
  Symbol: Symbol.prototype.valueOf, BigInt: BigInt.prototype.valueOf,
};
const comparedKinds = new Set(["Date", "RegExp", "Error", "Map", "Set", "ArrayBuffer", "SharedArrayBuffer", ...Object.keys(primitiveValues)]);

// The objects each side is comparing on the way to the current pair, each
// with the object it is compared against.
class Paths {
  actual = new Map();
  expected = new Map();
}

function isObject(value) { return typeof value === "object" && value !== null; }

// The engine class that decides how an object compares; "Object" for others.
function kindOf(value) {
  if (Array.isArray(value)) return "Array";
  const kind = objectClass(value);
  return comparedKinds.has(kind) || ArrayBuffer.isView(value) ? kind : "Object";
}

function equal(actual, expected, partial, paths) {
  if (Object.is(actual, expected)) return true;
  if (!isObject(actual) || !isObject(expected)) return false;
  if (Object.getPrototypeOf(actual) !== Object.getPrototypeOf(expected)) return false;
  if (toStringTag.call(actual) !== toStringTag.call(expected)) return false;
  const kind = kindOf(actual);
  if (kind !== kindOf(expected) || !sameIntrinsics(actual, expected, kind, partial)) return false;
  if (paths.actual.has(actual) || paths.expected.has(expected)) return paths.actual.get(actual) === expected;
  paths.actual.set(actual, expected);
  paths.expected.set(expected, actual);
  const same = sameKeys(actual, expected, partial, paths) && sameMembers(actual, expected, kind, partial, paths);
  paths.actual.delete(actual);
  paths.expected.delete(expected);
  return same;
}

function sameIntrinsics(actual, expected, kind, partial) {
  if (kind === "Array") return partial || actual.length === expected.length;
  if (kind === "Date") return getTime.call(actual) === getTime.call(expected);
  if (kind === "RegExp") return actual.source === expected.source && actual.flags === expected.flags && actual.lastIndex === expected.lastIndex;
  if (kind === "Error") return actual.message === expected.message && actual.name === expected.name;
  if (Object.hasOwn(primitiveValues, kind)) return Object.is(primitiveValues[kind].call(actual), primitiveValues[kind].call(expected));
  const bytes = bytesOf(expected, kind);
  return bytes === undefined || sameBytes(bytesOf(actual, kind), bytes, partial);
}

function bytesOf(value, kind) {
  if (kind === "ArrayBuffer" || kind === "SharedArrayBuffer") return new Uint8Array(value);
  if (ArrayBuffer.isView(value)) return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
}

function sameBytes(actual, expected, partial) {
  if (partial ? expected.length > actual.length : expected.length !== actual.length) return false;
  return expected.every((byte, index) => byte === actual[index]);
}

// Own enumerable keys, leaving out a typed array's elements, which compare as bytes.
function ownKeys(value) {
  const keys = Reflect.ownKeys(value).filter(key => propertyIsEnumerable.call(value, key));
  if (!ArrayBuffer.isView(value)) return keys;
  return keys.filter(key => typeof key === "symbol" || String(Number(key)) !== key);
}

function sameKeys(actual, expected, partial, paths) {
  const keys = ownKeys(expected);
  if (!partial && keys.length !== ownKeys(actual).length) return false;
  return keys.every(key => propertyIsEnumerable.call(actual, key) && equal(actual[key], expected[key], partial, paths));
}

function sameMembers(actual, expected, kind, partial, paths) {
  if (kind !== "Map" && kind !== "Set") return true;
  if (!partial && actual.size !== expected.size) return false;
  return kind === "Map" ? sameEntries(actual, expected, partial, paths) : sameValues(actual, expected, paths);
}

// Object keys match by deep equality, each `actual` entry at most once.
function sameEntries(actual, expected, partial, paths) {
  const candidates = [...actual].filter(([key]) => isObject(key));
  return [...expected].every(([key, value]) => isObject(key)
    ? take(candidates, ([other, otherValue]) => equal(other, key, false, paths) && equal(otherValue, value, partial, paths))
    : actual.has(key) && equal(actual.get(key), value, partial, paths));
}

// Object members match by deep equality, each `actual` member at most once.
function sameValues(actual, expected, paths) {
  const candidates = [...actual].filter(isObject);
  return [...expected].every(value => isObject(value) ? take(candidates, other => equal(other, value, false, paths)) : actual.has(value));
}

// Whether a candidate `matches`, removing the first that does.
function take(candidates, matches) {
  const index = candidates.findIndex(matches);
  if (index >= 0) candidates.splice(index, 1);
  return index >= 0;
}
