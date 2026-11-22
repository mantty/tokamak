import assert from "node:assert/strict";
import { test } from "node:test";

import type { NativeConnection } from "../src/context.js";
import { capabilitiesModule, createNativeContext, nativeConnection, TokamakEntrypoint, type StartEvent } from "../src/worker.js";
import { connectNative } from "../src/testing.js";

declare global {
  // eslint-disable-next-line @typescript-eslint/no-namespace -- the test's handler merges into Tokamak.Handlers.
  namespace Tokamak {
    interface Handlers {
      _onProbed?(event: { id: string }): string | undefined | Promise<string | undefined>;
    }
  }
}

void test("a worker's handlers are typed as Tokamak.Handlers declares them", () => {
  class Native extends TokamakEntrypoint<{ label: string }> {
    _onStart({ foreground }: StartEvent) {
      this.ctx.waitUntil(Promise.resolve(foreground));
    }
    _onProbed(event: { id: string }) {
      return `${this.env.label}:${event.id}`;
    }
    _own(count: number) {
      return count + 1;
    }
  }
  const worker = new Native(createNativeContext({ waitUntil: () => undefined }), { label: "device" });
  assert.equal(worker._onProbed({ id: "home" }), "device:home");
  assert.equal(worker._own(1), 2);

  class Mistyped extends TokamakEntrypoint {
    // @ts-expect-error -- `_onStart` receives `{ foreground: boolean }`.
    _onStart(event: { foreground: string }) {
      return void event;
    }
    // @ts-expect-error -- `_onProbed` replies with a string.
    _onProbed() {
      return 1;
    }
  }
  assert.equal(new Mistyped(createNativeContext({ waitUntil: () => undefined }), {})._onProbed(), 1);
});

void test("a native context uses the connection the host attached to the execution context", async () => {
  const ordinary = { waitUntil: () => undefined };
  // @ts-expect-error -- ordinary execution contexts do not contain a native connection.
  new TokamakEntrypoint(ordinary, {});
  const offline = createNativeContext({ waitUntil: () => undefined });
  await assert.rejects(offline[nativeConnection].call("example", "run", new Uint8Array()), { name: "NotSupportedError" });
  const connection: NativeConnection = {
    call: () => Promise.resolve(new Uint8Array([42])),
    listen: () => () => undefined,
  };
  Reflect.set(globalThis, nativeConnection, connection);
  try {
    const unattached = createNativeContext({ waitUntil: () => undefined });
    await assert.rejects(unattached[nativeConnection].call("example", "run", new Uint8Array()), { name: "NotSupportedError" });
  } finally {
    Reflect.deleteProperty(globalThis, nativeConnection);
  }
  const attached = createNativeContext({ waitUntil: () => undefined, [nativeConnection]: connection });
  assert.equal(attached[nativeConnection], connection);
  assert.deepEqual(await attached[nativeConnection].call("example", "run", new Uint8Array()), new Uint8Array([42]));
});

void test("the capabilities module reports each plugin operation on the context's platform", async () => {
  const { ctx } = connectNative("android", { maps: { route: ["ios", "android"], tile: ["ios"] } });
  const response = capabilitiesModule(ctx);
  assert.equal(response.headers.get("content-type"), "text/javascript; charset=utf-8");
  assert.equal(await response.text(), 'export default {"maps":{"route":true,"tile":false}};\n');
});
