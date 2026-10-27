import { codedError } from "./errors.mjs";
import { inspect } from "./util.mjs";

export function invalidArgument(message, code = "ERR_INVALID_ARG_VALUE") {
  return codedError(TypeError, message, code);
}

function receivedValue(value) {
  if (value === null || value === undefined) return String(value);
  if (typeof value === "function" && value.name) return `function ${value.name}`;
  if (typeof value === "object") return value.constructor?.name ? `an instance of ${value.constructor.name}` : inspect(value, { depth: -1 });
  const inspected = inspect(value);
  return `type ${typeof value} (${inspected.length > 25 ? `${inspected.slice(0, 25)}...` : inspected})`;
}

// "property" for a dotted name such as "options.port", "argument" otherwise.
function nameKind(name) {
  return name.includes(".") ? "property" : "argument";
}

export function invalidType(name, expected, value) {
  return invalidArgument(`The "${name}" ${nameKind(name)} must be ${expected}. Received ${receivedValue(value)}`, "ERR_INVALID_ARG_TYPE");
}

export function validateType(value, name, type) {
  if (typeof value !== type) throw invalidType(name, `of type ${type}`, value);
}

export function validateObject(value, name) {
  if (value === null || Array.isArray(value) || typeof value !== "object") throw invalidType(name, "of type object", value);
}

export function validateFunction(value, name) {
  if (typeof value !== "function") throw invalidType(name, "of type function", value);
}

export function validateAbortSignal(signal, name) {
  if (signal !== undefined && (signal === null || typeof signal !== "object" || !("aborted" in signal))) {
    throw invalidType(name, "an instance of AbortSignal", signal);
  }
}

export function invalidValue(name, value, reason) {
  return invalidArgument(`The ${nameKind(name)} '${name}' ${reason}. Received ${inspect(value)}`);
}

export function outOfRange(name, range, value) {
  return codedError(RangeError, `The value of "${name}" is out of range. It must be ${range}. Received ${inspect(value)}`, "ERR_OUT_OF_RANGE");
}
