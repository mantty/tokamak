import "./globals.mjs";
import { createTracing } from "./tracing.mjs";
import { reportError } from "../events/web.mjs";
import { DOMException } from "../globals/dom-exception.mjs";
import { Request, Response } from "../network/fetch.mjs";
import { URL } from "../network/url.mjs";

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

// Connects the Worker's byte endpoints to the native host.
export function installPlugins(call, subscribe) {
  const decoder = new TextDecoder();
  const decoded = payload => JSON.parse(decoder.decode(payload));
  const buffer = value => value.byteOffset === 0 && value.byteLength === value.buffer.byteLength
    ? value.buffer : value.buffer.slice(value.byteOffset, value.byteOffset + value.byteLength);
  const result = ({ payload, failed }) => {
    if (failed) {
      const error = decoded(payload);
      throw new DOMException(error.message, error.name);
    }
    return new Uint8Array(payload);
  };
  const connection = {
    async call(plugin, method, payload) {
      return result(await call(plugin, method, buffer(payload)));
    },
    listen(plugin, method, payload, next, error) {
      return subscribe(plugin, method, buffer(payload), reply => {
        let value;
        try { value = result(reply); }
        catch (failure) {
          try { error(failure, reply.done); } catch (thrown) { reportError(thrown); }
          return;
        }
        try { next(value, reply.done); } catch (thrown) { reportError(thrown); }
      });
    },
  };
  globalThis[Symbol.for("tokamak.native")] = connection;
  globalThis.__tokamak_context[Symbol.for("tokamak.native")] = globalThis[Symbol.for("tokamak.native")];
}

// Delivers the event `name`, whose JSON is `event`, to the `TokamakEvents` the
// Worker's entry module `exports`, returning the reply as JSON and the names of
// the events that have listeners.
export async function dispatchEvent(exports, name, event) {
  const { TokamakEvents } = exports;
  if (typeof TokamakEvents !== "function") throw new TypeError("The Worker does not export TokamakEvents");
  const events = new TokamakEvents(globalThis.__tokamak_context, globalThis.__tokamak_env);
  const { reply, listened } = await events.dispatch(name, JSON.parse(event));
  return { reply: JSON.stringify(reply) ?? "null", listened };
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
