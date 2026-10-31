// Runtime-owned brands cannot be forged or observed through a prototype/getter.
export const hostObjectKinds = new WeakMap();

export function defineHiddenProperty(object, name, value) {
  Object.defineProperty(object, name, { configurable: true, enumerable: false, writable: true, value });
}

export function markHostObject(object, kind = "Unsupported") {
  hostObjectKinds.set(object, kind);
  return object;
}
