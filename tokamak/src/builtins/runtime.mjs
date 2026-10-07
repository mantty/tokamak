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
