// Conversions of JavaScript values to strings and bytes.

// ToString, refusing a Symbol as WebIDL does.
export function string(value) {
  if (typeof value === "symbol") throw new TypeError("Cannot convert a Symbol value to a string");
  return String(value);
}

// ToString with lone surrogates replaced, as WebIDL's USVString does.
export function usvString(value) {
  return string(value).toWellFormed();
}
