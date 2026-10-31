import "./globals.ts";
import { createTracing } from "./tracing.ts";
import { reportError } from "../events/web.ts";
import type { AssetFetcher, BodyChunk, Bootstrap, NativeHost, NativePlatform, PluginReply, WaitUntilObserver } from "tokamak:host";
import type { NativeConnection } from "../context.ts";
import { Request, Response, hostRequest, hostResponse } from "../network/fetch.ts";
import { URL } from "../network/url.ts";
import { nativeError, platformGlobal } from "../native-protocol.mjs";
import { installServices, invokeService } from "../services/bindings.ts";
import { serviceStreamCodec } from "../services/streams.ts";
import { callMethod, propertyOf } from "../globals/objects.ts";

// Binds `binding` in the environment, unless taken, to the app's assets, which
// `fetcher` reads.
function installAssets(binding: string, fetcher: AssetFetcher): void {
  if (globalThis.__tokamak_env[binding] !== undefined) return;
  globalThis.__tokamak_env[binding] = {
    async fetch(input: RequestInfo | URL, init?: RequestInit) {
      const request = new Request(input, init);
      const { status, statusText, contentType, body } = fetcher.fetch(request.method, new URL(request.url).pathname);
      return new Response(body, { status, statusText, headers: contentType ? { "content-type": contentType } : {} });
    },
  };
}

// Connects the Worker's byte endpoints to the native `host` on `platform`,
// which `@tokamakdev/plugin` reads, as its `platformGlobal` names, while the
// Worker's modules load.
function installPlugins(host: NativeHost, platform: NativePlatform): void {
  globalThis[platformGlobal] = platform;
  const encoder = new TextEncoder();
  const decoder = new TextDecoder();
  // The DOMException that a failed reply's JSON `{ name, message, cause? }` describes.
  const failure = (payload: ArrayBuffer) => {
    const value: unknown = JSON.parse(decoder.decode(payload));
    const name = propertyOf(value, "name");
    const message = propertyOf(value, "message");
    if (typeof name !== "string" || typeof message !== "string") throw new TypeError("A native failure must have a name and a message");
    return nativeError({ name, message, cause: propertyOf(value, "cause") });
  };
  // The next chunk of a call's body, with a failure encoded as an endpoint error.
  const pull = (reader: ReadableStreamDefaultReader<Uint8Array>) => async (): Promise<BodyChunk> => {
    try {
      const { value, done } = await reader.read();
      if (done) return { payload: new Uint8Array(), failed: false, done: true };
      if (!(value instanceof Uint8Array)) throw new TypeError("A native call's body must contain Uint8Array chunks");
      return { payload: value, failed: false, done: false };
    } catch (error) {
      const failure = { name: propertyOf(error, "name") ?? "Error", message: String(propertyOf(error, "message") ?? error) };
      return { payload: encoder.encode(JSON.stringify(failure)), failed: true, done: true };
    }
  };
  const connection: NativeConnection = {
    platform,
    async call(plugin, method, payload, { body, signal } = {}) {
      signal?.throwIfAborted();
      const reader = body?.getReader();
      const { reply, cancellation } = host.call(plugin, method, payload, reader && pull(reader));
      const cancel = () => { cancellation.cancel(); };
      signal?.addEventListener("abort", cancel, { once: true });
      try {
        const { payload, failed } = await reply;
        if (failed) throw failure(payload);
        return new Uint8Array(payload);
      } catch (error) {
        throw signal?.aborted ? signal.reason : error;
      } finally {
        signal?.removeEventListener("abort", cancel);
        reader?.cancel().catch(() => undefined);
      }
    },
    listen(plugin, method, payload, { next, error, overflow = "error" }) {
      const deliver = ({ payload, failed, done }: PluginReply) => {
        try {
          if (failed) error(failure(payload), done);
          else next(new Uint8Array(payload), done);
        } catch (thrown) { reportError(thrown); }
      };
      const subscription = host.subscribe(plugin, method, payload, { deliver, overflow });
      return () => { subscription.cancel(); };
    },
  };
  globalThis.__tokamak_context[Symbol.for("tokamak.native")] = connection;
}

// Delivers the event `name`, whose encoded data is `event`, to the
// `TokamakEvents` the Worker's entry module `exports`, resolving to the
// encoded reply and the names of the events that have listeners.
async function dispatchEvent(exports: object, name: string, event: Uint8Array): Promise<unknown> {
  const TokamakEvents: unknown = Reflect.get(exports, "TokamakEvents");
  if (typeof TokamakEvents !== "function") throw new TypeError("The Worker does not export TokamakEvents");
  const events: unknown = Reflect.construct(TokamakEvents, [globalThis.__tokamak_context, globalThis.__tokamak_env]);
  return callMethod(events, "dispatch", name, event);
}

// The storage part attaches the app's storage bindings when it has any.
if (globalThis.__tokamak_storage) await (await import("../storage/bindings.ts")).install(globalThis.__tokamak_env, globalThis.__tokamak_storage);
const waitUntilValues: Promise<unknown>[] = [];
let waitUntilObserver: WaitUntilObserver | undefined;
function trackWaitUntil(observer: WaitUntilObserver): void { waitUntilObserver = observer; }

/** The execution context passed to the Worker's handlers. */
export class ExecutionContext {
  access = undefined;
  cache = undefined;
  exports = undefined;
  props = {};
  tracing = createTracing();
  /** Values that plugins read by registered symbol, such as the native connection. */
  [key: symbol]: unknown;
  waitUntil(value: unknown): void {
    const task = Promise.resolve(value);
    waitUntilValues.push(task);
    const observer = waitUntilObserver;
    if (observer) {
      observer.observe(true);
      const settled = () => { observer.observe(false); };
      task.then(settled, settled);
    }
  }
  abort(): never { throw new Error("The Worker isolate was aborted"); }
  passThroughOnException(): void {}
}
Object.defineProperty(globalThis, "__tokamak_drain_wait_until", {
  configurable: false,
  enumerable: false,
  value: async () => {
    while (waitUntilValues.length) {
      await Promise.allSettled(waitUntilValues.splice(0));
    }
  },
});
globalThis.__tokamak_context = new ExecutionContext();

export const bootstrap: Bootstrap = {
  hostRequest,
  hostResponse,
  dispatchEvent,
  installAssets,
  installPlugins,
  installServices,
  invokeService,
  serviceStreams: serviceStreamCodec,
  trackWaitUntil,
};
