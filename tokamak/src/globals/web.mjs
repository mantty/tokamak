import { markHostObject } from "./objects.mjs";
import { DOMException } from "./dom-exception.mjs";
import { atob, btoa } from "./base64.mjs";
import { clearImmediate, clearInterval, clearTimeout, setImmediate, setInterval, setTimeout } from "./timers.mjs";
import { Crypto, CryptoKey, SubtleCrypto, crypto } from "./crypto.mjs";
import { structuredClone } from "./structured-clone.mjs";
import { Performance, PerformanceEntry, PerformanceMark, PerformanceMeasure, PerformanceObserver, PerformanceObserverEntryList, PerformanceResourceTiming, performance } from "./performance.mjs";
import { CloseEvent, CustomEvent, ErrorEvent, Event, EventTarget, ExtendableEvent, FetchEvent, MessageChannel, MessageEvent, MessagePort, PromiseRejectionEvent, ScheduledEvent, TailEvent, TraceEvent, WebSocketRequestResponsePair, reportError } from "../events/web.mjs";
import { AbortController, AbortSignal } from "../events/abort.mjs";
import { Blob, Body, Cache, CacheStorage, EventSource, File, FormData, Headers, Request, Response, fetch } from "../network/fetch.mjs";
import { HTMLRewriter } from "../network/html-rewriter.mjs";
import { URL, URLPattern, URLSearchParams } from "../network/url.mjs";
import { ReadableByteStreamController, ReadableStream, ReadableStreamBYOBReader, ReadableStreamBYOBRequest, ReadableStreamDefaultController, ReadableStreamDefaultReader, WritableStream, WritableStreamDefaultController, WritableStreamDefaultWriter, TransformStream, TransformStreamDefaultController, CompressionStream, DecompressionStream, ByteLengthQueuingStrategy, CountQueuingStrategy, FixedLengthStream, IdentityTransformStream, TextDecoderStream, TextEncoderStream } from "../streams/web.mjs";
import { TextDecoder, TextEncoder } from "../streams/text.mjs";
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

export function installWebGlobals() {
  Object.assign(globalThis, {
    AbortController, AbortSignal, Blob, Body, Cache, CacheStorage, ByteLengthQueuingStrategy, CountQueuingStrategy,
    CloseEvent, CustomEvent, DOMException, ErrorEvent, Event, EventSource, EventTarget, ExtendableEvent, FetchEvent, File, FormData, Headers, MessageChannel, MessageEvent, MessagePort,
    PromiseRejectionEvent, ScheduledEvent, TailEvent, TraceEvent, WebSocketRequestResponsePair,
    ReadableByteStreamController, ReadableStream, ReadableStreamBYOBReader, ReadableStreamBYOBRequest, ReadableStreamDefaultController, ReadableStreamDefaultReader, Request, Response, TextDecoder, TextDecoderStream, TextEncoder, TextEncoderStream,
    Crypto, CryptoKey, SubtleCrypto,
    TransformStream, TransformStreamDefaultController, CompressionStream, DecompressionStream, FixedLengthStream, IdentityTransformStream, URL, URLPattern, URLSearchParams, WritableStream, WritableStreamDefaultController, WritableStreamDefaultWriter, structuredClone,
    Navigator, Performance, PerformanceEntry, PerformanceMark, PerformanceMeasure, PerformanceObserver, PerformanceObserverEntryList, PerformanceResourceTiming,
    WorkerGlobalScope, ServiceWorkerGlobalScope,
    atob, btoa,
    fetch, reportError, setTimeout, clearTimeout, setInterval, clearInterval, setImmediate, clearImmediate,
    caches: new CacheStorage(), crypto, HTMLRewriter, navigator: new Navigator(),
    origin: "null", self: globalThis, Cloudflare: { compatibilityFlags: [] },
  });
  Object.setPrototypeOf(globalThis, ServiceWorkerGlobalScope.prototype);
  installIntlGlobals();
  delete globalThis.WebAssembly;
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
}
