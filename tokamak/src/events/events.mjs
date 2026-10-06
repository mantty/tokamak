function internal(object, name, value) {
  Object.defineProperty(object, name, { configurable: true, enumerable: false, writable: true, value });
}

export function EventEmitter(options) {
  if (!Object.hasOwn(this, "__events")) internal(this, "__events", new Map());
  if (!Object.hasOwn(this, "__maxListeners")) internal(this, "__maxListeners", undefined);
  internal(this, "__captureRejections", Boolean(options?.captureRejections ?? EventEmitter.captureRejections));
}

EventEmitter.defaultMaxListeners = 10;
EventEmitter.errorMonitor = Symbol("events.errorMonitor");
EventEmitter.EventEmitter = EventEmitter;

Object.assign(EventEmitter.prototype, {
  on(name, listener) { return this.__add(name, listener, false, false); },
  addListener(name, listener) { return this.on(name, listener); },
  once(name, listener) { return this.__add(name, listener, true, false); },
  prependListener(name, listener) { return this.__add(name, listener, false, true); },
  prependOnceListener(name, listener) { return this.__add(name, listener, true, true); },
  off(name, listener) { return this.removeListener(name, listener); },

  removeListener(name, listener) {
    const list = this.__events?.get(name) ?? [];
    for (let index = 0; index < list.length; index += 1) {
      const entry = list[index];
      if (entry.listener !== listener && entry.wrapper !== listener) continue;
      this.__removeEntry(name, entry);
      break;
    }
    return this;
  },

  removeEventListener(name, listener) { return this.removeListener(name, listener); },
  addEventListener(name, listener) { return this.on(name, listener); },
  removeAllListeners(name) {
    if (name === undefined) {
      for (const key of [...this.__events.keys()].filter(value => value !== "removeListener")) this.removeAllListeners(key);
      return this.removeAllListeners("removeListener");
    }
    for (const entry of [...(this.__events.get(name) ?? [])]) this.__removeEntry(name, entry);
    return this;
  },

  emit(name, ...args) {
    const list = [...(this.__events?.get(name) ?? [])];
    if (name === "error") {
      for (const entry of [...(this.__events?.get(EventEmitter.errorMonitor) ?? [])]) {
        if (entry.once) this.__removeEntry(EventEmitter.errorMonitor, entry);
        entry.listener.apply(this, args);
      }
      if (list.length === 0) {
        throw args[0] instanceof Error ? args[0] : new Error(String(args[0] ?? "Unhandled error"));
      }
    }
    for (const entry of list) {
      if (entry.once) this.__removeEntry(name, entry);
      const value = entry.listener.apply(this, args);
      if (this.__captureRejections && value != null && typeof value.then === "function") {
        value.then(undefined, error => queueMicrotask(() => this.__rejected(error, name, args)));
      }
    }
    return list.length > 0;
  },

  __rejected(error, name, args) {
    const handler = this[EventEmitter.captureRejectionSymbol];
    if (typeof handler === "function") handler.call(this, error, name, ...args);
    else this.emit("error", error);
  },

  listeners(name) { return [...(this.__events?.get(name) ?? [])].map(entry => entry.listener); },
  rawListeners(name) { return [...(this.__events?.get(name) ?? [])].map(entry => entry.wrapper ?? entry.listener); },
  listenerCount(name, listener) {
    const list = this.__events?.get(name) ?? [];
    return listener === undefined ? list.length : list.filter(entry => entry.listener === listener || entry.wrapper === listener).length;
  },
  eventNames() { return [...(this.__events?.keys() ?? [])]; },
  setMaxListeners(value) {
    const max = Number(value);
    if (Number.isNaN(max) || max < 0 || (max !== Infinity && !Number.isInteger(max))) {
      throw new RangeError("The maximum number of listeners must be a non-negative number");
    }
    this.__maxListeners = max;
    return this;
  },
  getMaxListeners() { return this.__maxListeners ?? EventEmitter.defaultMaxListeners; },

  __add(name, listener, once, prepend) {
    if (typeof listener !== "function") throw new TypeError("listener must be a function");
    if (!Object.hasOwn(this, "__events")) internal(this, "__events", new Map());
    if (this.__events.has("newListener")) this.emit("newListener", name, listener);
    const entry = { listener, once, wrapper: once ? (...args) => listener.apply(this, args) : listener };
    entry.wrapper.listener = listener;
    const list = this.__events.get(name) ?? [];
    if (prepend) list.unshift(entry);
    else list.push(entry);
    this.__events.set(name, list);
    return this;
  },

  __removeEntry(name, entry) {
    const list = this.__events.get(name);
    const index = list?.indexOf(entry) ?? -1;
    if (index < 0) return;
    list.splice(index, 1);
    if (list.length === 0) this.__events.delete(name);
    if (this.__events.has("removeListener")) this.emit("removeListener", name, entry.listener);
  },
});

export class EventEmitterAsyncResource extends EventEmitter {
  constructor(options = {}) {
    super();
    this.asyncId = () => 0;
    this.triggerAsyncId = () => 0;
    this.asyncResource = this;
    this.type = options?.name ?? options?.type ?? "EventEmitterAsyncResource";
  }

  emitDestroy() { return this; }
}

function invalidSignal(signal) {
  return signal !== undefined && signal !== null && typeof signal.addEventListener !== "function";
}

export function once(emitter, name, options = {}) {
  if (options === null || (typeof options !== "object" && typeof options !== "function")) {
    return Promise.reject(new TypeError("options must be an object"));
  }
  const signal = options.signal;
  if (typeof emitter?.once !== "function" && typeof emitter?.addEventListener === "function") {
    return new Promise((resolve, reject) => {
      if (invalidSignal(signal)) return reject(new TypeError("options.signal must be an AbortSignal"));
      if (signal?.aborted) return reject(signal.reason);
      const listener = event => { signal?.removeEventListener?.("abort", onAbort); resolve([event]); };
      const onAbort = () => { emitter.removeEventListener(name, listener); reject(signal.reason); };
      emitter.addEventListener(name, listener, { once: true });
      signal?.addEventListener("abort", onAbort, { once: true });
    });
  }
  return new Promise((resolve, reject) => {
    let settled = false;
    const cleanup = () => {
      emitter.removeListener(name, listener);
      if (name !== "error") emitter.removeListener("error", onError);
      signal?.removeEventListener?.("abort", onAbort);
    };
    const listener = (...args) => {
      if (settled) return;
      settled = true;
      cleanup();
      resolve(args);
    };
    const onError = error => {
      if (settled) return;
      settled = true;
      cleanup();
      reject(error);
    };
    const onAbort = () => onError(signal.reason);
    if (invalidSignal(signal)) return reject(new TypeError("options.signal must be an AbortSignal"));
    if (signal?.aborted) return onAbort();
    emitter.once(name, listener);
    if (name !== "error") emitter.once("error", onError);
    signal?.addEventListener("abort", onAbort, { once: true });
  });
}

export function on(emitter, name, options = {}) {
  if (options === null || (typeof options !== "object" && typeof options !== "function")) {
    throw new TypeError("options must be an object");
  }
  const signal = options.signal;
  let failure;
  let failed = false;
  let wake;
  const queue = [];
  const listener = (...args) => {
    queue.push(args);
    wake?.();
    wake = undefined;
  };
  const onError = error => {
    failed = true;
    failure = error;
    wake?.();
    wake = undefined;
  };
  const onAbort = () => onError(signal.reason);
  const cleanup = () => {
    emitter.removeListener(name, listener);
    if (name !== "error") emitter.removeListener("error", onError);
    signal?.removeEventListener?.("abort", onAbort);
  };
  if (invalidSignal(signal)) throw new TypeError("options.signal must be an AbortSignal");
  if (signal?.aborted) onAbort();
  else {
    emitter.on(name, listener);
    if (name !== "error") emitter.on("error", onError);
    signal?.addEventListener("abort", onAbort, { once: true });
  }
  return (async function* () {
    try {
      while (true) {
        if (queue.length > 0) {
          yield queue.shift();
          continue;
        }
        if (failed) throw failure;
        await new Promise(resolve => { wake = resolve; });
      }
    } finally {
      cleanup();
    }
  })();
}

EventEmitter.once = once;
EventEmitter.on = on;
EventEmitter.EventEmitterAsyncResource = EventEmitterAsyncResource;
EventEmitter.addAbortListener = (signal, listener) => {
  if (signal === null || typeof signal?.addEventListener !== "function") throw new TypeError("signal must be an AbortSignal");
  signal.addEventListener("abort", listener, { once: true });
  return { [Symbol.dispose]() { signal.removeEventListener("abort", listener); } };
};
EventEmitter.captureRejectionSymbol = Symbol.for("nodejs.rejection");
EventEmitter.captureRejections = false;
EventEmitter.getEventListeners = (emitter, name) => emitter?.listeners?.(name) ?? [];
EventEmitter.getMaxListeners = emitter => emitter?.getMaxListeners?.() ?? EventEmitter.defaultMaxListeners;
EventEmitter.init = EventEmitter;
Object.defineProperty(EventEmitter, "listenerCount", {
  configurable: true,
  enumerable: true,
  value: (emitter, name, listener) => emitter.listenerCount(name, listener),
  writable: true,
});
EventEmitter.setMaxListeners = (value, ...emitters) => {
  for (const emitter of emitters) emitter.setMaxListeners(value);
  return emitters[0];
};
EventEmitter.usingDomains = false;
export const defaultMaxListeners = EventEmitter.defaultMaxListeners;
export const errorMonitor = EventEmitter.errorMonitor;
export const listenerCount = EventEmitter.listenerCount;
export const addAbortListener = EventEmitter.addAbortListener;
export const captureRejectionSymbol = EventEmitter.captureRejectionSymbol;
export const captureRejections = EventEmitter.captureRejections;
export const getEventListeners = EventEmitter.getEventListeners;
export const getMaxListeners = EventEmitter.getMaxListeners;
export const setMaxListeners = EventEmitter.setMaxListeners;
export const usingDomains = EventEmitter.usingDomains;
export default EventEmitter;
