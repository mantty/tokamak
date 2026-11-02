import { callMethod, isObject, markHostObject, objectOf } from "../globals/objects.ts";
import { DOMException } from "../globals/dom-exception.ts";
import { structuredClone } from "../globals/structured-clone.ts";
import { string } from "../globals/conversions.ts";

// An event listener: a function, or an object with a `handleEvent` method.
type Listener = ((this: unknown, event: Event) => unknown) | { handleEvent(event: Event): unknown };

// The AbortSignal a listener's `signal` option names: an object with the
// methods this module calls.
interface ListenerSignal {
  readonly aborted?: unknown;
  addEventListener(type: "abort", listener: () => void, options: { once: boolean }): unknown;
  removeEventListener(type: "abort", listener: () => void): unknown;
}

interface ListenerEntry {
  callback: Listener;
  once: boolean;
  passive: boolean;
  signal: ListenerSignal | null;
  abort: (() => void) | null;
}

// The listener entries of an EventTarget, by event type.
type Listeners = Map<string, ListenerEntry[]>;

function normalizeOptions(options: unknown, method: string) {
  if (typeof options === "boolean") {
    if (options) throw new TypeError(`${method}(): useCapture must be false.`);
    options = {};
  }
  const init = objectOf(options ?? {});
  if (init.capture) throw new TypeError(`${method}(): options.capture must be false.`);
  return {
    once: Boolean(init.once),
    passive: Boolean(init.passive),
    signal: init.signal ?? null,
  };
}

// An object with `addEventListener`, which is all that is checked of a signal.
function isListenerSignal(value: unknown): value is ListenerSignal {
  return isObject(value) && typeof value.addEventListener === "function";
}

// Runs `target`'s `listeners` for `event`; false when a listener called preventDefault().
let dispatch: (event: Event, target: object, listeners: Listeners) => boolean;

export class Event {
  #type: string;
  #bubbles: boolean;
  #cancelable: boolean;
  #composed: boolean;
  // Set even when the event is not cancelable.
  #preventDefaultCalled = false;
  #eventPhase = 0;
  #timeStamp = Date.now();
  #target: object | undefined;
  #currentTarget: object | undefined;
  #dispatching = false;
  #stopped = false;
  #immediateStopped = false;
  #inPassive = false;
  declare static readonly NONE: 0;
  declare static readonly CAPTURING_PHASE: 1;
  declare static readonly AT_TARGET: 2;
  declare static readonly BUBBLING_PHASE: 3;

  constructor(type: unknown, options: unknown = {}) {
    markHostObject(this);
    const init = objectOf(options ?? {});
    this.#type = string(type);
    this.#bubbles = Boolean(init.bubbles);
    this.#cancelable = Boolean(init.cancelable);
    this.#composed = Boolean(init.composed);
  }

  get type() { return this.#type; }
  get target() { return this.#target; }
  get currentTarget() { return this.#currentTarget; }
  get srcElement() { return this.#target; }
  get eventPhase() { return this.#eventPhase; }
  get bubbles() { return this.#bubbles; }
  get cancelable() { return this.#cancelable; }
  get defaultPrevented() { return this.#cancelable && this.#preventDefaultCalled; }
  get composed() { return this.#composed; }
  get isTrusted() { return false; }
  get timeStamp() { return this.#timeStamp; }
  get cancelBubble() { return this.#stopped; }
  set cancelBubble(value: unknown) { if (value) this.stopPropagation(); }
  get returnValue() { return !this.defaultPrevented; }

  preventDefault() {
    if (this.#inPassive) throw new TypeError("Unable to preventDefault inside passive event listener invocation.");
    this.#preventDefaultCalled = true;
  }
  stopPropagation() { this.#stopped = true; }
  stopImmediatePropagation() { this.#stopped = true; this.#immediateStopped = true; }
  composedPath(): object[] { return this.#target === undefined ? [] : [this.#target]; }

  #dispatch(target: object, listeners: Listeners): boolean {
    if (this.#dispatching) throw new DOMException("The event is already being dispatched", "InvalidStateError");
    this.#dispatching = true;
    this.#target = target;
    this.#currentTarget = target;
    this.#eventPhase = Event.AT_TARGET;
    this.#stopped = false;
    this.#immediateStopped = false;
    this.#preventDefaultCalled = false;
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
    return !this.#preventDefaultCalled;
  }

  #invoke(target: object, entry: ListenerEntry): void {
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

const phases = { NONE: 0, CAPTURING_PHASE: 1, AT_TARGET: 2, BUBBLING_PHASE: 3 } as const;
for (const [name, value] of Object.entries(phases)) {
  Object.defineProperty(Event, name, { configurable: true, enumerable: true, value, writable: false });
  Object.defineProperty(Event.prototype, name, { configurable: true, enumerable: true, value, writable: false });
}
// The phase constants defined above.
export interface Event extends Readonly<typeof phases> {}

export class CustomEvent extends Event {
  #detail: unknown;
  constructor(type: unknown, options: unknown = {}) {
    const init = objectOf(options ?? {});
    super(type, init);
    this.#detail = init.detail ?? null;
  }
  get detail() { return this.#detail; }
}

export class MessageEvent extends Event {
  declare data: unknown;
  declare origin: unknown;
  declare lastEventId: unknown;
  declare source: unknown;
  declare ports: unknown;
  constructor(type: unknown, options: unknown = {}) {
    const init = objectOf(options ?? {});
    super(type, init);
    this.data = init.data ?? null;
    this.origin = init.origin ?? "";
    this.lastEventId = init.lastEventId ?? "";
    this.source = init.source ?? null;
    this.ports = init.ports ?? [];
  }
}

export class ErrorEvent extends Event {
  #message: unknown;
  #filename: unknown;
  #lineno: unknown;
  #colno: unknown;
  #error: unknown;
  constructor(type: unknown, options: unknown = {}) {
    const init = objectOf(options ?? {});
    super(type, init);
    this.#message = init.message ?? "";
    this.#filename = init.filename ?? "";
    this.#lineno = init.lineno ?? 0;
    this.#colno = init.colno ?? 0;
    this.#error = init.error ?? null;
  }
  get message() { return this.#message; }
  get filename() { return this.#filename; }
  get lineno() { return this.#lineno; }
  get colno() { return this.#colno; }
  get error() { return this.#error; }
}

export class CloseEvent extends Event {
  declare wasClean: boolean;
  declare code: unknown;
  declare reason: unknown;
  constructor(type: unknown, options: unknown = {}) {
    const init = objectOf(options ?? {});
    super(type, init);
    this.wasClean = Boolean(init.wasClean);
    this.code = init.code ?? 0;
    this.reason = init.reason ?? "";
  }
}

function exposeProperties(prototype: object, names: string[]): void {
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
const eventListeners = new WeakMap<object, Listeners>();

// Gives `target` the listener state of a new EventTarget.
export function initEventTarget(target: object): void { eventListeners.set(target, new Map()); }

// The listener entries of `target`; an undefined or null receiver is the global scope, as in Web IDL.
function listenersOf(target: object | null | undefined): Listeners {
  const listeners = eventListeners.get(target ?? globalThis);
  if (listeners === undefined) {
    throw new TypeError("Illegal invocation: function called with incorrect `this` reference. See https://developers.cloudflare.com/workers/observability/errors/#illegal-invocation-errors for details.");
  }
  return listeners;
}

function removeListener(listeners: Listeners, type: string, callback: unknown): void {
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

  addEventListener(type: unknown, callback: unknown, options: unknown = {}): void {
    const listeners = listenersOf(this);
    if (callback == null) return;
    if (!isListener(callback)) throw new TypeError("Event listener must be callable");
    const name = string(type);
    const { once, passive, signal } = normalizeOptions(options, "addEventListener");
    const list = listeners.get(name) ?? [];
    if (list.some(entry => entry.callback === callback)) return;
    if (signal !== null && !isListenerSignal(signal)) throw new TypeError("Event listener signal must be an AbortSignal");
    if (signal?.aborted) return;
    const entry: ListenerEntry = { callback, once, passive, signal, abort: null };
    list.push(entry);
    listeners.set(name, list);
    if (entry.signal) {
      entry.abort = () => removeListener(listeners, name, callback);
      entry.signal.addEventListener("abort", entry.abort, { once: true });
    }
  }

  removeEventListener(type: unknown, callback: unknown, options: unknown = {}): void {
    const listeners = listenersOf(this);
    const name = string(type);
    normalizeOptions(options, "removeEventListener");
    removeListener(listeners, name, callback);
  }

  dispatchEvent(event: unknown): boolean {
    const listeners = listenersOf(this);
    if (!(event instanceof Event)) throw new TypeError("The event must be an Event");
    return dispatch(event, this ?? globalThis, listeners);
  }
}

exposeProperties(EventTarget.prototype, ["addEventListener", "removeEventListener", "dispatchEvent"]);

// Whether `value` can be called as an event listener.
function isListener(value: unknown): value is Listener {
  return typeof value === "function" || (isObject(value) && typeof value.handleEvent === "function");
}

export function logUncaught(error: unknown): void { console.error("Uncaught", error); }

// Dispatches an ErrorEvent for `error` on the global scope and logs `error` unless a listener calls preventDefault().
export function reportError(error: unknown): void {
  const event = new ErrorEvent("error", { error, message: `Uncaught ${String(error)}` });
  if (dispatch(event, globalThis, listenersOf(globalThis))) logUncaught(error);
}

export class ExtendableEvent extends Event {
  waitUntil(promise: unknown): void { globalThis.__tokamak_context?.waitUntil(promise); }
}

export class FetchEvent extends ExtendableEvent {
  #response: Promise<unknown> | null = null;
  declare request: unknown;
  declare clientId: unknown;
  declare resultingClientId: unknown;
  declare preloadResponse: unknown;
  declare handled: unknown;
  constructor(type: unknown, options: unknown = {}) {
    const init = objectOf(options ?? {});
    super(type, init);
    this.request = init.request;
    this.clientId = init.clientId ?? "";
    this.resultingClientId = init.resultingClientId ?? "";
    this.preloadResponse = init.preloadResponse ?? Promise.resolve(undefined);
    this.handled = init.handled ?? Promise.resolve();
  }
  respondWith(response: unknown): void {
    if (this.#response !== null) throw new DOMException("The fetch event has already been responded to.", "InvalidStateError");
    this.#response = Promise.resolve(response);
  }
  passThroughOnException() {}
}

export class PromiseRejectionEvent extends Event {
  declare promise: unknown;
  declare reason: unknown;
  constructor(type: unknown, options: unknown = {}) {
    const init = objectOf(options ?? {});
    super(type, init);
    this.promise = init.promise;
    this.reason = init.reason;
  }
}

export class ScheduledEvent extends ExtendableEvent {
  declare cron: unknown;
  declare scheduledTime: unknown;
  constructor(type: unknown, options: unknown = {}) {
    const init = objectOf(options ?? {});
    super(type, init);
    this.cron = init.cron ?? "";
    this.scheduledTime = init.scheduledTime ?? Date.now();
  }
  noRetry() {}
}

export class TailEvent extends Event {
  constructor(type: unknown, options: unknown = {}) {
    super(type, options);
    Object.assign(this, options);
  }
}

export class TraceEvent extends Event {
  constructor(type: unknown, options: unknown = {}) {
    super(type, options);
    Object.assign(this, options);
  }
}

export class WebSocketRequestResponsePair {
  #request: unknown;
  #response: unknown;
  constructor(request: unknown, response: unknown) {
    markHostObject(this);
    this.#request = request;
    this.#response = response;
  }
  get request() { return this.#request; }
  get response() { return this.#response; }
}

// Connects two new ports to each other.
let entangle: (first: MessagePort, second: MessagePort) => void;

export class MessagePort extends EventTarget {
  #peer: MessagePort | null = null;
  #closed = false;
  #onmessage: unknown = null;

  get onmessage() { return this.#onmessage; }
  set onmessage(value: unknown) { this.#onmessage = value; }

  postMessage(value: unknown): void {
    const peer = this.#peer;
    if (this.#closed || peer === null || peer.#closed) {
      throw new DOMException("The port is closed.", "InvalidStateError");
    }
    const copy = structuredClone(value);
    Promise.resolve().then(() => {
      if (peer.#closed) return;
      const event = new MessageEvent("message", { data: copy });
      peer.dispatchEvent(event);
      const handler = peer.onmessage;
      if (handler != null) callMethod(handler, "call", peer, event);
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
  declare port1: MessagePort;
  declare port2: MessagePort;
  constructor() {
    markHostObject(this);
    this.port1 = new MessagePort();
    this.port2 = new MessagePort();
    entangle(this.port1, this.port2);
  }
}
