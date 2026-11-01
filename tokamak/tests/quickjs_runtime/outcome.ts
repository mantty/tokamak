// The member `name` of `value`, read as `value?.[name]` reads it.
export function member(value: unknown, name: PropertyKey): unknown {
  return value == null ? undefined : Reflect.get(Object(value), name);
}

// Whether `value` is an object, whose members can be read by name.
export function isObject(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object";
}
// The name and code of a thrown error.
export function failure(error: unknown) {
  return { error: member(error, "name"), code: member(error, "code") ?? null };
}

// What `callback` returns, or the failure it throws.
export function outcome<T>(callback: () => T) {
  try { return callback(); }
  catch (error) { return failure(error); }
}

// What `callback` resolves to, or the failure it throws or rejects with.
export async function asyncOutcome<T>(callback: () => T | Promise<T>) {
  try { return await callback(); }
  catch (error) { return failure(error); }
}

// `value`, which a handler or callback has set by now. A missing value throws
// the TypeError that reading a member of it would.
export function present<T>(value: T | null | undefined): T {
  if (value == null) throw new TypeError("The value is not set");
  return value;
}
