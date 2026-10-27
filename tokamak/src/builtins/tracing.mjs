import { captureAsyncContext, runInAsyncContext } from "./async-context.mjs";

// The async context entry holding the span active inside a span's callback.
const activeSpanKey = Symbol("activeSpan");
const STATUS_CODES = new Set(["unset", "ok", "error"]);

function parameterError(method, owner, index, type) {
  return new TypeError(`Failed to execute '${method}' on '${owner}': parameter ${index} is not of type '${type}'.`);
}

// `value` converted as the runtime converts a string argument.
function convertToString(value) {
  if (typeof value === "symbol") throw new TypeError("Cannot convert a Symbol value to a string");
  return String(value);
}

// An attribute value converted as the runtime converts a boolean, number or string.
function convertAttributeValue(value) {
  if (typeof value === "bigint") throw new TypeError("Cannot convert a BigInt value to a number");
  if (value === undefined || typeof value === "boolean" || typeof value === "number") return value;
  return convertToString(value);
}

// Spans are not traced: they record nothing and report an all-zero context.
class Span {
  constructor() {
    throw new TypeError("Illegal constructor");
  }

  get isTraced() { return false; }

  spanContext() {
    return { traceId: "0".repeat(32), spanId: "0".repeat(16), traceFlags: 0 };
  }

  setAttribute(key, value) {
    if (arguments.length === 0) throw parameterError("setAttribute", "Span", 1, "string");
    convertToString(key);
    convertAttributeValue(value);
    return this;
  }

  setAttributes(attributes) {
    if (attributes === null || (typeof attributes !== "object" && typeof attributes !== "function") || Array.isArray(attributes)) {
      throw parameterError("setAttributes", "Span", 1, "object");
    }
    for (const key of Object.keys(attributes)) convertAttributeValue(attributes[key]);
    return this;
  }

  recordException() {
    if (arguments.length === 0) throw parameterError("recordException", "Span", 1, "Value");
  }

  updateName(name) {
    if (arguments.length === 0) throw parameterError("updateName", "Span", 1, "string");
    convertToString(name);
    return this;
  }

  setStatus(status) {
    if (arguments.length > 0 && (status === undefined || status === null)) {
      throw new TypeError("Cannot initialize N7workerd3api12user_tracing17TracingSpanStatusE with required members from an undefined or null value.");
    }
    if (typeof status !== "object" && typeof status !== "function") throw parameterError("setStatus", "Span", 1, "TracingSpanStatus");
    const code = convertToString(status.code);
    const message = status.message;
    if (message !== undefined) convertToString(message);
    if (!STATUS_CODES.has(code)) throw new TypeError("Span status code must be 'unset', 'ok', or 'error'.");
    return this;
  }

  end() {}
}

// The span active outside every span's callback.
const rootSpan = Object.create(Span.prototype);

// The result of `callback`, called with a new span active, as `method` calls it.
function runInNewSpan(method, argumentCount, name, callback, args) {
  if (argumentCount === 0) throw parameterError(method, "Tracing", 1, "string");
  convertToString(name);
  if (typeof callback !== "function") throw parameterError(method, "Tracing", 2, "Function");
  const span = Object.create(Span.prototype);
  const context = new Map(captureAsyncContext());
  context.set(activeSpanKey, span);
  return runInAsyncContext(context, callback, globalThis, [span, ...args]);
}

class Tracing {
  constructor() {
    throw new TypeError("Illegal constructor");
  }

  enterSpan(name, callback, ...args) {
    const result = runInNewSpan("enterSpan", arguments.length, name, callback, args);
    if (!(result instanceof Promise)) return result;
    // A new promise, settling two jobs after `result`.
    return new Promise(resolve => resolve(Promise.prototype.then.call(result)));
  }

  startActiveSpan(name, callback, ...args) {
    return runInNewSpan("startActiveSpan", arguments.length, name, callback, args);
  }

  startSpan(name) {
    if (arguments.length === 0) throw parameterError("startSpan", "Tracing", 1, "string");
    convertToString(name);
    return Object.create(Span.prototype);
  }

  getActiveSpan() {
    return captureAsyncContext()?.get(activeSpanKey) ?? rootSpan;
  }
}

Tracing.prototype.Span = Span;
Object.defineProperty(Span.prototype, Symbol.toStringTag, { value: "Span", configurable: true });
Object.defineProperty(Tracing.prototype, Symbol.toStringTag, { value: "Tracing", configurable: true });

export function createTracing() {
  return Object.create(Tracing.prototype);
}
