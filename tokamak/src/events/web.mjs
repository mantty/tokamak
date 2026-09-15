import { markHostObject } from "../globals/objects.mjs";
export { CloseEvent, CustomEvent, ErrorEvent, Event, EventTarget, MessageEvent } from "./events.mjs";

import { Event, EventTarget, MessageEvent } from "./events.mjs";

export class ExtendableEvent extends Event {
  constructor(type, options = {}) {
    super(type, options);
    this.__waitUntil = [];
  }
  waitUntil(promise) {
    this.__waitUntil.push(Promise.resolve(promise));
    globalThis.__tokamak_context?.waitUntil(promise);
  }
}

export class FetchEvent extends ExtendableEvent {
  constructor(type, options = {}) {
    options ??= {};
    super(type, options);
    this.request = options.request;
    this.clientId = options.clientId ?? "";
    this.resultingClientId = options.resultingClientId ?? "";
    this.preloadResponse = options.preloadResponse ?? Promise.resolve(undefined);
    this.handled = options.handled ?? Promise.resolve();
    this.__response = null;
  }
  respondWith(response) {
    if (this.__response !== null) throw new DOMException("The fetch event has already been responded to.", "InvalidStateError");
    this.__response = Promise.resolve(response);
  }
  passThroughOnException() { this.__passThrough = true; }
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
  noRetry() { this.__noRetry = true; }
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
  constructor(request, response) {
    markHostObject(this);
    Object.defineProperties(this, {
      __request: { configurable: true, enumerable: false, value: request },
      __response: { configurable: true, enumerable: false, value: response },
    });
  }
  get request() { return this.__request; }
  get response() { return this.__response; }
}

export class MessagePort extends EventTarget {
  constructor() {
    super();
    Object.defineProperties(this, {
      __onmessage: { configurable: true, enumerable: false, writable: true, value: null },
      __onmessageerror: { configurable: true, enumerable: false, writable: true, value: null },
    });
    this.__peer = null;
    this.__closed = false;
  }

  get onmessage() { return this.__onmessage; }
  set onmessage(value) { this.__onmessage = value; }

  postMessage(value) {
    if (this.__closed || this.__peer === null || this.__peer.__closed) {
      throw new DOMException("The port is closed.", "InvalidStateError");
    }
    const peer = this.__peer;
    const copy = globalThis.structuredClone?.(value);
    Promise.resolve().then(() => {
      if (peer.__closed) return;
      const event = new MessageEvent("message", { data: copy });
      peer.dispatchEvent(event);
      peer.onmessage?.call(peer, event);
    });
  }

  start() {}

  close() {
    this.__closed = true;
    this.__peer = null;
  }
}

export class MessageChannel {
  constructor() {
    markHostObject(this);
    this.port1 = new MessagePort();
    this.port2 = new MessagePort();
    this.port1.__peer = this.port2;
    this.port2.__peer = this.port1;
  }
}
