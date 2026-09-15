import { createTracing } from "./tracing.mjs";

export const env = new Proxy({}, {
  get(_target, property) { return globalThis.__tokamak_env?.[property]; },
  has(_target, property) { return property in (globalThis.__tokamak_env ?? {}); },
  ownKeys() { return Reflect.ownKeys(globalThis.__tokamak_env ?? {}); },
  getOwnPropertyDescriptor(_target, property) {
    return Object.getOwnPropertyDescriptor(globalThis.__tokamak_env ?? {}, property) ?? { enumerable: true, configurable: true };
  },
});
export function waitUntil(promise) { globalThis.__tokamak_context.waitUntil(promise); }
export const exports = new Proxy({}, {
  get(_target, property) { return globalThis.__tokamak_exports?.[property]; },
  has(_target, property) { return property in (globalThis.__tokamak_exports ?? {}); },
  ownKeys() { return Reflect.ownKeys(globalThis.__tokamak_exports ?? {}); },
  getOwnPropertyDescriptor(_target, property) {
    return Object.getOwnPropertyDescriptor(globalThis.__tokamak_exports ?? {}, property) ?? { enumerable: true, configurable: true };
  },
});
export const restore = Symbol("cloudflare:workers.restore");
export const tracing = createTracing();
export const cache = {};

export class RpcTarget {
  get [Symbol.toStringTag]() { return "JsRpcTarget"; }
}
export class RpcStub {
  constructor(value) {
    if (value === null || typeof value !== "object") throw new TypeError("RpcStub requires an object");
    Object.defineProperty(this, "__value", { configurable: false, enumerable: false, value });
  }
  dup() { return new RpcStub(this.__value); }
  get [Symbol.toStringTag]() { return "JsRpcStub"; }
}
export class RpcPromise extends Promise {
  constructor() { throw new TypeError("Illegal constructor"); }
  then(...args) { return Promise.prototype.then.apply(this, args); }
  catch(...args) { return Promise.prototype.catch.apply(this, args); }
  finally(...args) { return Promise.prototype.finally.apply(this, args); }
}
export class RpcProperty extends Promise {
  constructor() { throw new TypeError("Illegal constructor"); }
  then(...args) { return Promise.prototype.then.apply(this, args); }
  catch(...args) { return Promise.prototype.catch.apply(this, args); }
  finally(...args) { return Promise.prototype.finally.apply(this, args); }
}
export class ServiceStub {
  constructor() { throw new TypeError("Illegal constructor"); }
  fetch() { throw new TypeError("Service bindings are not available in the Tokamak runtime"); }
  connect() { throw new TypeError("Service bindings are not available in the Tokamak runtime"); }
}
export class WorkerEntrypoint {
  constructor(ctx, workerEnv = env) {
    if (ctx === null || typeof ctx !== "object") throw new TypeError("WorkerEntrypoint requires an execution context");
    this.ctx = ctx;
    this.env = workerEnv;
  }
}
export class DurableObject {
  constructor(ctx, workerEnv = env) {
    if (ctx === null || typeof ctx !== "object") throw new TypeError("DurableObject requires state");
    this.ctx = ctx;
    this.env = workerEnv;
  }
}
export class WorkflowEntrypoint extends WorkerEntrypoint {}
export function abortIsolate() { throw new Error("The Worker isolate was aborted"); }
export function withEnv(newEnv, callback) { return withValues({ __tokamak_env: newEnv }, callback); }
export function withExports(newExports, callback) { return withValues({ __tokamak_exports: newExports }, callback); }
export function withEnvAndExports(newEnv, newExports, callback) { return withValues({ __tokamak_env: newEnv, __tokamak_exports: newExports }, callback); }
function withValues(values, callback) {
  const previous = Object.fromEntries(Object.keys(values).map(key => [key, globalThis[key]]));
  Object.assign(globalThis, values);
  try { return callback(); }
  finally { Object.assign(globalThis, previous); }
}
