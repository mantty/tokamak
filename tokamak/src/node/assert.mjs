import { isDeepStrictEqual, isPartialDeepStrictEqual } from "./comparisons.mjs";
import { inspect } from "./util.mjs";

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

function matches(thrown, expected) {
  if (expected === undefined) return true;
  if (typeof expected === "function") return thrown instanceof expected;
  if (expected instanceof RegExp) return expected.test(thrown?.message ?? String(thrown));
  if (expected && typeof expected === "object") return Object.keys(expected).every(key => isDeepStrictEqual(thrown?.[key], expected[key]));
  return false;
}

function callbackResult(callback, expected, messageValue, operator) {
  let thrown;
  try { callback(); } catch (error) { thrown = error; }
  if (operator === "throws") {
    if (thrown === undefined || !matches(thrown, expected)) assertionFailure(thrown, expected, operator, messageValue);
    return thrown;
  }
  if (thrown !== undefined) assertionFailure(thrown, undefined, operator, messageValue);
}

function matchesRegExp(value, regexp) {
  if (!(regexp instanceof RegExp)) throw new TypeError("The \"regexp\" argument must be an instance of RegExp");
  return regexp.test(String(value));
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
assert.deepEqual = (actual, expected, messageValue) => { if (!isDeepStrictEqual(actual, expected)) assertionFailure(actual, expected, "deepStrictEqual", messageValue); };
assert.notDeepEqual = (actual, expected, messageValue) => { if (isDeepStrictEqual(actual, expected)) assertionFailure(actual, expected, "notDeepStrictEqual", messageValue); };
assert.deepStrictEqual = (actual, expected, messageValue) => { if (!isDeepStrictEqual(actual, expected)) assertionFailure(actual, expected, "deepStrictEqual", messageValue); };
assert.notDeepStrictEqual = (actual, expected, messageValue) => { if (isDeepStrictEqual(actual, expected)) assertionFailure(actual, expected, "notDeepStrictEqual", messageValue); };
assert.ifError = value => { if (value != null) throw value; };
assert.match = (value, regexp, messageValue) => { if (!matchesRegExp(value, regexp)) assertionFailure(value, regexp, "match", messageValue); };
assert.doesNotMatch = (value, regexp, messageValue) => { if (matchesRegExp(value, regexp)) assertionFailure(value, regexp, "doesNotMatch", messageValue); };
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
  if (!isPartialDeepStrictEqual(actual, expected)) assertionFailure(actual, expected, "partialDeepStrictEqual", messageValue);
};

function strictAssert(value, messageValue) { assert(value, messageValue); }
assert.strict = Object.assign(strictAssert, assert, { equal: assert.strictEqual, notEqual: assert.notStrictEqual, deepEqual: assert.deepStrictEqual, notDeepEqual: assert.notDeepStrictEqual });
assert.strict.strict = assert.strict;

export const {
  strict, ok, equal, notEqual, strictEqual, notStrictEqual, deepEqual, notDeepEqual, deepStrictEqual, notDeepStrictEqual,
  ifError, match, doesNotMatch, throws, doesNotThrow, rejects, doesNotReject, partialDeepStrictEqual, fail,
} = assert;
export default assert;
