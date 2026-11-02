import { objectClass } from "tokamak:host";
import { bufferBytes } from "../globals/conversions.ts";
import { isTypedArray, propertyOf } from "../globals/objects.ts";

// The objects each side is comparing on the way to the current pair, each
// with the object it is compared against.
interface Paths {
  actual: Map<object, object>;
  expected: Map<object, object>;
}

// Deep equality as node:assert and node:util define it. Partial equality asks
// only that `actual` hold `expected`'s keys, entries and leading elements.
export function isDeepStrictEqual(actual: unknown, expected: unknown): boolean { return equal(actual, expected, false, paths()); }
export function isPartialDeepStrictEqual(actual: unknown, expected: unknown): boolean { return equal(actual, expected, true, paths()); }

const toStringTag = Object.prototype.toString;
const propertyIsEnumerable = Object.prototype.propertyIsEnumerable;
const getTime = Date.prototype.getTime;
const primitiveValues = new Map<string, () => unknown>([
  ["Number", Number.prototype.valueOf], ["String", String.prototype.valueOf], ["Boolean", Boolean.prototype.valueOf],
  ["Symbol", Symbol.prototype.valueOf], ["BigInt", BigInt.prototype.valueOf],
]);
const comparedKinds = new Set(["Date", "RegExp", "Error", "Map", "Set", "ArrayBuffer", "SharedArrayBuffer", ...primitiveValues.keys()]);
// Maps and sets compare by their engine contents, as Node's do.
const mapEntries = Map.prototype.entries, mapHas = Map.prototype.has, mapGet = Map.prototype.get;
const setValues = Set.prototype.values, setHas = Set.prototype.has;

function paths(): Paths { return { actual: new Map(), expected: new Map() }; }

function isObject(value: unknown): value is object { return typeof value === "object" && value !== null; }

// The engine class that decides how an object compares; "Object" for others.
function kindOf(value: object): string {
  if (Array.isArray(value)) return "Array";
  const kind = objectClass(value);
  if (kind === undefined) return "Object";
  return comparedKinds.has(kind) || ArrayBuffer.isView(value) ? kind : "Object";
}

function equal(actual: unknown, expected: unknown, partial: boolean, paths: Paths): boolean {
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

// Whether `actual` and `expected` have the same values of `names`.
function sameProperties(actual: object, expected: object, names: readonly string[]): boolean {
  return names.every(name => propertyOf(actual, name) === propertyOf(expected, name));
}

function sameIntrinsics(actual: object, expected: object, kind: string, partial: boolean): boolean {
  if (Array.isArray(actual) && Array.isArray(expected)) return partial || actual.length === expected.length;
  if (kind === "Date") return getTime.call(actual) === getTime.call(expected);
  if (kind === "RegExp") return sameProperties(actual, expected, ["source", "flags", "lastIndex"]);
  if (kind === "Error") return sameProperties(actual, expected, ["message", "name"]);
  const valueOf = primitiveValues.get(kind);
  if (valueOf) return Object.is(valueOf.call(actual), valueOf.call(expected));
  const bytes = bytesOf(expected, kind);
  if (bytes === undefined) return true;
  const actualBytes = bytesOf(actual, kind);
  return actualBytes !== undefined && sameBytes(actualBytes, bytes, partial);
}

function bytesOf(value: object, kind: string): Uint8Array | undefined {
  if (kind === "ArrayBuffer" || kind === "SharedArrayBuffer" || ArrayBuffer.isView(value)) return bufferBytes(value);
}

function sameBytes(actual: Uint8Array, expected: Uint8Array, partial: boolean): boolean {
  if (partial ? expected.length > actual.length : expected.length !== actual.length) return false;
  return expected.every((byte, index) => byte === actual[index]);
}

// Own enumerable keys, leaving out a typed array's elements, which come first
// and compare as bytes.
function ownKeys(value: object): (string | symbol)[] {
  const keys = Reflect.ownKeys(value);
  return (isTypedArray(value) ? keys.slice(value.length) : keys).filter(key => propertyIsEnumerable.call(value, key));
}

function sameKeys(actual: object, expected: object, partial: boolean, paths: Paths): boolean {
  const keys = ownKeys(expected);
  if (!partial && keys.length !== ownKeys(actual).length) return false;
  return keys.every(key => propertyIsEnumerable.call(actual, key)
    && equal(Reflect.get(actual, key), Reflect.get(expected, key), partial, paths));
}

function sameMembers(actual: object, expected: object, kind: string, partial: boolean, paths: Paths): boolean {
  if (kind === "Map") return sameEntries(actual, expected, partial, paths);
  if (kind === "Set") return sameValues(actual, expected, partial, paths);
  return true;
}

// Object keys match by deep equality, each `actual` entry at most once.
function sameEntries(actual: object, expected: object, partial: boolean, paths: Paths): boolean {
  const actualEntries = [...mapEntries.call(actual)], expectedEntries = [...mapEntries.call(expected)];
  if (!partial && actualEntries.length !== expectedEntries.length) return false;
  const candidates = actualEntries.filter(([key]) => isObject(key));
  return expectedEntries.every(([key, value]) => isObject(key)
    ? take(candidates, ([other, otherValue]) => equal(other, key, false, paths) && equal(otherValue, value, partial, paths))
    : mapHas.call(actual, key) && equal(mapGet.call(actual, key), value, partial, paths));
}

// Object members match by deep equality, each `actual` member at most once.
function sameValues(actual: object, expected: object, partial: boolean, paths: Paths): boolean {
  const actualValues = [...setValues.call(actual)], expectedValues = [...setValues.call(expected)];
  if (!partial && actualValues.length !== expectedValues.length) return false;
  const candidates = actualValues.filter(isObject);
  return expectedValues.every(value => isObject(value) ? take(candidates, other => equal(other, value, false, paths)) : setHas.call(actual, value));
}

// Whether a candidate `matches`, removing the first that does.
function take<T>(candidates: T[], matches: (candidate: T) => boolean): boolean {
  const index = candidates.findIndex(matches);
  if (index >= 0) candidates.splice(index, 1);
  return index >= 0;
}
