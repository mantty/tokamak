import { markHostObject } from "../globals/objects.mjs";
import { DOMException } from "../globals/dom-exception.mjs";
import { atob, btoa } from "../globals/base64.mjs";
import { console } from "../globals/console.mjs";
import process from "../globals/process.mjs";
import { clearImmediate, clearInterval, clearTimeout, setImmediate, setInterval, setTimeout } from "../globals/timers.mjs";
import { Crypto, CryptoKey, SubtleCrypto, crypto } from "../globals/crypto.mjs";
import { structuredClone } from "../globals/structured-clone.mjs";
import { Performance, PerformanceEntry, PerformanceMark, PerformanceMeasure, PerformanceObserver, PerformanceObserverEntryList, PerformanceResourceTiming, performance } from "../globals/performance.mjs";
import { CloseEvent, CustomEvent, ErrorEvent, Event, EventTarget, ExtendableEvent, FetchEvent, MessageChannel, MessageEvent, MessagePort, PromiseRejectionEvent, ScheduledEvent, TailEvent, TraceEvent, WebSocketRequestResponsePair, initEventTarget, reportError } from "../events/web.mjs";
import { AbortController, AbortSignal } from "../events/abort.mjs";
import { Blob, Body, Cache, CacheStorage, EventSource, File, FormData, Headers, Request, Response, fetch } from "../network/fetch.mjs";
import { HTMLRewriter } from "../network/html-rewriter.mjs";
import { URL, URLPattern, URLSearchParams } from "../network/url.mjs";
import { WebSocket, WebSocketPair } from "../network/websocket.mjs";
import { ReadableByteStreamController, ReadableStream, ReadableStreamBYOBReader, ReadableStreamBYOBRequest, ReadableStreamDefaultController, ReadableStreamDefaultReader, WritableStream, WritableStreamDefaultController, WritableStreamDefaultWriter, TransformStream, TransformStreamDefaultController, CompressionStream, DecompressionStream, ByteLengthQueuingStrategy, CountQueuingStrategy, FixedLengthStream, IdentityTransformStream, TextDecoderStream, TextEncoderStream } from "../streams/web.mjs";
import { TextDecoder, TextEncoder } from "../streams/text.mjs";
import { Buffer } from "../node/buffer.mjs";
import { installIntlGlobals } from "../intl.mjs";

class Navigator {
  constructor() {
    markHostObject(this);
    this.userAgent = "Cloudflare-Workers";
    this.platform = "";
    this.language = "en";
    this.languages = Object.freeze(["en"]);
    this.hardwareConcurrency = 1;
  }
  sendBeacon() { return false; }
}

class WorkerGlobalScope extends EventTarget {}
class ServiceWorkerGlobalScope extends WorkerGlobalScope {}
ServiceWorkerGlobalScope.prototype[Symbol.toStringTag] = "ServiceWorkerGlobalScope";
Object.defineProperty(WorkerGlobalScope.prototype, "EventTarget", {
  configurable: true,
  enumerable: true,
  value: EventTarget,
});

Object.assign(globalThis, {
  AbortController, AbortSignal, Blob, Body, Cache, CacheStorage, ByteLengthQueuingStrategy, CountQueuingStrategy,
  CloseEvent, CustomEvent, DOMException, ErrorEvent, Event, EventSource, EventTarget, ExtendableEvent, FetchEvent, File, FormData, Headers, MessageChannel, MessageEvent, MessagePort,
  PromiseRejectionEvent, ScheduledEvent, TailEvent, TraceEvent, WebSocket, WebSocketPair, WebSocketRequestResponsePair,
  ReadableByteStreamController, ReadableStream, ReadableStreamBYOBReader, ReadableStreamBYOBRequest, ReadableStreamDefaultController, ReadableStreamDefaultReader, Request, Response, TextDecoder, TextDecoderStream, TextEncoder, TextEncoderStream,
  Crypto, CryptoKey, SubtleCrypto,
  TransformStream, TransformStreamDefaultController, CompressionStream, DecompressionStream, FixedLengthStream, IdentityTransformStream, URL, URLPattern, URLSearchParams, WritableStream, WritableStreamDefaultController, WritableStreamDefaultWriter, structuredClone,
  Navigator, Performance, PerformanceEntry, PerformanceMark, PerformanceMeasure, PerformanceObserver, PerformanceObserverEntryList, PerformanceResourceTiming,
  WorkerGlobalScope, ServiceWorkerGlobalScope,
  atob, btoa,
  fetch, reportError, setTimeout, clearTimeout, setInterval, clearInterval, setImmediate, clearImmediate,
  caches: new CacheStorage(), crypto, HTMLRewriter, navigator: new Navigator(),
  origin: "null", self: globalThis, Cloudflare: { compatibilityFlags: [] },
  Buffer, console, global: globalThis, process,
});
Object.setPrototypeOf(globalThis, ServiceWorkerGlobalScope.prototype);
initEventTarget(globalThis);
installIntlGlobals();
delete globalThis.WebAssembly;
delete globalThis.InternalError;
globalThis.performance = performance;
globalThis.scheduler = {
  wait: (delay, options = {}) => {
    if (options.signal?.aborted) return Promise.reject(options.signal.reason);
    return new Promise((resolve, reject) => {
      const id = setTimeout(() => {
        if (options.signal?.aborted) reject(options.signal.reason);
        else resolve();
      }, delay);
      options.signal?.addEventListener("abort", () => {
        clearTimeout(id);
        reject(options.signal.reason);
      }, { once: true });
    });
  },
};

// Array/TypedArray toLocaleString forward locales and options to each element,
// which the engine's own implementation does not.
function localeElements(locales, options) {
  let result = "";
  for (let index = 0; index < this.length; index += 1) {
    if (index > 0) result += ",";
    const element = this[index];
    if (element !== undefined && element !== null) result += element.toLocaleString(locales, options);
  }
  return result;
}
for (const prototype of [Array.prototype, Object.getPrototypeOf(Uint8Array.prototype)]) {
  Object.defineProperty(prototype, "toLocaleString", { value: localeElements, writable: true, configurable: true });
}
