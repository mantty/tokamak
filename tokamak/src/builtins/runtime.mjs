import "./globals.mjs";
import { createTracing } from "./tracing.mjs";
import { reportError } from "../events/web.mjs";
import { Request, Response } from "../network/fetch.mjs";
import { URL } from "../network/url.mjs";
import { nativeError, platformGlobal } from "../native-protocol.mjs";

export { installServices, invokeService } from "../services/bindings.mjs";

export { hostRequest, hostResponse } from "../network/fetch.mjs";

// Binds `binding` in the environment, unless taken, to the app's assets, which
// `fetchAsset(method, path)` answers.
export function installAssets(binding, fetchAsset) {
  if (globalThis.__tokamak_env[binding] !== undefined) return;
  globalThis.__tokamak_env[binding] = {
    async fetch(input, init) {
      const request = new Request(input, init);
      const { status, statusText, contentType, body } = fetchAsset(request.method, new URL(request.url).pathname);
      return new Response(body, { status, statusText, headers: contentType ? { "content-type": contentType } : {} });
    },
  };
}

// Connects the Worker's byte endpoints to the native host on `platform`,
// which `@tokamakdev/plugin` reads, as its `platformGlobal` names, while the
// Worker's modules load.
export function installPlugins(call, subscribe, platform) {
  globalThis[platformGlobal] = platform;
  const encoder = new TextEncoder();
  const decoder = new TextDecoder();
  const decoded = payload => JSON.parse(decoder.decode(payload));
  const buffer = value => value.byteOffset === 0 && value.byteLength === value.buffer.byteLength
    ? value.buffer : value.buffer.slice(value.byteOffset, value.byteOffset + value.byteLength);
  const result = ({ payload, failed }) => {
    if (failed) throw nativeError(decoded(payload));
    return new Uint8Array(payload);
  };
  // The next chunk of a call's body, with a failure encoded as an endpoint error.
  const pull = reader => async () => {
    try {
      const { value, done } = await reader.read();
      if (done) return { payload: new Uint8Array(), failed: false, done: true };
      if (!(value instanceof Uint8Array)) throw new TypeError("A native call's body must contain Uint8Array chunks");
      return { payload: value, failed: false, done: false };
    } catch (error) {
      const failure = { name: error?.name ?? "Error", message: String(error?.message ?? error) };
      return { payload: encoder.encode(JSON.stringify(failure)), failed: true, done: true };
    }
  };
  const connection = {
    platform,
    async call(plugin, method, payload, { body, signal } = {}) {
      signal?.throwIfAborted();
      const reader = body?.getReader();
      const [reply, cancel] = reader
        ? call(plugin, method, buffer(payload), pull(reader))
        : call(plugin, method, buffer(payload));
      signal?.addEventListener("abort", cancel, { once: true });
      try {
        return result(await reply);
      } catch (error) {
        throw signal?.aborted ? signal.reason : error;
      } finally {
        signal?.removeEventListener("abort", cancel);
        reader?.cancel().catch(() => undefined);
      }
    },
    listen(plugin, method, payload, { next, error, overflow = "error" }) {
      return subscribe(plugin, method, buffer(payload), reply => {
        let value;
        try { value = result(reply); }
        catch (failure) {
          try { error(failure, reply.done); } catch (thrown) { reportError(thrown); }
          return;
        }
        try { next(value, reply.done); } catch (thrown) { reportError(thrown); }
      }, overflow);
    },
  };
  globalThis.__tokamak_context[Symbol.for("tokamak.native")] = connection;
}

// Delivers the event `name`, whose encoded data is `event`, to the
// `TokamakEvents` the Worker's entry module `exports`, returning the encoded
// reply and the names of the events that have listeners.
export async function dispatchEvent(exports, name, event) {
  const { TokamakEvents } = exports;
  if (typeof TokamakEvents !== "function") throw new TypeError("The Worker does not export TokamakEvents");
  const events = new TokamakEvents(globalThis.__tokamak_context, globalThis.__tokamak_env);
  const { reply, listened } = await events.dispatch(name, new Uint8Array(event));
  return { reply: reply.slice().buffer, listened };
}

// The storage part attaches the app's storage bindings when it has any.
if (globalThis.__tokamak_storage) await (await import("../storage/bindings.mjs")).install(globalThis.__tokamak_env, globalThis.__tokamak_storage);
const waitUntilValues = [];
let observeWaitUntil;
export function trackWaitUntil(observer) { observeWaitUntil = observer; }
class ExecutionContext {
  waitUntil(value) {
    const task = Promise.resolve(value);
    waitUntilValues.push(task);
    if (observeWaitUntil) {
      observeWaitUntil(true);
      const settled = () => observeWaitUntil(false);
      task.then(settled, settled);
    }
  }
  abort() { throw new Error("The Worker isolate was aborted"); }
  passThroughOnException() {}
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
globalThis.__tokamak_context = Object.assign(Object.create(ExecutionContext.prototype), {
  access: undefined,
  cache: undefined,
  exports: undefined,
  props: {},
  tracing: createTracing(),
});

export { openServiceStream, receiveServiceStream } from "../services/streams.mjs";
