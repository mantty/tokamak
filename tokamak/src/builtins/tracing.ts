import { captureAsyncContext, runInAsyncContext } from "./async-context.ts";
import { parameterTypeError, string } from "../globals/conversions.ts";
import { isFunction, isObject } from "../globals/objects.ts";

// The async context entry holding the span active inside a span's callback.
const activeSpanKey = Symbol("activeSpan");
const STATUS_CODES = new Set(["unset", "ok", "error"]);

// An attribute value converted as the runtime converts a boolean, number or string.
function convertAttributeValue(value: unknown): string | number | boolean | undefined {
  if (typeof value === "bigint") throw new TypeError("Cannot convert a BigInt value to a number");
  if (value === undefined || typeof value === "boolean" || typeof value === "number") return value;
  return string(value);
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

  setAttribute(key: unknown, value: unknown): this {
    if (arguments.length === 0) throw parameterTypeError("setAttribute", "Span", 1, "string");
    string(key);
    convertAttributeValue(value);
    return this;
  }

  setAttributes(attributes: unknown): this {
    if (!isObject(attributes) || Array.isArray(attributes)) {
      throw parameterTypeError("setAttributes", "Span", 1, "object");
    }
    for (const key of Object.keys(attributes)) convertAttributeValue(attributes[key]);
    return this;
  }

  recordException(): void {
    if (arguments.length === 0) throw parameterTypeError("recordException", "Span", 1, "Value");
  }

  updateName(name: unknown): this {
    if (arguments.length === 0) throw parameterTypeError("updateName", "Span", 1, "string");
    string(name);
    return this;
  }

  setStatus(status: unknown): this {
    if (arguments.length > 0 && (status === undefined || status === null)) {
      throw new TypeError("Cannot initialize N7workerd3api12user_tracing17TracingSpanStatusE with required members from an undefined or null value.");
    }
    if (!isObject(status)) throw parameterTypeError("setStatus", "Span", 1, "TracingSpanStatus");
    const code = string(status.code);
    const message = status.message;
    if (message !== undefined) string(message);
    if (!STATUS_CODES.has(code)) throw new TypeError("Span status code must be 'unset', 'ok', or 'error'.");
    return this;
  }

  end(): void {}
}

// The span active outside every span's callback.
const rootSpan: Span = Object.create(Span.prototype);

// The result of `callback`, called with a new span active, as `method` calls it.
function runInNewSpan(method: string, argumentCount: number, name: unknown, callback: unknown, args: unknown[]): unknown {
  if (argumentCount === 0) throw parameterTypeError(method, "Tracing", 1, "string");
  string(name);
  if (!isFunction(callback)) throw parameterTypeError(method, "Tracing", 2, "Function");
  const span: Span = Object.create(Span.prototype);
  const context = new Map(captureAsyncContext());
  context.set(activeSpanKey, span);
  return runInAsyncContext(context, callback, globalThis, [span, ...args]);
}

class Tracing {
  declare Span: typeof Span;

  constructor() {
    throw new TypeError("Illegal constructor");
  }

  enterSpan(name: unknown, callback: unknown, ...args: unknown[]): unknown {
    const result = runInNewSpan("enterSpan", arguments.length, name, callback, args);
    if (!(result instanceof Promise)) return result;
    // A new promise, settling two jobs after `result`.
    return new Promise(resolve => resolve(Promise.prototype.then.call(result)));
  }

  startActiveSpan(name: unknown, callback: unknown, ...args: unknown[]): unknown {
    return runInNewSpan("startActiveSpan", arguments.length, name, callback, args);
  }

  startSpan(name: unknown): Span {
    if (arguments.length === 0) throw parameterTypeError("startSpan", "Tracing", 1, "string");
    string(name);
    return Object.create(Span.prototype);
  }

  getActiveSpan(): unknown {
    return captureAsyncContext()?.get(activeSpanKey) ?? rootSpan;
  }
}

Tracing.prototype.Span = Span;
Object.defineProperty(Span.prototype, Symbol.toStringTag, { value: "Span", configurable: true });
Object.defineProperty(Tracing.prototype, Symbol.toStringTag, { value: "Tracing", configurable: true });

export function createTracing(): Tracing {
  return Object.create(Tracing.prototype);
}
