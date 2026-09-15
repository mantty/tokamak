function domString(value) {
  if (typeof value === "symbol") throw new TypeError("Cannot convert a Symbol value to a string");
  return String(value);
}

function internal(object, name, value) {
  Object.defineProperty(object, name, {
    configurable: true,
    enumerable: false,
    writable: true,
    value,
  });
}

const listenerStore = Symbol("event-listeners");

function normalizeOptions(options) {
  if (typeof options === "boolean") throw new TypeError("Event listener options must be an object");
  options ??= {};
  return {
    capture: Boolean(options.capture),
    once: Boolean(options.once),
    passive: Boolean(options.passive),
    signal: options.signal ?? null,
  };
}

function captureOption(options) {
  if (typeof options === "boolean") throw new TypeError("Event listener options must be an object");
  return Boolean(options?.capture);
}

export class Event {
  constructor(type, options = {}) {
    options ??= {};
    internal(this, "__type", domString(type));
    internal(this, "__bubbles", Boolean(options.bubbles));
    internal(this, "__cancelable", Boolean(options.cancelable));
    internal(this, "__composed", Boolean(options.composed));
    internal(this, "__defaultPrevented", false);
    internal(this, "__eventPhase", 0);
    internal(this, "__isTrusted", false);
    internal(this, "__timeStamp", Date.now());
    internal(this, "__target", undefined);
    internal(this, "__currentTarget", undefined);
    internal(this, "__dispatching", false);
    internal(this, "__stopped", false);
    internal(this, "__immediateStopped", false);
    internal(this, "__inPassive", false);
  }

  get type() { return this.__type; }
  get target() { return this.__target; }
  get currentTarget() { return this.__currentTarget; }
  get srcElement() { return this.__target; }
  get eventPhase() { return this.__eventPhase; }
  get bubbles() { return this.__bubbles; }
  get cancelable() { return this.__cancelable; }
  get defaultPrevented() { return this.__defaultPrevented; }
  get composed() { return this.__composed; }
  get isTrusted() { return this.__isTrusted; }
  get timeStamp() { return this.__timeStamp; }
  get cancelBubble() { return this.__stopped; }
  set cancelBubble(value) { if (value) this.stopPropagation(); }
  get returnValue() { return !this.__defaultPrevented; }

  preventDefault() {
    if (this.__inPassive) throw new TypeError("Unable to preventDefault inside passive event listener invocation.");
    if (this.__cancelable) this.__defaultPrevented = true;
  }
  stopPropagation() { this.__stopped = true; }
  stopImmediatePropagation() { this.__stopped = true; this.__immediateStopped = true; }
  composedPath() { return this.__target === undefined ? [] : [this.__target]; }
}

for (const [name, value] of Object.entries({ NONE: 0, CAPTURING_PHASE: 1, AT_TARGET: 2, BUBBLING_PHASE: 3 })) {
  Object.defineProperty(Event, name, { configurable: true, enumerable: true, value, writable: false });
  Object.defineProperty(Event.prototype, name, { configurable: true, enumerable: true, value, writable: false });
}

export class CustomEvent extends Event {
  constructor(type, options = {}) {
    options ??= {};
    super(type, options);
    internal(this, "__detail", options.detail ?? null);
  }
  get detail() { return this.__detail; }
}

export class MessageEvent extends Event {
  constructor(type, options = {}) {
    options ??= {};
    super(type, options);
    this.data = options.data ?? null;
    this.origin = options.origin ?? "";
    this.lastEventId = options.lastEventId ?? "";
    this.source = options.source ?? null;
    this.ports = options.ports ?? [];
  }
}

export class ErrorEvent extends Event {
  constructor(type, options = {}) {
    options ??= {};
    super(type, options);
    internal(this, "__message", options.message ?? "");
    internal(this, "__filename", options.filename ?? "");
    internal(this, "__lineno", options.lineno ?? 0);
    internal(this, "__colno", options.colno ?? 0);
    internal(this, "__error", options.error ?? null);
  }
  get message() { return this.__message; }
  get filename() { return this.__filename; }
  get lineno() { return this.__lineno; }
  get colno() { return this.__colno; }
  get error() { return this.__error; }
}

export class CloseEvent extends Event {
  constructor(type, options = {}) {
    options ??= {};
    super(type, options);
    this.wasClean = Boolean(options.wasClean);
    this.code = options.code ?? 0;
    this.reason = options.reason ?? "";
  }
}

function exposeProperties(prototype, names) {
  for (const name of names) {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    if (descriptor) Object.defineProperty(prototype, name, { ...descriptor, enumerable: true });
  }
}

exposeProperties(Event.prototype, [
  "type", "target", "currentTarget", "srcElement", "eventPhase", "bubbles", "cancelable",
  "defaultPrevented", "composed", "isTrusted", "timeStamp", "cancelBubble", "returnValue",
  "preventDefault", "stopPropagation", "stopImmediatePropagation", "composedPath",
]);
exposeProperties(CustomEvent.prototype, ["detail"]);
exposeProperties(ErrorEvent.prototype, ["message", "filename", "lineno", "colno", "error"]);
exposeProperties(CloseEvent.prototype, ["wasClean", "code", "reason"]);

export class EventTarget {
  constructor() {
    Object.defineProperty(this, listenerStore, { value: new Map() });
  }

  addEventListener(type, callback, options = {}) {
    if (callback == null) return;
    if (typeof callback !== "function" && (callback === null || typeof callback.handleEvent !== "function")) {
      throw new TypeError("Event listener must be callable");
    }
    const name = domString(type);
    const normalized = normalizeOptions(options);
    const list = this[listenerStore].get(name) ?? [];
    if (list.some(entry => entry.callback === callback && entry.capture === normalized.capture)) return;
    if (normalized.signal !== null && typeof normalized.signal.addEventListener !== "function") {
      throw new TypeError("Event listener signal must be an AbortSignal");
    }
    if (normalized.signal?.aborted) return;
    const entry = { callback, ...normalized, abort: null };
    list.push(entry);
    this[listenerStore].set(name, list);
    if (entry.signal) {
      entry.abort = () => this.removeEventListener(name, callback, { capture: entry.capture });
      entry.signal.addEventListener("abort", entry.abort, { once: true });
    }
  }

  removeEventListener(type, callback, options = {}) {
    const name = domString(type);
    const capture = captureOption(options);
    const list = this[listenerStore].get(name);
    if (!list) return;
    const remaining = list.filter(entry => {
      const remove = entry.callback === callback && entry.capture === capture;
      if (remove && entry.signal && entry.abort) entry.signal.removeEventListener("abort", entry.abort);
      return !remove;
    });
    if (remaining.length === 0) this[listenerStore].delete(name);
    else this[listenerStore].set(name, remaining);
  }

  dispatchEvent(event) {
    if (!(event instanceof Event)) throw new TypeError("The event must be an Event");
    if (event.__dispatching) throw new DOMException("The event is already being dispatched", "InvalidStateError");
    event.__dispatching = true;
    event.__target = this;
    event.__currentTarget = this;
    event.__eventPhase = Event.AT_TARGET;
    event.__stopped = false;
    event.__immediateStopped = false;
    try {
      for (const entry of [...(this[listenerStore].get(event.type) ?? [])]) {
        if (event.__immediateStopped) break;
        if (!this[listenerStore].get(event.type)?.includes(entry)) continue;
        if (entry.once) this.removeEventListener(event.type, entry.callback, { capture: entry.capture });
        event.__inPassive = entry.passive;
        try {
          if (typeof entry.callback === "function") entry.callback.call(this, event);
          else entry.callback.handleEvent.call(entry.callback, event);
        } finally {
          event.__inPassive = false;
        }
      }
    } finally {
      event.__dispatching = false;
      event.__eventPhase = Event.NONE;
      event.__inPassive = false;
    }
    return !event.defaultPrevented;
  }
}

exposeProperties(EventTarget.prototype, ["addEventListener", "removeEventListener", "dispatchEvent"]);

export class EventEmitter {
  static defaultMaxListeners = 10;
  static errorMonitor = Symbol("events.errorMonitor");
  static EventEmitter = EventEmitter;

  constructor() {
    internal(this, "__events", new Map());
    internal(this, "__maxListeners", undefined);
  }
  on(name, listener) { return this.__add(name, listener, false, false); }
  addListener(name, listener) { return this.on(name, listener); }
  once(name, listener) { return this.__add(name, listener, true, false); }
  prependListener(name, listener) { return this.__add(name, listener, false, true); }
  prependOnceListener(name, listener) { return this.__add(name, listener, true, true); }
  off(name, listener) { return this.removeListener(name, listener); }

  removeListener(name, listener) {
    const list = this.__events.get(name) ?? [];
    for (let index = 0; index < list.length; index += 1) {
      const entry = list[index];
      if (entry.listener !== listener && entry.wrapper !== listener) continue;
      this.__removeEntry(name, entry);
      break;
    }
    return this;
  }

  removeEventListener(name, listener) { return this.removeListener(name, listener); }
  addEventListener(name, listener) { return this.on(name, listener); }
  removeAllListeners(name) {
    if (name === undefined) this.__events.clear();
    else this.__events.delete(name);
    return this;
  }

  emit(name, ...args) {
    const list = [...(this.__events.get(name) ?? [])];
    if (name === "error") {
      for (const entry of [...(this.__events.get(EventEmitter.errorMonitor) ?? [])]) {
        if (entry.once) this.__removeEntry(EventEmitter.errorMonitor, entry);
        entry.listener.apply(this, args);
      }
      if (list.length === 0) {
        throw args[0] instanceof Error ? args[0] : new Error(String(args[0] ?? "Unhandled error"));
      }
    }
    for (const entry of list) {
      if (entry.once) this.__removeEntry(name, entry);
      entry.listener.apply(this, args);
    }
    return list.length > 0;
  }

  listeners(name) { return [...(this.__events.get(name) ?? [])].map(entry => entry.listener); }
  rawListeners(name) { return [...(this.__events.get(name) ?? [])].map(entry => entry.wrapper ?? entry.listener); }
  listenerCount(name, listener) {
    const list = this.__events.get(name) ?? [];
    return listener === undefined ? list.length : list.filter(entry => entry.listener === listener || entry.wrapper === listener).length;
  }
  eventNames() { return [...this.__events.keys()]; }
  setMaxListeners(value) {
    const max = Number(value);
    if (Number.isNaN(max) || max < 0 || (max !== Infinity && !Number.isInteger(max))) {
      throw new RangeError("The maximum number of listeners must be a non-negative number");
    }
    this.__maxListeners = max;
    return this;
  }
  getMaxListeners() { return this.__maxListeners ?? EventEmitter.defaultMaxListeners; }

  __add(name, listener, once, prepend) {
    if (typeof listener !== "function") throw new TypeError("listener must be a function");
    const entry = { listener, once, wrapper: once ? (...args) => listener.apply(this, args) : listener };
    entry.wrapper.listener = listener;
    const list = this.__events.get(name) ?? [];
    if (prepend) list.unshift(entry);
    else list.push(entry);
    this.__events.set(name, list);
    return this;
  }

  __removeEntry(name, entry) {
    const list = this.__events.get(name);
    const index = list?.indexOf(entry) ?? -1;
    if (index < 0) return;
    list.splice(index, 1);
    if (list.length === 0) this.__events.delete(name);
  }

  static listenerCount(emitter, name, listener) { return emitter.listenerCount(name, listener); }
}

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

export function once(emitter, name, options = {}) {
  if (options === null || (typeof options !== "object" && typeof options !== "function")) {
    return Promise.reject(new TypeError("options must be an object"));
  }
  const signal = options.signal;
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
    if (signal !== undefined && signal !== null && typeof signal.addEventListener !== "function") {
      reject(new TypeError("options.signal must be an AbortSignal"));
      return;
    }
    if (signal?.aborted) {
      onError(signal.reason);
      return;
    }
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
  let ended = false;
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
    ended = true;
    wake?.();
    wake = undefined;
  };
  const onAbort = () => onError(signal.reason);
  const cleanup = () => {
    emitter.removeListener(name, listener);
    if (name !== "error") emitter.removeListener("error", onError);
    signal?.removeEventListener?.("abort", onAbort);
  };
  if (signal !== undefined && signal !== null && typeof signal.addEventListener !== "function") {
    throw new TypeError("options.signal must be an AbortSignal");
  }
  if (signal?.aborted) {
    failed = true;
    failure = signal.reason;
    ended = true;
  } else {
    emitter.on(name, listener);
    if (name !== "error") emitter.on("error", onError);
    signal?.addEventListener("abort", onAbort, { once: true });
  }
  const iterator = (async function* () {
    try {
      while (true) {
        if (queue.length > 0) {
          yield queue.shift();
          continue;
        }
        if (failed) throw failure;
        if (ended) return;
        await new Promise(resolve => { wake = resolve; });
      }
    } finally {
      cleanup();
    }
  })();
  return iterator;
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
EventEmitter.init = () => {};
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
export const listenerCount = EventEmitter.listenerCount;
export const addAbortListener = EventEmitter.addAbortListener;
export const captureRejectionSymbol = EventEmitter.captureRejectionSymbol;
export const captureRejections = EventEmitter.captureRejections;
export const getEventListeners = EventEmitter.getEventListeners;
export const getMaxListeners = EventEmitter.getMaxListeners;
export const init = EventEmitter.init;
export const setMaxListeners = EventEmitter.setMaxListeners;
export const usingDomains = EventEmitter.usingDomains;
export default EventEmitter;
