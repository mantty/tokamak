import { markHostObject } from "../globals/objects.mjs";
import { DOMException } from "../globals/dom-exception.mjs";
import { structuredClone } from "../globals/structured-clone.mjs";
import { string } from "../globals/conversions.mjs";

function normalizeOptions(options, method) {
  if (typeof options === "boolean") {
    if (options) throw new TypeError(`${method}(): useCapture must be false.`);
    options = {};
  }
  options ??= {};
  if (options.capture) throw new TypeError(`${method}(): options.capture must be false.`);
  return {
    once: Boolean(options.once),
    passive: Boolean(options.passive),
    signal: options.signal ?? null,
  };
}

// Runs `target`'s `listeners` for `event`.
let dispatch;

export class Event {
  #type;
  #bubbles;
  #cancelable;
  #composed;
  #defaultPrevented = false;
  #eventPhase = 0;
  #timeStamp = Date.now();
  #target;
  #currentTarget;
  #dispatching = false;
  #stopped = false;
  #immediateStopped = false;
  #inPassive = false;

  constructor(type, options = {}) {
    markHostObject(this);
    options ??= {};
    this.#type = string(type);
    this.#bubbles = Boolean(options.bubbles);
    this.#cancelable = Boolean(options.cancelable);
    this.#composed = Boolean(options.composed);
  }

  get type() { return this.#type; }
  get target() { return this.#target; }
  get currentTarget() { return this.#currentTarget; }
  get srcElement() { return this.#target; }
  get eventPhase() { return this.#eventPhase; }
  get bubbles() { return this.#bubbles; }
  get cancelable() { return this.#cancelable; }
  get defaultPrevented() { return this.#defaultPrevented; }
  get composed() { return this.#composed; }
  get isTrusted() { return false; }
  get timeStamp() { return this.#timeStamp; }
  get cancelBubble() { return this.#stopped; }
  set cancelBubble(value) { if (value) this.stopPropagation(); }
  get returnValue() { return !this.#defaultPrevented; }

  preventDefault() {
    if (this.#inPassive) throw new TypeError("Unable to preventDefault inside passive event listener invocation.");
    if (this.#cancelable) this.#defaultPrevented = true;
  }
  stopPropagation() { this.#stopped = true; }
  stopImmediatePropagation() { this.#stopped = true; this.#immediateStopped = true; }
  composedPath() { return this.#target === undefined ? [] : [this.#target]; }

  #dispatch(target, listeners) {
    if (this.#dispatching) throw new DOMException("The event is already being dispatched", "InvalidStateError");
    this.#dispatching = true;
    this.#target = target;
    this.#currentTarget = target;
    this.#eventPhase = Event.AT_TARGET;
    this.#stopped = false;
    this.#immediateStopped = false;
    try {
      for (const entry of [...(listeners.get(this.#type) ?? [])]) {
        if (this.#immediateStopped) break;
        if (!listeners.get(this.#type)?.includes(entry)) continue;
        if (entry.once) removeListener(listeners, this.#type, entry.callback);
        this.#invoke(target, entry);
      }
    } finally {
      this.#dispatching = false;
      this.#eventPhase = Event.NONE;
    }
    return !this.#defaultPrevented;
  }

  #invoke(target, entry) {
    this.#inPassive = entry.passive;
    try {
      if (typeof entry.callback === "function") entry.callback.call(target, this);
      else entry.callback.handleEvent.call(entry.callback, this);
    } finally {
      this.#inPassive = false;
    }
  }

  static {
    dispatch = (event, target, listeners) => event.#dispatch(target, listeners);
  }
}

for (const [name, value] of Object.entries({ NONE: 0, CAPTURING_PHASE: 1, AT_TARGET: 2, BUBBLING_PHASE: 3 })) {
  Object.defineProperty(Event, name, { configurable: true, enumerable: true, value, writable: false });
  Object.defineProperty(Event.prototype, name, { configurable: true, enumerable: true, value, writable: false });
}

export class CustomEvent extends Event {
  #detail;
  constructor(type, options = {}) {
    options ??= {};
    super(type, options);
    this.#detail = options.detail ?? null;
  }
  get detail() { return this.#detail; }
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
  #message;
  #filename;
  #lineno;
  #colno;
  #error;
  constructor(type, options = {}) {
    options ??= {};
    super(type, options);
    this.#message = options.message ?? "";
    this.#filename = options.filename ?? "";
    this.#lineno = options.lineno ?? 0;
    this.#colno = options.colno ?? 0;
    this.#error = options.error ?? null;
  }
  get message() { return this.#message; }
  get filename() { return this.#filename; }
  get lineno() { return this.#lineno; }
  get colno() { return this.#colno; }
  get error() { return this.#error; }
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

// The listener entries of each EventTarget, by event type.
const eventListeners = new WeakMap();

// Gives `target` the listener state of a new EventTarget.
export function initEventTarget(target) { eventListeners.set(target, new Map()); }

// The listener entries of `target`; an undefined or null receiver is the global scope, as in Web IDL.
function listenersOf(target) {
  const listeners = eventListeners.get(target ?? globalThis);
  if (listeners === undefined) {
    throw new TypeError("Illegal invocation: function called with incorrect `this` reference. See https://developers.cloudflare.com/workers/observability/errors/#illegal-invocation-errors for details.");
  }
  return listeners;
}

function removeListener(listeners, type, callback) {
  const list = listeners.get(type);
  if (!list) return;
  const remaining = list.filter(entry => {
    const remove = entry.callback === callback;
    if (remove && entry.signal && entry.abort) entry.signal.removeEventListener("abort", entry.abort);
    return !remove;
  });
  if (remaining.length === 0) listeners.delete(type);
  else listeners.set(type, remaining);
}

export class EventTarget {
  constructor() { markHostObject(this); initEventTarget(this); }

  addEventListener(type, callback, options = {}) {
    const listeners = listenersOf(this);
    if (callback == null) return;
    if (typeof callback !== "function" && typeof callback.handleEvent !== "function") {
      throw new TypeError("Event listener must be callable");
    }
    const name = string(type);
    const normalized = normalizeOptions(options, "addEventListener");
    const list = listeners.get(name) ?? [];
    if (list.some(entry => entry.callback === callback)) return;
    if (normalized.signal !== null && typeof normalized.signal.addEventListener !== "function") {
      throw new TypeError("Event listener signal must be an AbortSignal");
    }
    if (normalized.signal?.aborted) return;
    const entry = { callback, ...normalized, abort: null };
    list.push(entry);
    listeners.set(name, list);
    if (entry.signal) {
      entry.abort = () => removeListener(listeners, name, callback);
      entry.signal.addEventListener("abort", entry.abort, { once: true });
    }
  }

  removeEventListener(type, callback, options = {}) {
    const listeners = listenersOf(this);
    const name = string(type);
    normalizeOptions(options, "removeEventListener");
    removeListener(listeners, name, callback);
  }

  dispatchEvent(event) {
    const listeners = listenersOf(this);
    if (!(event instanceof Event)) throw new TypeError("The event must be an Event");
    return dispatch(event, this ?? globalThis, listeners);
  }
}

exposeProperties(EventTarget.prototype, ["addEventListener", "removeEventListener", "dispatchEvent"]);

// Dispatches an ErrorEvent for `error` on the global scope.
export function reportError(error) {
  const event = new ErrorEvent("error", { error, message: error?.message ?? String(error) });
  if (!globalThis.dispatchEvent?.(event)) console.error(error);
}

export class ExtendableEvent extends Event {
  waitUntil(promise) { globalThis.__tokamak_context?.waitUntil(promise); }
}

export class FetchEvent extends ExtendableEvent {
  #response = null;
  constructor(type, options = {}) {
    options ??= {};
    super(type, options);
    this.request = options.request;
    this.clientId = options.clientId ?? "";
    this.resultingClientId = options.resultingClientId ?? "";
    this.preloadResponse = options.preloadResponse ?? Promise.resolve(undefined);
    this.handled = options.handled ?? Promise.resolve();
  }
  respondWith(response) {
    if (this.#response !== null) throw new DOMException("The fetch event has already been responded to.", "InvalidStateError");
    this.#response = Promise.resolve(response);
  }
  passThroughOnException() {}
}

export class PromiseRejectionEvent extends Event {
  constructor(type, options = {}) {
    options ??= {};
    super(type, options);
    this.promise = options.promise;
    this.reason = options.reason;
  }
}

export class ScheduledEvent extends ExtendableEvent {
  constructor(type, options = {}) {
    options ??= {};
    super(type, options);
    this.cron = options.cron ?? "";
    this.scheduledTime = options.scheduledTime ?? Date.now();
  }
  noRetry() {}
}

export class TailEvent extends Event {
  constructor(type, options = {}) {
    super(type, options);
    Object.assign(this, options);
  }
}

export class TraceEvent extends Event {
  constructor(type, options = {}) {
    super(type, options);
    Object.assign(this, options);
  }
}

export class WebSocketRequestResponsePair {
  #request;
  #response;
  constructor(request, response) {
    markHostObject(this);
    this.#request = request;
    this.#response = response;
  }
  get request() { return this.#request; }
  get response() { return this.#response; }
}

// Connects two new ports to each other.
let entangle;

export class MessagePort extends EventTarget {
  #peer = null;
  #closed = false;
  #onmessage = null;

  get onmessage() { return this.#onmessage; }
  set onmessage(value) { this.#onmessage = value; }

  postMessage(value) {
    const peer = this.#peer;
    if (this.#closed || peer === null || peer.#closed) {
      throw new DOMException("The port is closed.", "InvalidStateError");
    }
    const copy = structuredClone(value);
    Promise.resolve().then(() => {
      if (peer.#closed) return;
      const event = new MessageEvent("message", { data: copy });
      peer.dispatchEvent(event);
      peer.onmessage?.call(peer, event);
    });
  }

  start() {}

  close() {
    this.#closed = true;
    this.#peer = null;
  }

  static {
    entangle = (first, second) => {
      first.#peer = second;
      second.#peer = first;
    };
  }
}

export class MessageChannel {
  constructor() {
    markHostObject(this);
    this.port1 = new MessagePort();
    this.port2 = new MessagePort();
    entangle(this.port1, this.port2);
  }
}
