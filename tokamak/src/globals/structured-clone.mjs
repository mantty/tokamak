import { cloneArrayBuffer, arrayBufferView, detachArrayBuffer, objectClass } from "tokamak:host";
import { Blob, Headers, Request, Response } from "../network/fetch.mjs";
import { DOMException } from "./web.mjs";
import { hostObjectKinds } from "./objects.mjs";

const errors = { Error, EvalError, RangeError, ReferenceError, SyntaxError, TypeError, URIError, AggregateError };
const typedArrays = Object.fromEntries(["Int8Array", "Uint8Array", "Uint8ClampedArray", "Int16Array", "Uint16Array", "Int32Array", "Uint32Array", "Float16Array", "Float32Array", "Float64Array", "BigInt64Array", "BigUint64Array"].map(name => [name, globalThis[name]]));
const typedBuffer = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(Uint8Array.prototype), "buffer").get;
const dataBuffer = Object.getOwnPropertyDescriptor(DataView.prototype, "buffer").get;
const regexpSource = Object.getOwnPropertyDescriptor(RegExp.prototype, "source").get;
const regexpFlags = ["hasIndices", "global", "ignoreCase", "multiline", "dotAll", "unicode", "unicodeSets", "sticky"]
  .map((name, index) => [Object.getOwnPropertyDescriptor(RegExp.prototype, name)?.get, "dgimsuvy"[index]]);
const boxedValues = Object.fromEntries([Number, String, Boolean, BigInt].map(Type => [Type.name, Type.prototype.valueOf]));

function uncloneable() { return new DOMException("Value cannot be cloned.", "DataCloneError"); }

function copyProperties(value, copy, seen) {
  for (const key of Object.keys(value)) {
    if (!Object.getOwnPropertyDescriptor(value, key)?.enumerable) continue;
    Object.defineProperty(copy, key, { value: clone(value[key], seen), writable: true, enumerable: true, configurable: true });
  }
}

function cloneError(value, seen) {
  if (hostObjectKinds.get(value) === "DOMException") {
    const copy = new DOMException(value.message, value.name);
    seen.set(value, copy);
    return copy;
  }
  const Constructor = Object.hasOwn(errors, value.name) ? errors[value.name] : Error;
  const copy = Constructor === AggregateError ? new AggregateError([], value.message) : new Constructor(value.message);
  seen.set(value, copy);
  const cause = Object.getOwnPropertyDescriptor(value, "cause");
  if (cause && !cause.enumerable) Object.defineProperty(copy, "cause", { value: clone(value.cause, seen), writable: true, configurable: true });
  if (Constructor === AggregateError) copy.errors = clone(value.errors, seen);
  if (typeof value.stack === "string") copy.stack = value.stack;
  copyProperties(value, copy, seen);
  return copy;
}

function clone(value, seen) {
  if (typeof value === "function" || typeof value === "symbol") throw uncloneable();
  if (value === null || typeof value !== "object") return value;
  if (seen.has(value)) return seen.get(value);
  const kind = objectClass(value);
  const hostKind = hostObjectKinds.get(value);
  let copy;
  if (kind === "Array") copy = new Array(value.length);
  else if (kind === "Date") copy = new Date(Date.prototype.getTime.call(value));
  else if (kind === "RegExp") copy = new RegExp(regexpSource.call(value), regexpFlags.filter(([get]) => get?.call(value)).map(([, flag]) => flag).join(""));
  else if (kind === "ArrayBuffer" || kind === "SharedArrayBuffer") {
    try { copy = cloneArrayBuffer(value, kind === "SharedArrayBuffer"); }
    catch { throw uncloneable(); }
  } else if (kind === "DataView" || Object.hasOwn(typedArrays, kind)) {
    let view;
    try { view = arrayBufferView(value); }
    catch { throw uncloneable(); }
    const buffer = clone(view.buffer, seen);
    const Type = kind === "DataView" ? DataView : typedArrays[kind];
    const length = view.tracking ? undefined : view.length / (Type.BYTES_PER_ELEMENT ?? 1);
    copy = new Type(buffer, view.offset, length);
  } else if (kind === "Number" || kind === "String" || kind === "Boolean" || kind === "BigInt") {
    copy = Object(boxedValues[kind].call(value));
  } else if (kind === "Map") copy = new Map();
  else if (kind === "Set") copy = new Set();
  else if (kind === "Error") return cloneError(value, seen);
  else if (kind !== "Object") throw uncloneable();
  else if (hostKind === "DOMException") return cloneError(value, seen);
  else if (hostKind === "Blob" || hostKind === "Headers") {
    copy = hostKind === "Blob" ? new Blob([value.__bytes], { type: value.__type }) : new Headers([...value.__values].flatMap(([name, values]) => values.map(entry => [name, entry])));
    seen.set(value, copy);
    return copy;
  }
  else if (hostKind === "Request" || hostKind === "Response") {
    if (value.__bodyStream !== null || value.__signalProvided || value.__webSocket) throw uncloneable();
    copy = hostKind === "Request" ? new Request(value.__url, { method: value.__method, headers: value.__headers, redirect: value.__redirect, cache: value.__cache })
      : value.__status === 0 ? Response.error() : new Response(null, { status: value.__status, statusText: value.__statusText, headers: value.__headers });
    seen.set(value, copy);
    if (value.__cf !== undefined) copy.__cf = clone(value.__cf, seen);
    return copy;
  } else {
    if (hostKind !== undefined) throw uncloneable();
    copy = {};
  }
  seen.set(value, copy);
  if (kind === "Map") for (const [key, entry] of [...Map.prototype.entries.call(value)]) copy.set(clone(key, seen), clone(entry, seen));
  else if (kind === "Set") for (const entry of [...Set.prototype.values.call(value)]) copy.add(clone(entry, seen));
  else if (kind === "Array" || kind === "Object") copyProperties(value, copy, seen);
  return copy;
}

export function structuredClone(value, options = {}) {
  if (options != null && typeof options !== "object" && typeof options !== "function") throw new TypeError("Options must be an object.");
  const transferred = new Set();
  const transfer = options?.transfer;
  if (transfer !== undefined) {
    if (transfer == null || typeof transfer[Symbol.iterator] !== "function") throw new TypeError("The transfer value must be an iterable.");
    for (const item of transfer) {
      const buffer = ArrayBuffer.isView(item) ? (objectClass(item) === "DataView" ? dataBuffer : typedBuffer).call(item) : item;
      const kind = objectClass(buffer);
      if (kind === "SharedArrayBuffer") throw uncloneable();
      if (kind !== "ArrayBuffer") throw new TypeError("Only buffers and views can be transferred.");
      transferred.add(buffer);
    }
  }
  const copy = clone(value, new Map());
  for (const item of transferred) detachArrayBuffer(item);
  return copy;
}
