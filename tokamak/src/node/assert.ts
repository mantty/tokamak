import { isDeepStrictEqual, isPartialDeepStrictEqual } from "./comparisons.ts";
import { callFunction, objectOf, propertyOf } from "../globals/objects.ts";
import { inspect } from "./util.ts";

export class AssertionError extends Error {
  declare code: string;
  declare generatedMessage: boolean;
  declare actual: unknown;
  declare expected: unknown;
  declare operator: unknown;

  constructor(options: unknown = {}) {
    const message = propertyOf(options, "message"), actual = propertyOf(options, "actual");
    const expected = propertyOf(options, "expected"), operator = propertyOf(options, "operator");
    super(message === undefined ? assertionMessage(actual, expected, operator) : String(message));
    this.name = "AssertionError";
    this.code = "ERR_ASSERTION";
    this.generatedMessage = message === undefined;
    this.actual = actual;
    this.expected = expected;
    this.operator = operator;
  }
}

function assertionMessage(actual: unknown, expected: unknown, operator: unknown): string {
  if (operator === "fail") return "Failed";
  if (operator === "throws" || operator === "rejects") return `Expected ${operator === "throws" ? "an exception" : "a rejection"}`;
  if (operator === "doesNotThrow" || operator === "doesNotReject") return `Got unwanted ${operator === "doesNotThrow" ? "exception" : "rejection"}`;
  if (operator === "match" || operator === "doesNotMatch") return `The input was expected to ${operator === "match" ? "match" : "not match"} ${inspect(expected)}`;
  if (operator === "deepStrictEqual" || operator === "deepEqual" || operator === "notDeepStrictEqual" || operator === "notDeepEqual") {
    return `Expected values to ${operator.startsWith("not") ? "not be" : "be"} deeply equal:\n+ actual ${inspect(actual)}\n- expected ${inspect(expected)}`;
  }
  return `Expected values to ${operator === "notStrictEqual" || operator === "notEqual" ? "not be" : "be"} strictly equal:\n\n- ${inspect(actual)}\n+ ${inspect(expected)}`;
}

function message(value: unknown): string | undefined { return value === undefined ? undefined : String(value); }

function assertionFailure(actual: unknown, expected: unknown, operator: string, messageValue: unknown): never {
  throw new AssertionError({ actual, expected, operator, message: message(messageValue) });
}

function matches(thrown: unknown, expected: unknown): boolean {
  if (expected === undefined) return true;
  if (typeof expected === "function") return thrown instanceof expected;
  if (expected instanceof RegExp) return expected.test(String(propertyOf(thrown, "message") ?? thrown));
  if (expected && typeof expected === "object") {
    const fields = objectOf(expected);
    return Object.keys(fields).every(key => isDeepStrictEqual(propertyOf(thrown, key), fields[key]));
  }
  return false;
}

function callbackResult(callback: unknown, expected: unknown, messageValue: unknown, operator: "throws" | "doesNotThrow"): unknown {
  let thrown: unknown;
  try { callFunction(callback, undefined); } catch (error) { thrown = error; }
  if (operator === "throws") {
    if (thrown === undefined || !matches(thrown, expected)) assertionFailure(thrown, expected, operator, messageValue);
    return thrown;
  }
  if (thrown !== undefined) assertionFailure(thrown, undefined, operator, messageValue);
}

function matchesRegExp(value: unknown, regexp: unknown): boolean {
  if (!(regexp instanceof RegExp)) throw new TypeError("The \"regexp\" argument must be an instance of RegExp");
  return regexp.test(String(value));
}

// The settled value of `promise`, or of calling it when it is a function.
async function settle(promise: unknown): Promise<unknown> {
  return typeof promise === "function" ? callFunction(promise, undefined) : promise;
}

function assert(value: unknown, messageValue?: unknown): void {
  if (!value) assertionFailure(value, true, "==", messageValue);
}

assert.AssertionError = AssertionError;
assert.fail = (messageValue?: unknown): never => {
  if (messageValue == null) throw new AssertionError({ operator: "fail" });
  if (typeof messageValue === "string") throw new AssertionError({ operator: "fail", message: messageValue });
  throw messageValue;
};
assert.ok = assert;
assert.equal = (actual: unknown, expected: unknown, messageValue?: unknown): void => { if (!Object.is(actual, expected)) assertionFailure(actual, expected, "strictEqual", messageValue); };
assert.notEqual = (actual: unknown, expected: unknown, messageValue?: unknown): void => { if (Object.is(actual, expected)) assertionFailure(actual, expected, "notStrictEqual", messageValue); };
assert.strictEqual = (actual: unknown, expected: unknown, messageValue?: unknown): void => { if (!Object.is(actual, expected)) assertionFailure(actual, expected, "strictEqual", messageValue); };
assert.notStrictEqual = (actual: unknown, expected: unknown, messageValue?: unknown): void => { if (Object.is(actual, expected)) assertionFailure(actual, expected, "notStrictEqual", messageValue); };
assert.deepEqual = (actual: unknown, expected: unknown, messageValue?: unknown): void => { if (!isDeepStrictEqual(actual, expected)) assertionFailure(actual, expected, "deepStrictEqual", messageValue); };
assert.notDeepEqual = (actual: unknown, expected: unknown, messageValue?: unknown): void => { if (isDeepStrictEqual(actual, expected)) assertionFailure(actual, expected, "notDeepStrictEqual", messageValue); };
assert.deepStrictEqual = (actual: unknown, expected: unknown, messageValue?: unknown): void => { if (!isDeepStrictEqual(actual, expected)) assertionFailure(actual, expected, "deepStrictEqual", messageValue); };
assert.notDeepStrictEqual = (actual: unknown, expected: unknown, messageValue?: unknown): void => { if (isDeepStrictEqual(actual, expected)) assertionFailure(actual, expected, "notDeepStrictEqual", messageValue); };
assert.ifError = (value: unknown): void => { if (value != null) throw value; };
assert.match = (value: unknown, regexp: unknown, messageValue?: unknown): void => { if (!matchesRegExp(value, regexp)) assertionFailure(value, regexp, "match", messageValue); };
assert.doesNotMatch = (value: unknown, regexp: unknown, messageValue?: unknown): void => { if (matchesRegExp(value, regexp)) assertionFailure(value, regexp, "doesNotMatch", messageValue); };
assert.throws = (callback: unknown, expected?: unknown, messageValue?: unknown): unknown => callbackResult(callback, expected, messageValue, "throws");
assert.doesNotThrow = (callback: unknown, messageValue?: unknown): unknown => callbackResult(callback, undefined, messageValue, "doesNotThrow");
assert.rejects = async (promise: unknown, expected?: unknown, messageValue?: unknown): Promise<unknown> => {
  try { await settle(promise); }
  catch (error) {
    if (!matches(error, expected)) assertionFailure(error, expected, "rejects", messageValue);
    return error;
  }
  assertionFailure(undefined, expected, "rejects", messageValue);
};
assert.doesNotReject = async (promise: unknown, messageValue?: unknown): Promise<void> => {
  try { await settle(promise); }
  catch (error) { assertionFailure(error, undefined, "doesNotReject", messageValue); }
};
assert.partialDeepStrictEqual = (actual: unknown, expected: unknown, messageValue?: unknown): void => {
  if (!isPartialDeepStrictEqual(actual, expected)) assertionFailure(actual, expected, "partialDeepStrictEqual", messageValue);
};

function strictAssert(value: unknown, messageValue?: unknown): void { assert(value, messageValue); }
assert.strict = Object.assign(strictAssert, assert, { equal: assert.strictEqual, notEqual: assert.notStrictEqual, deepEqual: assert.deepStrictEqual, notDeepEqual: assert.notDeepStrictEqual });
assert.strict.strict = assert.strict;

export const {
  strict, ok, equal, notEqual, strictEqual, notStrictEqual, deepEqual, notDeepEqual, deepStrictEqual, notDeepStrictEqual,
  ifError, match, doesNotMatch, throws, doesNotThrow, rejects, doesNotReject, partialDeepStrictEqual, fail,
} = assert;
export default assert;
