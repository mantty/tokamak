// Conversions of JavaScript values to strings and bytes.

// ToString, refusing a Symbol as WebIDL does.
export function string(value) {
  if (typeof value === "symbol") throw new TypeError("Cannot convert a Symbol value to a string");
  return String(value);
}
