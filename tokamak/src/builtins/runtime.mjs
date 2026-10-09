import "./globals.mjs";
import { createTracing } from "./tracing.mjs";
import { Request, Response } from "../network/fetch.mjs";
import { URL } from "../network/url.mjs";

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
class ExecutionContext {
  waitUntil(value) { waitUntilValues.push(Promise.resolve(value)); }
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
