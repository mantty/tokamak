export class AssertionError extends Error {
  constructor(options = {}) {
    const hasMessage = options.message !== undefined;
    super(hasMessage ? String(options.message) : assertionMessage(options));
    this.name = "AssertionError";
    this.code = "ERR_ASSERTION";
    this.generatedMessage = !hasMessage;
    this.actual = options.actual;
    this.expected = options.expected;
    this.operator = options.operator;
  }
}

function inspect(value, seen = []) {
  if (typeof value === "string") return `'${value.replaceAll("'", "\\'")}'`;
  if (value === undefined) return "undefined";
  if (value === null) return "null";
  if (typeof value === "bigint") return `${value}n`;
  if (typeof value !== "object") return String(value);
  if (seen.includes(value)) return "[Circular]";
  if (value instanceof Date) return Number.isNaN(value.getTime()) ? "Invalid Date" : value.toISOString();
  if (value instanceof RegExp) return String(value);
  if (Array.isArray(value)) return `[ ${value.map(item => inspect(item, [...seen, value])).join(", ")} ]`;
  const entries = Object.keys(value).map(key => `${key}: ${inspect(value[key], [...seen, value])}`);
  return `{ ${entries.join(", ")} }`;
}

function assertionMessage({ actual, expected, operator }) {
  if (operator === "fail") return "Failed";
  if (operator === "throws" || operator === "rejects") return `Expected ${operator === "throws" ? "an exception" : "a rejection"}`;
  if (operator === "doesNotThrow" || operator === "doesNotReject") return `Got unwanted ${operator === "doesNotThrow" ? "exception" : "rejection"}`;
  if (operator === "match" || operator === "doesNotMatch") return `The input was expected to ${operator === "match" ? "match" : "not match"} ${inspect(expected)}`;
  if (operator === "deepStrictEqual" || operator === "deepEqual" || operator === "notDeepStrictEqual" || operator === "notDeepEqual") {
    return `Expected values to ${operator.startsWith("not") ? "not be" : "be"} deeply equal:\n+ actual ${inspect(actual)}\n- expected ${inspect(expected)}`;
  }
  return `Expected values to ${operator === "notStrictEqual" || operator === "notEqual" ? "not be" : "be"} strictly equal:\n\n- ${inspect(actual)}\n+ ${inspect(expected)}`;
}

function message(value) { return value === undefined ? undefined : String(value); }

function assertionFailure(actual, expected, operator, messageValue) {
  throw new AssertionError({ actual, expected, operator, message: message(messageValue) });
}

function same(left, right, seen = []) {
  if (Object.is(left, right)) return true;
  if (left === null || right === null || typeof left !== "object" || typeof right !== "object") return false;
  const previous = seen.find(pair => pair[0] === left && pair[1] === right);
  if (previous) return true;
  seen.push([left, right]);
  if (left.constructor !== right.constructor) return false;
  if (left instanceof Date) return left.getTime() === right.getTime();
  if (left instanceof RegExp) return left.source === right.source && left.flags === right.flags && left.lastIndex === right.lastIndex;
  if (left instanceof Error) return left.name === right.name && left.message === right.message && same(left.cause, right.cause, seen);
  if (left instanceof ArrayBuffer || (typeof SharedArrayBuffer !== "undefined" && left instanceof SharedArrayBuffer)) {
    return left.byteLength === right.byteLength && same(new Uint8Array(left), new Uint8Array(right), seen);
  }
  if (ArrayBuffer.isView(left)) {
    if (left.byteLength !== right.byteLength) return false;
    return same(new Uint8Array(left.buffer, left.byteOffset, left.byteLength), new Uint8Array(right.buffer, right.byteOffset, right.byteLength), seen);
  }
  if (left instanceof Map) return sameMap(left, right, seen);
  if (left instanceof Set) return sameSet(left, right, seen);
  const leftKeys = Reflect.ownKeys(left).filter(key => Object.prototype.propertyIsEnumerable.call(left, key));
  const rightKeys = Reflect.ownKeys(right).filter(key => Object.prototype.propertyIsEnumerable.call(right, key));
  return leftKeys.length === rightKeys.length && leftKeys.every(key => rightKeys.includes(key) && same(left[key], right[key], seen));
}

function sameMap(left, right, seen) {
  if (left.size !== right.size) return false;
  const unmatched = [...right];
  for (const [key, value] of left) {
    const index = unmatched.findIndex(([otherKey, otherValue]) => same(key, otherKey, seen) && same(value, otherValue, seen));
    if (index < 0) return false;
    unmatched.splice(index, 1);
  }
  return true;
}

function sameSet(left, right, seen) {
  if (left.size !== right.size) return false;
  const unmatched = [...right];
  for (const value of left) {
    const index = unmatched.findIndex(other => same(value, other, seen));
    if (index < 0) return false;
    unmatched.splice(index, 1);
  }
  return true;
}

function matches(thrown, expected) {
  if (expected === undefined) return true;
  if (typeof expected === "function") return thrown instanceof expected;
  if (expected instanceof RegExp) return expected.test(thrown?.message ?? String(thrown));
  if (expected && typeof expected === "object") return Object.keys(expected).every(key => same(thrown?.[key], expected[key]));
  return false;
}

function callbackResult(callback, expected, messageValue, operator) {
  let thrown;
  try { callback(); } catch (error) { thrown = error; }
  if (operator === "throws" || operator === "rejects") {
    if (thrown === undefined || !matches(thrown, expected)) assertionFailure(thrown, expected, operator, messageValue);
    return thrown;
  }
  if (thrown !== undefined) assertionFailure(thrown, undefined, operator, messageValue);
  return undefined;
}

function assert(value, messageValue) {
  if (!value) assertionFailure(value, true, "==", messageValue);
}

assert.AssertionError = AssertionError;
assert.fail = messageValue => {
  if (messageValue == null) throw new AssertionError({ operator: "fail" });
  if (typeof messageValue === "string") throw new AssertionError({ operator: "fail", message: messageValue });
  throw messageValue;
};
assert.ok = assert;
assert.equal = (actual, expected, messageValue) => { if (!Object.is(actual, expected)) assertionFailure(actual, expected, "strictEqual", messageValue); };
assert.notEqual = (actual, expected, messageValue) => { if (Object.is(actual, expected)) assertionFailure(actual, expected, "notStrictEqual", messageValue); };
assert.strictEqual = (actual, expected, messageValue) => { if (!Object.is(actual, expected)) assertionFailure(actual, expected, "strictEqual", messageValue); };
assert.notStrictEqual = (actual, expected, messageValue) => { if (Object.is(actual, expected)) assertionFailure(actual, expected, "notStrictEqual", messageValue); };
assert.deepEqual = (actual, expected, messageValue) => { if (!same(actual, expected)) assertionFailure(actual, expected, "deepStrictEqual", messageValue); };
assert.notDeepEqual = (actual, expected, messageValue) => { if (same(actual, expected)) assertionFailure(actual, expected, "notDeepStrictEqual", messageValue); };
assert.deepStrictEqual = (actual, expected, messageValue) => { if (!same(actual, expected)) assertionFailure(actual, expected, "deepStrictEqual", messageValue); };
assert.notDeepStrictEqual = (actual, expected, messageValue) => { if (same(actual, expected)) assertionFailure(actual, expected, "notDeepStrictEqual", messageValue); };
assert.ifError = value => { if (value != null) throw value; };
assert.match = (value, regexp, messageValue) => { if (!(regexp instanceof RegExp)) throw new TypeError("The \"regexp\" argument must be an instance of RegExp"); if (!regexp.test(String(value))) assertionFailure(value, regexp, "match", messageValue); };
assert.doesNotMatch = (value, regexp, messageValue) => { if (!(regexp instanceof RegExp)) throw new TypeError("The \"regexp\" argument must be an instance of RegExp"); if (regexp.test(String(value))) assertionFailure(value, regexp, "doesNotMatch", messageValue); };
assert.throws = (callback, expected, messageValue) => callbackResult(callback, expected, messageValue, "throws");
assert.doesNotThrow = (callback, messageValue) => callbackResult(callback, undefined, messageValue, "doesNotThrow");
assert.rejects = async (promise, expected, messageValue) => {
  try { await (typeof promise === "function" ? promise() : promise); }
  catch (error) {
    if (!matches(error, expected)) assertionFailure(error, expected, "rejects", messageValue);
    return error;
  }
  assertionFailure(undefined, expected, "rejects", messageValue);
};
assert.doesNotReject = async (promise, messageValue) => {
  try { await (typeof promise === "function" ? promise() : promise); }
  catch (error) { assertionFailure(error, undefined, "doesNotReject", messageValue); }
};
assert.partialDeepStrictEqual = (actual, expected, messageValue) => {
  if (actual === null || actual === undefined || typeof actual !== "object") assertionFailure(actual, expected, "partialDeepStrictEqual", messageValue);
  for (const key of Reflect.ownKeys(expected)) if (!same(actual[key], expected[key])) assertionFailure(actual, expected, "partialDeepStrictEqual", messageValue);
};

function strictAssert(value, messageValue) { assert(value, messageValue); }
assert.strict = Object.assign(strictAssert, assert, { equal: assert.strictEqual, notEqual: assert.notStrictEqual, deepEqual: assert.deepStrictEqual, notDeepEqual: assert.notDeepStrictEqual });
assert.strict.strict = assert.strict;

export const strict = assert.strict;
export const ok = assert.ok;
export const equal = assert.equal;
export const notEqual = assert.notEqual;
export const strictEqual = assert.strictEqual;
export const notStrictEqual = assert.notStrictEqual;
export const deepEqual = assert.deepEqual;
export const notDeepEqual = assert.notDeepEqual;
export const deepStrictEqual = assert.deepStrictEqual;
export const notDeepStrictEqual = assert.notDeepStrictEqual;
export const ifError = assert.ifError;
export const match = assert.match;
export const doesNotMatch = assert.doesNotMatch;
export const throws = assert.throws;
export const doesNotThrow = assert.doesNotThrow;
export const rejects = assert.rejects;
export const doesNotReject = assert.doesNotReject;
export const partialDeepStrictEqual = assert.partialDeepStrictEqual;
export const fail = assert.fail;
export default assert;
