// Runtime-owned brands cannot be forged or observed through a prototype/getter.
export const hostObjectKinds = new WeakMap();

export function markHostObject(object, kind = "Unsupported") {
  hostObjectKinds.set(object, kind);
  return object;
}
