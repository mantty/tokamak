import { cloneArrayBuffer, arrayBufferView, detachArrayBuffer, objectClass } from "tokamak:host";
import { DOMException } from "./dom-exception.ts";
import { hostObjectKinds, isIterable, isObject } from "./objects.ts";

// The copies made so far, by original, so that shared and cyclic references stay so.
type Copies = Map<object, unknown>;

// How a cloneable host object kind is copied: `copy(value, clone, keep)` returns the copy,
// passing it through `keep` before copying nested values with `clone`.
export type HostObjectCopy = (value: object, clone: (nested: unknown) => unknown, keep: <T>(kept: T) => T) => unknown;
export const hostObjectCopies = new Map<string, HostObjectCopy>();

type ViewConstructor = { new (buffer: ArrayBufferLike, byteOffset?: number, length?: number): object; BYTES_PER_ELEMENT?: number };
const errors = new Map<string, ErrorConstructor>([["Error", Error], ["EvalError", EvalError], ["RangeError", RangeError], ["ReferenceError", ReferenceError], ["SyntaxError", SyntaxError], ["TypeError", TypeError], ["URIError", URIError]]);
const typedArrays = new Map<string, ViewConstructor>([Int8Array, Uint8Array, Uint8ClampedArray, Int16Array, Uint16Array, Int32Array, Uint32Array, Float16Array, Float32Array, Float64Array, BigInt64Array, BigUint64Array].map(Type => [Type.name, Type]));
const typedBuffer = getter(Object.getPrototypeOf(Uint8Array.prototype), "buffer");
const dataBuffer = getter(DataView.prototype, "buffer");
const regexpSource = getter(RegExp.prototype, "source");
const regexpFlags = ["hasIndices", "global", "ignoreCase", "multiline", "dotAll", "unicode", "unicodeSets", "sticky"]
  .map((name, index) => [Object.getOwnPropertyDescriptor(RegExp.prototype, name)?.get, "dgimsuvy"[index]] as const);
const boxedValues = new Map<string, (this: unknown) => unknown>([Number, String, Boolean, BigInt].map(Type => [Type.name, Type.prototype.valueOf]));

// The getter of the accessor `name` on `prototype`.
function getter(prototype: object, name: string): (this: unknown) => unknown {
  const get = Object.getOwnPropertyDescriptor(prototype, name)?.get;
  if (get === undefined) throw new TypeError(`${name} is not an accessor`);
  return get;
}

export function uncloneable() { return new DOMException("Value cannot be cloned.", "DataCloneError"); }

function copyProperties(value: Record<PropertyKey, unknown>, copy: object, seen: Copies): void {
  for (const key of Object.keys(value)) {
    if (!Object.getOwnPropertyDescriptor(value, key)?.enumerable) continue;
    Object.defineProperty(copy, key, { value: clone(value[key], seen), writable: true, enumerable: true, configurable: true });
  }
}

function cloneError(value: Record<PropertyKey, unknown>, seen: Copies): Error {
  if (hostObjectKinds.get(value) === "DOMException") {
    const copy = new DOMException(value.message, value.name);
    seen.set(value, copy);
    return copy;
  }
  const name = value.name;
  const aggregate = name === "AggregateError";
  // Converted as the constructor converts it: undefined leaves the copy without a message.
  const { message: original } = value;
  const message = original === undefined ? undefined : `${original}`;
  const copy = aggregate ? new AggregateError([], message) : new ((typeof name === "string" ? errors.get(name) : undefined) ?? Error)(message);
  seen.set(value, copy);
  const cause = Object.getOwnPropertyDescriptor(value, "cause");
  if (cause && !cause.enumerable) Object.defineProperty(copy, "cause", { value: clone(value.cause, seen), writable: true, configurable: true });
  if (aggregate) Object.assign(copy, { errors: clone(value.errors, seen) });
  if (typeof value.stack === "string") copy.stack = value.stack;
  copyProperties(value, copy, seen);
  return copy;
}

function clone(value: unknown, seen: Copies): unknown {
  if (typeof value === "function" || typeof value === "symbol") throw uncloneable();
  if (!isObject(value)) return value;
  if (seen.has(value)) return seen.get(value);
  const kind = objectClass(value);
  const hostKind = hostObjectKinds.get(value);
  let copy: object;
  if (kind === "Array") copy = new Array(Number(value.length));
  else if (kind === "Date") copy = new Date(Date.prototype.getTime.call(value));
  else if (kind === "RegExp") copy = new RegExp(String(regexpSource.call(value)), regexpFlags.filter(([get]) => get?.call(value)).map(([, flag]) => flag).join(""));
  else if (kind === "ArrayBuffer" || kind === "SharedArrayBuffer") {
    try { copy = cloneArrayBuffer(value, kind === "SharedArrayBuffer"); }
    catch { throw uncloneable(); }
  } else if (kind === "DataView" || (kind !== undefined && typedArrays.has(kind))) {
    let view;
    try { view = arrayBufferView(value); }
    catch { throw uncloneable(); }
    // A buffer's copy is a buffer.
    const buffer = clone(view.buffer, seen) as ArrayBufferLike;
    const Type: ViewConstructor = typedArrays.get(kind) ?? DataView;
    const length = view.tracking ? undefined : view.length / (Type.BYTES_PER_ELEMENT ?? 1);
    copy = new Type(buffer, view.offset, length);
  } else if (kind !== undefined && boxedValues.has(kind)) {
    copy = Object(boxedValues.get(kind)?.call(value));
  } else if (kind === "Map") {
    const map = new Map();
    seen.set(value, map);
    for (const [key, entry] of [...Map.prototype.entries.call(value)]) map.set(clone(key, seen), clone(entry, seen));
    return map;
  } else if (kind === "Set") {
    const set = new Set();
    seen.set(value, set);
    for (const entry of [...Set.prototype.values.call(value)]) set.add(clone(entry, seen));
    return set;
  } else if (kind === "Error") return cloneError(value, seen);
  else if (kind !== "Object") throw uncloneable();
  else if (hostKind === "DOMException") return cloneError(value, seen);
  else if (hostKind !== undefined && hostObjectCopies.has(hostKind)) {
    const keep = <T,>(kept: T): T => { seen.set(value, kept); return kept; };
    return keep(hostObjectCopies.get(hostKind)?.(value, nested => clone(nested, seen), keep));
  } else {
    if (hostKind !== undefined) throw uncloneable();
    copy = {};
  }
  seen.set(value, copy);
  if (kind === "Array" || kind === "Object") copyProperties(value, copy, seen);
  return copy;
}

export function structuredClone(value: unknown, options: unknown = {}): unknown {
  if (options != null && typeof options !== "object" && typeof options !== "function") throw new TypeError("Options must be an object.");
  const transferred = new Set<ArrayBuffer>();
  const transfer = isObject(options) ? options.transfer : undefined;
  if (transfer !== undefined) {
    if (!isIterable(transfer)) throw new TypeError("The transfer value must be an iterable.");
    for (const item of transfer) {
      const buffer = ArrayBuffer.isView(item) ? (objectClass(item) === "DataView" ? dataBuffer : typedBuffer).call(item) : item;
      if (!isArrayBuffer(buffer)) throw objectClass(buffer) === "SharedArrayBuffer" ? uncloneable() : new TypeError("Only buffers and views can be transferred.");
      transferred.add(buffer);
    }
  }
  const copy = clone(value, new Map());
  for (const item of transferred) detachArrayBuffer(item);
  return copy;
}

// Whether the engine created `value` as an ArrayBuffer.
function isArrayBuffer(value: unknown): value is ArrayBuffer {
  return objectClass(value) === "ArrayBuffer";
}
