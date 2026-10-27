// workerd's node-internal:validators, and the Node errors they throw.
import { codedError } from "./errors.mjs";
import { inspect } from "./util.mjs";

const kTypes = ["string", "function", "number", "object", "Function", "Object", "boolean", "bigint", "symbol"];
const classRegExp = /^([A-Z][a-z0-9]*)+$/;

// `values` joined as Node's messages list alternatives.
function alternatives(values) {
  if (values.length > 2) return `${values.slice(0, -1).join(", ")}, or ${values.at(-1)}`;
  return values.join(" or ");
}

function expectedTypes(expected) {
  const types = [];
  const instances = [];
  const other = [];
  for (const value of [expected].flat()) {
    if (kTypes.includes(value)) types.push(value.toLowerCase());
    else if (classRegExp.test(value)) instances.push(value);
    else other.push(value);
  }
  if (instances.length > 0 && types.includes("object")) {
    types.splice(types.indexOf("object"), 1);
    instances.push("Object");
  }
  const parts = [];
  if (types.length > 0) parts.push(`${types.length > 1 ? "one of type" : "of type"} ${alternatives(types)}`);
  if (instances.length > 0) parts.push(`an instance of ${alternatives(instances)}`);
  if (other.length > 1) parts.push(`one of ${alternatives(other)}`);
  if (other.length === 1) parts.push(`${other[0].toLowerCase() !== other[0] ? "an " : ""}${other[0]}`);
  return parts.join(" or ");
}

function receivedType(value) {
  if (value == null) return `${value}`;
  if (typeof value === "function" && value.name) return `function ${value.name}`;
  if (typeof value === "object") {
    return value.constructor?.name ? `an instance of ${value.constructor.name}` : inspect(value, { depth: -1 });
  }
  const inspected = inspect(value, { colors: false });
  return `type ${typeof value} (${inspected.length > 25 ? `${inspected.slice(0, 25)}...` : inspected})`;
}

function separated(digits) {
  const start = digits[0] === "-" ? 1 : 0;
  let end = digits.length;
  let groups = "";
  for (; end >= start + 4; end -= 3) groups = `_${digits.slice(end - 3, end)}${groups}`;
  return `${digits.slice(0, end)}${groups}`;
}

function receivedRange(value) {
  if (Number.isInteger(value) && Math.abs(value) > 2 ** 32) return separated(String(value));
  if (typeof value !== "bigint") return inspect(value);
  const large = value > 2n ** 32n || value < -(2n ** 32n);
  return `${large ? separated(String(value)) : value}n`;
}

export function invalidArgType(name, expected, value) {
  const subject = name.endsWith(" argument") ? name : `"${name}" ${name.includes(".") ? "property" : "argument"}`;
  return codedError(TypeError, `The ${subject} must be ${expectedTypes(expected)}. Received ${receivedType(value)}`, "ERR_INVALID_ARG_TYPE");
}

export function invalidArgValue(name, value, reason = "is invalid") {
  const subject = name.includes(".") ? "property" : "argument";
  return codedError(TypeError, `The ${subject} '${name}' ${reason}. Received ${inspect(value)}`, "ERR_INVALID_ARG_VALUE");
}

export function outOfRange(name, range, value) {
  return codedError(RangeError, `The value of "${name}" is out of range. It must be ${range}. Received ${receivedRange(value)}`, "ERR_OUT_OF_RANGE");
}

export function validateObject(value, name) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) throw invalidArgType(name, "Object", value);
}

export function validateBoolean(value, name) {
  if (typeof value !== "boolean") throw invalidArgType(name, "boolean", value);
}

export function validateString(value, name) {
  if (typeof value !== "string") throw invalidArgType(name, "string", value);
}

export function validateNumber(value, name) {
  if (typeof value !== "number") throw invalidArgType(name, "number", value);
}

export function validateFunction(value, name) {
  if (typeof value !== "function") throw invalidArgType(name, "Function", value);
}

export function validateAbortSignal(signal, name) {
  if (signal !== undefined && (signal === null || typeof signal !== "object" || !("aborted" in signal))) {
    throw invalidArgType(name, "AbortSignal", signal);
  }
}

export function validateInt32(value, name, min = -2147483648, max = 2147483647) {
  if (typeof value !== "number") throw invalidArgType(name, "number", value);
  if (!Number.isInteger(value)) throw outOfRange(name, "an integer", value);
  if (value !== (value | 0) || value < min || value > max) throw outOfRange(name, `>= ${min} && <= ${max}`, value);
}

export function validateUint32(value, name, positive) {
  if (value !== value >>> 0) {
    if (typeof value !== "number") throw invalidArgType(name, "number", value);
    if (!Number.isInteger(value)) throw outOfRange(name, "an integer", value);
    throw outOfRange(name, `>= ${positive ? 1 : 0} && < 4294967296`, value);
  }
  if (positive && value === 0) throw outOfRange(name, ">= 1 && < 4294967296", value);
}

export function parseFileMode(value, name, defaultMode) {
  if (defaultMode != null) value ??= defaultMode;
  if (typeof value === "string") {
    if (!/^[0-7]+$/.test(value)) throw invalidArgValue(name, value, "must be a 32-bit unsigned integer or an octal string");
    value = Number.parseInt(value, 8);
  }
  validateUint32(value, name);
  return value;
}

export function validateThisInternalField(object, fieldKey, className) {
  if (typeof object !== "object" || object === null || !Object.prototype.hasOwnProperty.call(object, fieldKey)) {
    throw codedError(TypeError, `Value of "this" must be of type ${className}`, "ERR_INVALID_THIS");
  }
}
