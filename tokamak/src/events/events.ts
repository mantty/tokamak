// A port of workerd's node:events, which follows Node's.
import { AsyncResource } from "../node/async-hooks.ts";
import { abortError, codedError } from "../node/errors.ts";
import { inspect } from "../node/util.ts";
import { emitProcessWarning } from "./process-warnings.ts";
import { type AnyFunction, callMethod, hasMethod, objectOf } from "../globals/objects.ts";
import { invalidArgType, outOfRange, validateAbortSignal, validateFunction, validateObject } from "../node/validators.ts";

const kRejection = Symbol.for("nodejs.rejection");
const kCapture = Symbol("kCapture");
const kErrorMonitor = Symbol("events.errorMonitor");
const kMaxEventTargetListeners = Symbol("events.maxEventTargetListeners");
const kMaxEventTargetListenersWarned = Symbol("events.maxEventTargetListenersWarned");

type EventName = string | symbol;
// A listener, and the listener a `once` wrapper calls.
type Listener = AnyFunction & { readonly listener?: unknown };
type ListenerList = Listener[] & { warned?: boolean };
// An emitter's listeners, by event, in an object without a prototype.
type EventMap = Record<EventName, Listener | ListenerList | undefined>;

export interface EventEmitter {
  _events: EventMap | undefined;
  _eventsCount: number;
  _maxListeners: number | undefined;
  domain: unknown;
  [kCapture]: boolean;
  setMaxListeners(n: unknown): this;
  getMaxListeners(): number;
  emit(type: EventName, ...args: unknown[]): boolean;
  addListener(type: EventName, listener: unknown): this;
  on(type: EventName, listener: unknown): this;
  prependListener(type: EventName, listener: unknown): this;
  once(type: EventName, listener: unknown): this;
  prependOnceListener(type: EventName, listener: unknown): this;
  removeListener(type: EventName, listener: unknown): this;
  off(type: EventName, listener: unknown): this;
  removeAllListeners(type?: EventName): this;
  listeners(type: EventName): unknown[];
  rawListeners(type: EventName): unknown[];
  listenerCount(type: EventName): number;
  eventNames(): EventName[];
}

interface EventEmitterConstructor {
  new (options?: unknown): EventEmitter;
  (this: EventEmitter, options?: unknown): EventEmitter;
  readonly prototype: EventEmitter;
  init(this: EventEmitter, options?: unknown): void;
  on: typeof on;
  once: typeof once;
  getEventListeners: typeof getEventListeners;
  getMaxListeners: typeof _getMaxListeners;
  setMaxListeners: typeof setMaxListeners;
  listenerCount: typeof listenerCount;
  addAbortListener: typeof addAbortListener;
  EventEmitter: EventEmitterConstructor;
  EventEmitterAsyncResource: new (options?: unknown) => EventEmitter;
  usingDomains: boolean;
  captureRejectionSymbol: symbol;
  errorMonitor: typeof kErrorMonitor;
  captureRejections: boolean;
  defaultMaxListeners: number;
  readonly kMaxEventTargetListeners: typeof kMaxEventTargetListeners;
  readonly kMaxEventTargetListenersWarned: typeof kMaxEventTargetListenersWarned;
}

// A function rather than a class, as in Node, so that a constructor written
// before ES classes can call it. TypeScript gives a function expression no
// construct signature, so its type is asserted.
export const EventEmitter = function (this: EventEmitter, opts?: unknown) {
  EventEmitter.init.call(this, opts);
  return this;
} as EventEmitterConstructor;

class EventEmitterReferencingAsyncResource extends AsyncResource {
  #eventEmitter: EventEmitter;

  constructor(emitter: EventEmitter) {
    super("");
    this.#eventEmitter = emitter;
  }

  get eventEmitter() {
    return this.#eventEmitter;
  }
}

export class EventEmitterAsyncResource extends EventEmitter {
  #asyncResource: EventEmitterReferencingAsyncResource;

  constructor(options?: unknown) {
    super(options);
    this.#asyncResource = new EventEmitterReferencingAsyncResource(this);
  }

  get asyncResource() {
    return this.#asyncResource;
  }

  emit(event: EventName, ...args: unknown[]): boolean {
    this.#asyncResource.runInAsyncScope(super.emit, this, event, ...args);
    return true;
  }
}

export function addAbortListener(signal: unknown, listener: unknown) {
  if (signal === undefined) throw invalidArgType("signal", "AbortSignal", signal);
  validateAbortSignal(signal, "signal");
  validateFunction(listener, "listener");
  let removeEventListener: (() => void) | undefined;
  if (signal.aborted) {
    queueMicrotask(() => listener());
  } else {
    signal.addEventListener("abort", listener, { once: true });
    removeEventListener = () => signal.removeEventListener("abort", listener);
  }
  return {
    __proto__: null,
    [Symbol.dispose]() {
      removeEventListener?.();
    },
  };
}

export default EventEmitter;

EventEmitter.on = on;
EventEmitter.once = once;
EventEmitter.getEventListeners = getEventListeners;
EventEmitter.getMaxListeners = _getMaxListeners;
EventEmitter.setMaxListeners = setMaxListeners;
EventEmitter.listenerCount = listenerCount;
EventEmitter.EventEmitter = EventEmitter;
EventEmitter.usingDomains = false;
EventEmitter.captureRejectionSymbol = kRejection;
EventEmitter.errorMonitor = kErrorMonitor;
EventEmitter.EventEmitterAsyncResource = EventEmitterAsyncResource;

export const captureRejectionSymbol = EventEmitter.captureRejectionSymbol;
export const captureRejections = false;
export const errorMonitor = EventEmitter.errorMonitor;
export const getMaxListeners = _getMaxListeners;
export const usingDomains = EventEmitter.usingDomains;
export let defaultMaxListeners = 10;

function nonNegativeNumber(name: string, value: unknown): asserts value is number {
  if (typeof value !== "number" || value < 0 || Number.isNaN(value)) throw outOfRange(name, "a non-negative number", value);
}

Object.defineProperties(EventEmitter, {
  captureRejections: {
    get() {
      return EventEmitter.prototype[kCapture];
    },
    set(value: unknown) {
      if (typeof value !== "boolean") throw invalidArgType("EventEmitter.captureRejections", "boolean", value);
      EventEmitter.prototype[kCapture] = value;
    },
    enumerable: true,
  },
  defaultMaxListeners: {
    enumerable: true,
    get() {
      return defaultMaxListeners;
    },
    set(value: unknown) {
      nonNegativeNumber("defaultMaxListeners", value);
      defaultMaxListeners = value;
    },
  },
  kMaxEventTargetListeners: { value: kMaxEventTargetListeners },
  kMaxEventTargetListenersWarned: { value: kMaxEventTargetListenersWarned },
});

Object.defineProperty(EventEmitter.prototype, kCapture, { value: false, writable: true });

EventEmitter.init = function (opts) {
  if (this._events === undefined || this._events === Object.getPrototypeOf(this)._events) {
    this._events = emptyEventMap();
    this._eventsCount = 0;
  }
  this._maxListeners ??= undefined;
  this.domain = null;
  const options = opts == null ? undefined : objectOf(opts);
  if (options?.captureRejections) {
    if (typeof options.captureRejections !== "boolean") {
      throw invalidArgType("options.captureRejections", "boolean", options.captureRejections);
    }
    this[kCapture] = true;
  } else {
    this[kCapture] = EventEmitter.prototype[kCapture];
  }
};

export function setMaxListeners(n: unknown = defaultMaxListeners, ...eventTargets: unknown[]): void {
  nonNegativeNumber("n", n);
  if (eventTargets.length === 0) {
    defaultMaxListeners = n;
    return;
  }
  for (const target of eventTargets) {
    if (target instanceof EventTarget) {
      Object.assign(target, { [kMaxEventTargetListeners]: n, [kMaxEventTargetListenersWarned]: false });
    } else if (hasMethod(target, "setMaxListeners")) {
      target.setMaxListeners(n);
    } else {
      throw invalidArgType("eventTargets", ["EventEmitter", "EventTarget"], target);
    }
  }
}

EventEmitter.prototype._events = undefined;
EventEmitter.prototype._eventsCount = 0;
EventEmitter.prototype._maxListeners = undefined;
EventEmitter.addAbortListener = addAbortListener;

// Routes a rejection of the promise a listener returned to the emitter when
// it captures rejections.
function addCatch(that: EventEmitter, promise: {}, type: EventName, args: unknown[]): void {
  if (!that[kCapture]) return;
  try {
    const then = objectOf(promise).then;
    if (typeof then === "function") {
      then.call(promise, undefined, (error: unknown) => queueMicrotask(() => emitUnhandledRejectionOrErr(that, error, type, args)));
    }
  } catch (error) {
    that.emit("error", error);
  }
}

function emitUnhandledRejectionOrErr(emitter: EventEmitter, error: unknown, type: EventName, args: unknown[]): void {
  const rejection = objectOf(emitter)[kRejection];
  if (typeof rejection === "function") {
    Reflect.apply(rejection, emitter, [error, type, ...args]);
    return;
  }
  // Capturing stays off while the error is emitted, so a rejecting error
  // listener does not loop.
  const previous = emitter[kCapture];
  try {
    emitter[kCapture] = false;
    emitter.emit("error", error);
  } finally {
    emitter[kCapture] = previous;
  }
}

EventEmitter.prototype.setMaxListeners = function setMaxListeners(n: unknown) {
  nonNegativeNumber("n", n);
  this._maxListeners = n;
  return this;
};

// Named as workerd names it, which `getMaxListeners.name` shows.
function _getMaxListeners(that: EventEmitter): number {
  return that._maxListeners === undefined ? EventEmitter.defaultMaxListeners : that._maxListeners;
}

EventEmitter.prototype.getMaxListeners = function getMaxListeners() {
  return _getMaxListeners(this);
};

EventEmitter.prototype.emit = function emit(type: EventName, ...args: unknown[]) {
  let doError = type === "error";
  const events = this._events;
  if (events !== undefined) {
    if (doError && events[kErrorMonitor] !== undefined) this.emit(kErrorMonitor, ...args);
    doError = doError && events.error === undefined;
  } else if (!doError) {
    return false;
  }
  if (doError) {
    const error = args.length > 0 ? args[0] : undefined;
    if (error instanceof Error) throw error;
    let stringified: unknown;
    try { stringified = inspect(error); } catch { stringified = error; }
    throw Object.assign(codedError(Error, `Unhandled error. (${stringified})`, "ERR_UNHANDLED_ERROR"), { context: error });
  }
  const handler = events?.[type];
  if (handler === undefined) return false;
  if (typeof handler === "function") {
    const result = Reflect.apply(handler, this, args);
    if (result !== undefined && result !== null) addCatch(this, result, type, args);
  } else {
    for (const listener of [...handler]) {
      const result = Reflect.apply(listener, this, args);
      if (result !== undefined && result !== null) addCatch(this, result, type, args);
    }
  }
  return true;
};

function emptyEventMap(): EventMap {
  return Object.create(null);
}

// Checks that `listener` is a function; any function may have a `listener` property.
function validateListener(listener: unknown): asserts listener is Listener {
  validateFunction(listener, "listener");
}

function _addListener<T extends EventEmitter>(target: T, type: EventName, listener: unknown, prepend: boolean): T {
  validateListener(listener);
  let events = target._events;
  let existing: Listener | ListenerList | undefined;
  if (events === undefined) {
    events = target._events = emptyEventMap();
    target._eventsCount = 0;
  } else {
    // "newListener" is emitted before the listener is added, so a
    // "newListener" listener does not hear itself.
    if (events.newListener !== undefined) {
      target.emit("newListener", type, listener.listener ?? listener);
      events = target._events;
      // A "newListener" listener cleared the emitter's private state.
      if (events === undefined) return target;
    }
    existing = events[type];
  }
  if (existing === undefined) {
    events[type] = listener;
    ++target._eventsCount;
    return target;
  }
  if (typeof existing === "function") {
    existing = events[type] = prepend ? [listener, existing] : [existing, listener];
  } else if (prepend) {
    existing.unshift(listener);
  } else {
    existing.push(listener);
  }
  const max = _getMaxListeners(target);
  if (max > 0 && existing.length > max && !existing.warned) {
    existing.warned = true;
    warnLeak(target, type, existing.length);
  }
  return target;
}

function warnLeak(target: EventEmitter, type: EventName, count: number): void {
  const listeners = `${count} ${String(type)} listeners`;
  console.log(`Possible EventEmitter memory leak detected. ${listeners} added to an EventEmitter. Use emitter.setMaxListeners() to increase limit`);
  const warning = Object.assign(
    new Error(`Possible EventEmitter memory leak detected. ${listeners} added to ${inspect(target, { depth: -1 })}. Use emitter.setMaxListeners() to increase limit`),
    { name: "MaxListenersExceededWarning", emitter: target, type, count },
  );
  emitProcessWarning(warning);
}

EventEmitter.prototype.addListener = function addListener(type: EventName, listener: unknown) {
  return _addListener(this, type, listener, false);
};

EventEmitter.prototype.on = EventEmitter.prototype.addListener;

EventEmitter.prototype.prependListener = function prependListener(type: EventName, listener: unknown) {
  return _addListener(this, type, listener, true);
};

// The state of a listener that `once` added.
interface OnceState {
  fired: boolean;
  wrapFn: Listener | undefined;
  target: EventEmitter;
  type: EventName;
  listener: AnyFunction;
}

function onceWrapper(this: OnceState, ...args: unknown[]): unknown {
  if (this.fired) return undefined;
  this.target.removeListener(this.type, this.wrapFn);
  this.fired = true;
  return Reflect.apply(this.listener, this.target, args);
}

function onceWrap(target: EventEmitter, type: EventName, listener: AnyFunction): Listener {
  const state: OnceState = { fired: false, wrapFn: undefined, target, type, listener };
  const wrapped = Object.assign(onceWrapper.bind(state), { listener });
  state.wrapFn = wrapped;
  return wrapped;
}

EventEmitter.prototype.once = function once(type: EventName, listener: unknown) {
  validateFunction(listener, "listener");
  this.on(type, onceWrap(this, type, listener));
  return this;
};

EventEmitter.prototype.prependOnceListener = function prependOnceListener(type: EventName, listener: unknown) {
  validateFunction(listener, "listener");
  this.prependListener(type, onceWrap(this, type, listener));
  return this;
};

EventEmitter.prototype.removeListener = function removeListener(type: EventName, listener: unknown) {
  validateFunction(listener, "listener");
  const events = this._events;
  const list = events?.[type];
  if (events === undefined || list === undefined) return this;
  if (list === listener || ("listener" in list && list.listener === listener)) {
    if (--this._eventsCount === 0) {
      this._events = emptyEventMap();
    } else {
      delete events[type];
      if (events.removeListener) this.emit("removeListener", type, ("listener" in list && list.listener) || listener);
    }
    return this;
  }
  if (typeof list === "function") return this;
  const position = list.findLastIndex(entry => entry === listener || entry.listener === listener);
  if (position < 0) return this;
  list.splice(position, 1);
  if (list.length === 1) events[type] = list[0];
  if (events.removeListener !== undefined) this.emit("removeListener", type, listener);
  return this;
};

EventEmitter.prototype.off = EventEmitter.prototype.removeListener;

EventEmitter.prototype.removeAllListeners = function removeAllListeners(type: EventName) {
  const events = this._events;
  if (events === undefined) return this;
  if (events.removeListener === undefined) {
    if (arguments.length === 0) {
      this._events = emptyEventMap();
      this._eventsCount = 0;
    } else if (events[type] !== undefined) {
      if (--this._eventsCount === 0) this._events = emptyEventMap();
      else delete events[type];
    }
    return this;
  }
  if (arguments.length === 0) {
    for (const key of Reflect.ownKeys(events)) {
      if (key !== "removeListener") this.removeAllListeners(key);
    }
    this.removeAllListeners("removeListener");
    this._events = emptyEventMap();
    this._eventsCount = 0;
    return this;
  }
  const listeners = events[type];
  if (typeof listeners === "function") {
    this.removeListener(type, listeners);
  } else if (listeners !== undefined) {
    for (let index = listeners.length - 1; index >= 0; index--) this.removeListener(type, listeners[index]);
  }
  return this;
};

function listenersOf(target: EventEmitter, type: EventName, unwrap: boolean): unknown[] {
  const listener = target._events?.[type];
  if (listener === undefined) return [];
  if (typeof listener === "function") return unwrap ? [listener.listener || listener] : [listener];
  return unwrap ? listener.map(entry => (typeof entry.listener === "function" ? entry.listener : entry)) : [...listener];
}

EventEmitter.prototype.listeners = function listeners(type: EventName) {
  return listenersOf(this, type, true);
};

EventEmitter.prototype.rawListeners = function rawListeners(type: EventName) {
  return listenersOf(this, type, false);
};

const ownListenerCount = function listenerCount(this: EventEmitter, type: EventName): number {
  const listener = this._events?.[type];
  if (typeof listener === "function") return 1;
  return listener?.length ?? 0;
};

EventEmitter.prototype.listenerCount = ownListenerCount;

export function listenerCount(emitter: EventEmitter, type: EventName): number {
  if (typeof emitter.listenerCount === "function") return emitter.listenerCount(type);
  return ownListenerCount.call(emitter, type);
}

EventEmitter.prototype.eventNames = function eventNames() {
  return this._eventsCount > 0 ? Reflect.ownKeys(this._events || {}) : [];
};

export function getEventListeners(emitterOrTarget: unknown, type: EventName): unknown {
  if (hasMethod(emitterOrTarget, "listeners")) return emitterOrTarget.listeners(type);
  // Like workerd, an EventTarget does not report its listeners.
  if (emitterOrTarget instanceof EventTarget) return [];
  throw invalidArgType("emitter", ["EventEmitter", "EventTarget"], emitterOrTarget);
}

export async function once(emitter: unknown, name: EventName, options: unknown = {}): Promise<unknown[]> {
  validateObject(options, "options");
  const { signal } = options;
  validateAbortSignal(signal, "options.signal");
  if (signal?.aborted) throw abortError(signal.reason);
  return new Promise((resolve, reject) => {
    const errorListener = (error: unknown) => {
      callMethod(emitter, "removeListener", name, resolver);
      if (signal != null) removeAgnosticListener(signal, "abort", abortListener);
      reject(error);
    };
    const resolver = (...args: unknown[]) => {
      if (hasMethod(emitter, "removeListener")) emitter.removeListener("error", errorListener);
      if (signal != null) removeAgnosticListener(signal, "abort", abortListener);
      resolve(args);
    };
    addAgnosticListener(emitter, name, resolver, { once: true });
    // An EventTarget has no "error" event semantics.
    if (name !== "error" && hasMethod(emitter, "once")) emitter.once("error", errorListener);
    function abortListener() {
      removeAgnosticListener(emitter, name, resolver);
      removeAgnosticListener(emitter, "error", errorListener);
      reject(abortError());
    }
    if (signal != null) addAgnosticListener(signal, "abort", abortListener, { once: true });
  });
}

const AsyncIteratorPrototype = Object.getPrototypeOf(Object.getPrototypeOf(async function* () {}).prototype);

function iterResult(value: unknown, done: boolean): IteratorResult<unknown> {
  return { value, done };
}

function removeAgnosticListener(emitter: unknown, name: EventName, listener: AnyFunction, flags: unknown = undefined): void {
  if (hasMethod(emitter, "removeListener")) emitter.removeListener(name, listener);
  else if (hasMethod(emitter, "removeEventListener")) emitter.removeEventListener(name, listener, flags);
  else throw invalidArgType("emitter", "EventEmitter", emitter);
}

function addAgnosticListener(emitter: unknown, name: EventName, listener: AnyFunction, flags: { once?: boolean } = {}): void {
  if (hasMethod(emitter, "on")) {
    if (flags?.once) callMethod(emitter, "once", name, listener);
    else emitter.on(name, listener);
  } else if (hasMethod(emitter, "addEventListener")) {
    emitter.addEventListener(name, (arg: unknown) => listener(arg), flags);
  } else {
    throw invalidArgType("emitter", "EventEmitter", emitter);
  }
}

export function on(emitter: unknown, event: EventName, options: unknown = {}): AsyncIterableIterator<unknown> {
  const signal = options == null ? undefined : objectOf(options).signal;
  validateAbortSignal(signal, "options.signal");
  if (signal?.aborted) throw abortError();
  const unconsumedEvents: unknown[][] = [];
  const unconsumedPromises: { resolve: (result: IteratorResult<unknown>) => void; reject: (error: unknown) => void }[] = [];
  let error: unknown = null;
  let finished = false;
  const iterator = Object.setPrototypeOf({
    next() {
      const value = unconsumedEvents.shift();
      if (value) return Promise.resolve(iterResult(value, false));
      // Only the first read after an error rejects.
      if (error) {
        const rejected = Promise.reject(error);
        error = null;
        return rejected;
      }
      if (finished) return Promise.resolve(iterResult(undefined, true));
      return new Promise((resolve, reject) => unconsumedPromises.push({ resolve, reject }));
    },

    return() {
      removeAgnosticListener(emitter, event, eventHandler);
      removeAgnosticListener(emitter, "error", errorHandler);
      if (signal) removeAgnosticListener(signal, "abort", abortListener, { once: true });
      finished = true;
      for (const promise of unconsumedPromises) promise.resolve(iterResult(undefined, true));
      return Promise.resolve(iterResult(undefined, true));
    },

    throw(thrown: unknown) {
      if (!thrown || !(thrown instanceof Error)) throw invalidArgType("EventEmitter.AsyncIterator", "Error", thrown);
      error = thrown;
      removeAgnosticListener(emitter, event, eventHandler);
      removeAgnosticListener(emitter, "error", errorHandler);
    },

    [Symbol.asyncIterator]() {
      return this;
    },
  }, AsyncIteratorPrototype);
  addAgnosticListener(emitter, event, eventHandler);
  if (event !== "error" && hasMethod(emitter, "on")) emitter.on("error", errorHandler);
  if (signal) addAgnosticListener(signal, "abort", abortListener, { once: true });
  return iterator;

  function abortListener() {
    errorHandler(abortError());
  }

  function eventHandler(...args: unknown[]) {
    const promise = unconsumedPromises.shift();
    if (promise) promise.resolve(iterResult(args, false));
    else unconsumedEvents.push(args);
  }

  function errorHandler(thrown: unknown) {
    finished = true;
    const toError = unconsumedPromises.shift();
    if (toError) toError.reject(thrown);
    else error = thrown;
    iterator.return();
  }
}
