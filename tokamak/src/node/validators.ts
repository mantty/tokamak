// workerd's node-internal:validators, and the Node errors they throw.
import { codedError } from "./errors.ts";
import type { AnyFunction } from "../globals/objects.ts";
import { inspect } from "./util.ts";


// An AbortSignal as Node's APIs use one, although they only check that it has
// an `aborted` property.
export interface AbortSignalLike {
  readonly aborted: unknown;
  readonly reason?: unknown;
  addEventListener(type: "abort", listener: AnyFunction, options?: { once?: boolean }): unknown;
  removeEventListener(type: "abort", listener: AnyFunction): unknown;
}

const kTypes = ["string", "function", "number", "object", "Function", "Object", "boolean", "bigint", "symbol"];
const classRegExp = /^([A-Z][a-z0-9]*)+$/;

// `values` joined as Node's messages list alternatives.
function alternatives(values: string[]): string {
  if (values.length > 2) return `${values.slice(0, -1).join(", ")}, or ${values.at(-1)}`;
  return values.join(" or ");
}

function expectedTypes(expected: string | string[]): string {
  const types: string[] = [];
  const instances: string[] = [];
  const other: string[] = [];
  for (const value of [expected].flat()) {
    if (kTypes.includes(value)) types.push(value.toLowerCase());
    else if (classRegExp.test(value)) instances.push(value);
    else other.push(value);
  }
  if (instances.length > 0 && types.includes("object")) {
    types.splice(types.indexOf("object"), 1);
    instances.push("Object");
  }
  const parts: string[] = [];
  if (types.length > 0) parts.push(`${types.length > 1 ? "one of type" : "of type"} ${alternatives(types)}`);
  if (instances.length > 0) parts.push(`an instance of ${alternatives(instances)}`);
  if (other.length > 1) parts.push(`one of ${alternatives(other)}`);
  if (other.length === 1) parts.push(`${other[0].toLowerCase() !== other[0] ? "an " : ""}${other[0]}`);
  return parts.join(" or ");
}

function receivedType(value: unknown): string {
  if (value == null) return `${value}`;
  if (typeof value === "function" && value.name) return `function ${value.name}`;
  if (typeof value === "object") {
    return value.constructor?.name ? `an instance of ${value.constructor.name}` : inspect(value, { depth: -1 });
  }
  const inspected = inspect(value, { colors: false });
  return `type ${typeof value} (${inspected.length > 25 ? `${inspected.slice(0, 25)}...` : inspected})`;
}

function separated(digits: string): string {
  const start = digits[0] === "-" ? 1 : 0;
  let end = digits.length;
  let groups = "";
  for (; end >= start + 4; end -= 3) groups = `_${digits.slice(end - 3, end)}${groups}`;
  return `${digits.slice(0, end)}${groups}`;
}

function receivedRange(value: unknown): string {
  if (typeof value === "number" && Number.isInteger(value) && Math.abs(value) > 2 ** 32) return separated(String(value));
  if (typeof value !== "bigint") return inspect(value);
  const large = value > 2n ** 32n || value < -(2n ** 32n);
  return `${large ? separated(String(value)) : value}n`;
}

export function invalidArgType(name: string, expected: string | string[], value: unknown) {
  const subject = name.endsWith(" argument") ? name : `"${name}" ${name.includes(".") ? "property" : "argument"}`;
  return codedError(TypeError, `The ${subject} must be ${expectedTypes(expected)}. Received ${receivedType(value)}`, "ERR_INVALID_ARG_TYPE");
}

export function invalidArgValue(name: string, value: unknown, reason = "is invalid") {
  const subject = name.includes(".") ? "property" : "argument";
  return codedError(TypeError, `The ${subject} '${name}' ${reason}. Received ${inspect(value)}`, "ERR_INVALID_ARG_VALUE");
}

export function outOfRange(name: string, range: string, value: unknown) {
  return codedError(RangeError, `The value of "${name}" is out of range. It must be ${range}. Received ${receivedRange(value)}`, "ERR_OUT_OF_RANGE");
}

export function validateObject(value: unknown, name: string): asserts value is Record<PropertyKey, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) throw invalidArgType(name, "Object", value);
}

export function validateBoolean(value: unknown, name: string): asserts value is boolean {
  if (typeof value !== "boolean") throw invalidArgType(name, "boolean", value);
}

export function validateString(value: unknown, name: string): asserts value is string {
  if (typeof value !== "string") throw invalidArgType(name, "string", value);
}

export function validateNumber(value: unknown, name: string): asserts value is number {
  if (typeof value !== "number") throw invalidArgType(name, "number", value);
}

export function validateFunction(value: unknown, name: string): asserts value is AnyFunction {
  if (typeof value !== "function") throw invalidArgType(name, "Function", value);
}

export function validateAbortSignal(signal: unknown, name: string): asserts signal is AbortSignalLike | undefined {
  if (signal !== undefined && (signal === null || typeof signal !== "object" || !("aborted" in signal))) {
    throw invalidArgType(name, "AbortSignal", signal);
  }
}

export function validateInt32(value: unknown, name: string, min = -2147483648, max = 2147483647): asserts value is number {
  if (typeof value !== "number") throw invalidArgType(name, "number", value);
  if (!Number.isInteger(value)) throw outOfRange(name, "an integer", value);
  if (value !== (value | 0) || value < min || value > max) throw outOfRange(name, `>= ${min} && <= ${max}`, value);
}

export function validateUint32(value: unknown, name: string, positive?: boolean): asserts value is number {
  if (typeof value !== "number" || value !== value >>> 0) {
    if (typeof value !== "number") throw invalidArgType(name, "number", value);
    if (!Number.isInteger(value)) throw outOfRange(name, "an integer", value);
    throw outOfRange(name, `>= ${positive ? 1 : 0} && < 4294967296`, value);
  }
  if (positive && value === 0) throw outOfRange(name, ">= 1 && < 4294967296", value);
}

export function parseFileMode(mode: unknown, name: string, defaultMode?: unknown): number {
  let value = mode;
  if (defaultMode != null) value ??= defaultMode;
  if (typeof value === "string") {
    if (!/^[0-7]+$/.test(value)) throw invalidArgValue(name, value, "must be a 32-bit unsigned integer or an octal string");
    value = Number.parseInt(value, 8);
  }
  validateUint32(value, name);
  return value;
}

export function validateThisInternalField(object: unknown, fieldKey: PropertyKey, className: string): asserts object is Record<PropertyKey, unknown> {
  if (typeof object !== "object" || object === null || !Object.prototype.hasOwnProperty.call(object, fieldKey)) {
    throw codedError(TypeError, `Value of "this" must be of type ${className}`, "ERR_INVALID_THIS");
  }
}
