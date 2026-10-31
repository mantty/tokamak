import { captureAsyncContext, runInAsyncContext } from "../builtins/async-context.ts";
import { objectOf } from "../globals/objects.ts";

const localStorageState = new WeakMap<AsyncLocalStorage, { name: unknown; defaultValue: unknown }>();
const resourceState = new WeakMap<AsyncResource, { type: string; asyncId: number; triggerAsyncId: number; context: AsyncContextFrame | undefined }>();

function notImplemented(name: string): never { throw new Error(`asyncLocalStorage.${name}() is not implemented`); }

// A callback these APIs run with any receiver and arguments.
type Callback<This, Args extends unknown[], Result> = (this: This, ...args: Args) => Result;

function validateCallback<T>(callback: T): asserts callback is Extract<T, Callback<never, never[], unknown>> {
  if (typeof callback !== "function") throw new TypeError("The callback argument must be of type function");
}

// The state set for `object` by its constructor; a TypeError for a receiver it did not construct.
function stateOf<K extends object, V>(states: WeakMap<K, V>, object: K): V {
  const state = states.get(object);
  if (state === undefined) throw new TypeError("Illegal invocation");
  return state;
}

export class AsyncLocalStorage {
  constructor(options: unknown = {}) {
    const init = options == null ? undefined : objectOf(options);
    localStorageState.set(this, { name: init?.name, defaultValue: init?.defaultValue });
  }
  get name() { return stateOf(localStorageState, this).name; }
  getStore(): unknown {
    const context = captureAsyncContext();
    return context?.has(this) ? context.get(this) : stateOf(localStorageState, this).defaultValue;
  }
  static bind<Args extends unknown[], Result>(callback: Callback<undefined, Args, Result>): (...args: Args) => Result {
    validateCallback(callback);
    const context = captureAsyncContext();
    return (...args) => runInAsyncContext(context, callback, undefined, args);
  }
  static snapshot() {
    const context = captureAsyncContext();
    return <Args extends unknown[], Result>(callback: Callback<undefined, Args, Result>, ...args: Args): Result => {
      validateCallback(callback);
      return runInAsyncContext(context, callback, undefined, args);
    };
  }
  disable() { return notImplemented("disable"); }
  enterWith() { return notImplemented("enterWith"); }
  run<Args extends unknown[], Result>(value: unknown, callback: Callback<undefined, Args, Result>, ...args: Args): Result {
    validateCallback(callback);
    const context = new Map(captureAsyncContext());
    context.set(this, value);
    return runInAsyncContext(context, callback, undefined, args);
  }
  exit<Args extends unknown[], Result>(callback: Callback<undefined, Args, Result>, ...args: Args): Result {
    return this.run(undefined, callback, ...args);
  }
}

export class AsyncResource {
  constructor(type?: unknown) { resourceState.set(this, { type: String(type ?? ""), asyncId: 0, triggerAsyncId: 0, context: captureAsyncContext() }); }
  asyncId() { return stateOf(resourceState, this).asyncId; }
  triggerAsyncId() { return stateOf(resourceState, this).triggerAsyncId; }
  bind<Args extends unknown[], Result>(callback: Callback<unknown, Args, Result>, thisArg: unknown = globalThis): (...args: Args) => Result {
    validateCallback(callback);
    const resource = this;
    return function (...args) { return resource.runInAsyncScope(callback, thisArg, ...args); };
  }
  runInAsyncScope<This, Args extends unknown[], Result>(callback: Callback<This, Args, Result>, thisArg: This, ...args: Args): Result {
    validateCallback(callback);
    return runInAsyncContext(stateOf(resourceState, this).context, callback, thisArg, args);
  }
  emitDestroy() { return this; }
  static bind<Args extends unknown[], Result>(callback: Callback<unknown, Args, Result>, type?: unknown, thisArg: unknown = globalThis): (...args: Args) => Result {
    validateCallback(callback);
    return new AsyncResource(type).bind(callback, thisArg);
  }
}

export function createHook(callbacks: unknown = {}) {
  const hook = {
    enable() { hook.enabled = true; return hook; },
    disable() { hook.enabled = false; return hook; },
    enabled: false,
    callbacks: callbacks ?? {},
  };
  return hook;
}

export function executionAsyncId() { return 0; }
export const triggerAsyncId = executionAsyncId;
export function executionAsyncResource() { return {}; }

export const asyncWrapProviders = {
  NONE: 0, DIRHANDLE: 0, DNSCHANNEL: 0, ELASTIC_APM: 0, FILEHANDLE: 0, FSEVENTWRAP: 0, FSREQCALLBACK: 0,
  FSREQPROMISE: 0, GETADDRINFOREQWRAP: 0, GETNAMEINFOREQWRAP: 0, HTTP2SESSION: 0, HTTP2STREAM: 0,
  HTTPINCOMINGMESSAGE: 0, HTTPCLIENTREQUEST: 0, JSSTREAM: 0, MESSAGEPORT: 0, PIPECONNECTWRAP: 0,
  PIPESERVERWRAP: 0, PIPEWRAP: 0, PROCESSWRAP: 0, PROMISE: 0, QUERYWRAP: 0, QUEUEWORK: 0,
  SHUTDOWNWRAP: 0, SIGNALWRAP: 0, STATWATCHER: 0, STREAMPIPE: 0, TCPCONNECTWRAP: 0, TCPSERVERWRAP: 0,
  TCPWRAP: 0, TTYWRAP: 0, UDPSENDWRAP: 0, UDPWRAP: 0, WRITEWRAP: 0, ZLIB: 0,
};

export default { AsyncLocalStorage, AsyncResource, createHook, executionAsyncId, triggerAsyncId, executionAsyncResource, asyncWrapProviders };
