import { createTracing } from "./tracing.ts";

// The globals that `env` and `exports` view, which `withEnv` and `withExports` replace.
type ViewedGlobal = "__tokamak_env" | "__tokamak_exports";

// A live view of the object currently stored at `globalThis[key]`.
function globalView(key: ViewedGlobal): Record<PropertyKey, unknown> {
  return new Proxy({}, {
    get(_target, property) { return globalThis[key]?.[property]; },
    has(_target, property) { return property in (globalThis[key] ?? {}); },
    ownKeys() { return Reflect.ownKeys(globalThis[key] ?? {}); },
    getOwnPropertyDescriptor(_target, property) {
      return Object.getOwnPropertyDescriptor(globalThis[key] ?? {}, property) ?? { enumerable: true, configurable: true };
    },
  });
}

export const env = globalView("__tokamak_env");
export function waitUntil(promise: unknown): void { globalThis.__tokamak_context.waitUntil(promise); }
export const exports = globalView("__tokamak_exports");
export const restore = Symbol("cloudflare:workers.restore");
export const tracing = createTracing();
export const cache = {};

export class RpcTarget {
  get [Symbol.toStringTag]() { return "JsRpcTarget"; }
}
export class RpcStub {
  declare readonly __value: object;
  constructor(value: unknown) {
    if (value === null || typeof value !== "object") throw new TypeError("RpcStub requires an object");
    Object.defineProperty(this, "__value", { configurable: false, enumerable: false, value });
  }
  dup() { return new RpcStub(this.__value); }
  get [Symbol.toStringTag]() { return "JsRpcStub"; }
}
// Tokamak makes no RPC calls, so the RPC result classes have no instances and
// their methods never have a valid `this`.
const illegalInvocation = "Illegal invocation: function called with incorrect `this` reference. See https://developers.cloudflare.com/workers/observability/errors/#illegal-invocation-errors for details.";
export function RpcPromise() { throw new TypeError("Illegal constructor"); }
export function RpcProperty() { throw new TypeError("Illegal constructor"); }
for (const [RpcResult, tag] of [[RpcPromise, "JsRpcPromise"], [RpcProperty, "JsRpcProperty"]] as const) {
  Object.assign(RpcResult.prototype, {
    then() { throw new TypeError(illegalInvocation); },
    catch() { throw new TypeError(illegalInvocation); },
    finally() { throw new TypeError(illegalInvocation); },
  });
  Object.defineProperty(RpcResult.prototype, Symbol.toStringTag, { value: tag, writable: true, configurable: true });
}
Object.defineProperty(RpcPromise.prototype, Symbol.dispose, {
  value() { throw new TypeError(illegalInvocation); },
  writable: true,
  configurable: true,
});
export class ServiceStub {
  constructor() { throw new TypeError("Illegal constructor"); }
  fetch() { throw new TypeError("Service bindings are not available in the Tokamak runtime"); }
  connect() { throw new TypeError("Service bindings are not available in the Tokamak runtime"); }
}
export class WorkerEntrypoint {
  declare ctx: object;
  declare env: unknown;
  constructor(ctx: unknown, workerEnv: unknown = env) {
    if (ctx === null || typeof ctx !== "object") throw new TypeError("WorkerEntrypoint requires an execution context");
    this.ctx = ctx;
    this.env = workerEnv;
  }
}
export class DurableObject {
  declare ctx: object;
  declare env: unknown;
  constructor(ctx: unknown, workerEnv: unknown = env) {
    if (ctx === null || typeof ctx !== "object") throw new TypeError("DurableObject requires state");
    this.ctx = ctx;
    this.env = workerEnv;
  }
}
export class WorkflowEntrypoint extends WorkerEntrypoint {}
export function abortIsolate() { throw new Error("The Worker isolate was aborted"); }
export function withEnv(newEnv: unknown, callback: () => unknown): unknown { return withValues({ __tokamak_env: newEnv }, callback); }
export function withExports(newExports: unknown, callback: () => unknown): unknown { return withValues({ __tokamak_exports: newExports }, callback); }
export function withEnvAndExports(newEnv: unknown, newExports: unknown, callback: () => unknown): unknown { return withValues({ __tokamak_env: newEnv, __tokamak_exports: newExports }, callback); }
function withValues(values: Partial<Record<ViewedGlobal, unknown>>, callback: () => unknown): unknown {
  const previous = Object.fromEntries(Object.keys(values).map(key => [key, Reflect.get(globalThis, key)]));
  Object.assign(globalThis, values);
  try { return callback(); }
  finally { Object.assign(globalThis, previous); }
}
