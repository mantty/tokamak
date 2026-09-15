const localStorageState = new WeakMap();
const resourceState = new WeakMap();

function notImplemented(name) { throw new Error(`asyncLocalStorage.${name}() is not implemented`); }

export class AsyncLocalStorage {
  constructor(options = {}) { localStorageState.set(this, { store: undefined, name: options?.name }); }
  get name() { return localStorageState.get(this).name; }
  getStore() { return localStorageState.get(this).store; }
  disable() { return notImplemented("disable"); }
  enterWith() { return notImplemented("enterWith"); }
  run(value, callback, ...args) {
    if (typeof callback !== "function") throw new TypeError("The callback argument must be of type function");
    const state = localStorageState.get(this);
    const previous = state.store;
    state.store = value;
    try { return callback(...args); }
    finally { state.store = previous; }
  }
  exit(callback, ...args) {
    if (typeof callback !== "function") throw new TypeError("The callback argument must be of type function");
    const state = localStorageState.get(this);
    const previous = state.store;
    state.store = undefined;
    try { return callback(...args); }
    finally { state.store = previous; }
  }
}

export class AsyncResource {
  constructor(type) { resourceState.set(this, { type: String(type ?? ""), asyncId: 0, triggerAsyncId: 0 }); }
  asyncId() { return resourceState.get(this).asyncId; }
  triggerAsyncId() { return resourceState.get(this).triggerAsyncId; }
  bind(callback, thisArg) { return AsyncResource.bind(callback, thisArg ?? this); }
  runInAsyncScope(callback, thisArg, ...args) {
    if (typeof callback !== "function") throw new TypeError("The callback argument must be of type function");
    return callback.apply(thisArg, args);
  }
  emitDestroy() { return this; }
  static bind(callback, thisArg) {
    if (typeof callback !== "function") throw new TypeError("The callback argument must be of type function");
    return function (...args) { return callback.apply(thisArg ?? this, args); };
  }
}

export function createHook(callbacks = {}) {
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
