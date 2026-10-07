import { markHostObject } from "../globals/objects.mjs";
import { DOMException } from "../globals/dom-exception.mjs";
import { setTimeout } from "../globals/timers.mjs";
import { Event, EventTarget } from "./web.mjs";

function abortError() { return new DOMException("The operation was aborted", "AbortError"); }

// Aborts `signal` with `reason` unless it is already aborted.
let abort;

export class AbortSignal extends EventTarget {
  #aborted = false;
  #reason;
  #onabort = null;

  get aborted() { return this.#aborted; }
  get reason() { return this.#reason; }
  get onabort() { return this.#onabort; }
  set onabort(value) { this.#onabort = value; }
  throwIfAborted() { if (this.aborted) throw this.reason; }

  static abort(reason = abortError()) {
    const signal = new AbortSignal();
    abort(signal, reason);
    return signal;
  }
  static timeout(milliseconds) {
    const signal = new AbortSignal();
    setTimeout(() => abort(signal, new DOMException("The operation timed out.", "TimeoutError")), milliseconds);
    return signal;
  }
  static any(signals) {
    const result = new AbortSignal();
    for (const signal of signals) {
      if (signal.aborted) { abort(result, signal.reason); break; }
      signal.addEventListener("abort", () => abort(result, signal.reason), { once: true });
    }
    return result;
  }

  static {
    abort = (signal, reason) => {
      if (signal.#aborted) return;
      signal.#aborted = true;
      signal.#reason = reason;
      const event = new Event("abort");
      signal.dispatchEvent(event);
      signal.#onabort?.call(signal, event);
    };
  }
}

export class AbortController {
  #signal = new AbortSignal();
  constructor() { markHostObject(this); }
  get signal() { return this.#signal; }
  abort(reason) { abort(this.#signal, reason === undefined ? abortError() : reason); }
}
