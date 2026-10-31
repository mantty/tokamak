// A function, which may be called with any receiver and arguments.
export type AnyFunction = (this: unknown, ...args: unknown[]) => unknown;

// Runtime-owned brands cannot be forged or observed through a prototype/getter.
export const hostObjectKinds = new WeakMap<object, string>();

export function defineHiddenProperty(object: object, name: PropertyKey, value: unknown): void {
  Object.defineProperty(object, name, { configurable: true, enumerable: false, writable: true, value });
}

export function markHostObject<T extends object>(object: T, kind = "Unsupported"): T {
  hostObjectKinds.set(object, kind);
  return object;
}

export function isFunction(value: unknown): value is AnyFunction {
  return typeof value === "function";
}

export type TypedArray = Int8Array | Uint8Array | Uint8ClampedArray | Int16Array | Uint16Array | Int32Array | Uint32Array
  | Float16Array | Float32Array | Float64Array | BigInt64Array | BigUint64Array;

// Whether `value` is a typed array: a view that is not a DataView.
export function isTypedArray(value: unknown): value is TypedArray {
  return ArrayBuffer.isView(value) && !(value instanceof DataView);
}

// Whether `value` is an object or a function, whose properties are then read
// as values of any type.
export function isObject(value: unknown): value is Record<PropertyKey, unknown> {
  return (typeof value === "object" && value !== null) || typeof value === "function";
}

// Whether `value`, which may be a primitive, has a `Symbol.iterator` method.
export function isIterable(value: unknown): value is Iterable<unknown> {
  return value != null && typeof Object(value)[Symbol.iterator] === "function";
}

// `value` as an object: itself, or a primitive's wrapper object, whose
// properties read as the primitive's do.
export function objectOf(value: {}): Record<PropertyKey, unknown> {
  return Object(value);
}

// Calls `target[name](...args)` as JavaScript does, with a TypeError when
// there is no such method.
export function callMethod(target: unknown, name: PropertyKey, ...args: unknown[]): unknown {
  if (target == null) throw new TypeError(`Cannot read properties of ${target} (reading '${String(name)}')`);
  const method = objectOf(target)[name];
  if (typeof method !== "function") throw new TypeError(`${String(name)} is not a function`);
  return Reflect.apply(method, target, args);
}

// Whether `value` has a method `name`.
export function hasMethod<K extends PropertyKey>(value: unknown, name: K): value is Record<K, AnyFunction> {
  return isObject(value) && typeof value[name] === "function";
}
