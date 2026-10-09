import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { Plugin } from "../src/index.js";
import { connectNative, disconnectNative } from "../src/testing.js";

class Example extends Plugin {
  constructor() {
    super("example");
  }

  get native(): boolean {
    return this.hasNativeTransport;
  }

  run(arguments_?: unknown): Promise<unknown> {
    return this.call("run", arguments_);
  }

  watch(next: (value: number) => void, error: (error: DOMException) => void): () => void {
    return this.listen("watch", next, error, { every: 1 });
  }
}

const example = new Example();

afterEach(() => {
  disconnectNative();
  Reflect.deleteProperty(globalThis, "__tokamakNativeCall");
  Reflect.deleteProperty(globalThis, "__tokamakNativeListen");
});

void test("calls through the Worker's native calls without a page transport", async () => {
  const calls: unknown[][] = [];
  globalThis.__tokamakNativeCall = (...arguments_) => {
    calls.push(arguments_);
    return Promise.resolve("ran");
  };

  assert.equal(example.native, true);
  assert.equal(await example.run({ fast: true }), "ran");
  assert.equal(await example.run(), "ran");
  assert.deepEqual(calls, [
    ["example", "run", { fast: true }],
    ["example", "run", null],
  ]);
});

void test("passes on the Worker's native call failures", async () => {
  const failure = new DOMException("Prompt needs UI", "NeedsUIError");
  globalThis.__tokamakNativeCall = () => Promise.reject(failure);

  await assert.rejects(example.run(), failure);
});

void test("listens through the Worker's native listeners without a page transport", () => {
  const values: number[] = [];
  const errors: string[] = [];
  let unsubscribed = 0;
  globalThis.__tokamakNativeListen = (plugin, method, arguments_, next, error) => {
    assert.deepEqual([plugin, method, arguments_], ["example", "watch", { every: 1 }]);
    next(1);
    error(new DOMException("No fix", "NotReadableError"));
    next(2);
    return () => unsubscribed++;
  };

  const stop = example.watch((value) => values.push(value), (error) => errors.push(error.name));
  stop();

  assert.deepEqual(values, [1, 2]);
  assert.deepEqual(errors, ["NotReadableError"]);
  assert.equal(unsubscribed, 1);
});

void test("uses the page transport where the page has one", async () => {
  const native = connectNative();
  globalThis.__tokamakNativeCall = () => Promise.reject(new Error("the Worker's calls were used"));

  const result = example.run();
  native.respond({ value: "page" });

  assert.equal(await result, "page");
});

void test("is unsupported without a native transport", async () => {
  assert.equal(example.native, false);
  await assert.rejects(
    example.run(),
    (error: unknown) => error instanceof DOMException && error.name === "NotSupportedError",
  );
  assert.throws(
    () => example.watch(() => undefined, () => undefined),
    (error: unknown) => error instanceof DOMException && error.name === "NotSupportedError",
  );
});

void test("listens for pagehide only once a page transport connects", async () => {
  const listened: string[] = [];
  Reflect.set(globalThis, "addEventListener", (type: string) => listened.push(type));
  try {
    // A fresh copy of the module, imported without a page transport, as the Worker imports it.
    const fresh = (await import(new URL("../src/index.ts?fresh", import.meta.url).href)) as typeof import("../src/index.js");
    class Fresh extends fresh.Plugin {
      constructor() {
        super("fresh");
      }

      run(): Promise<unknown> {
        return this.call("run");
      }
    }
    const plugin = new Fresh();
    await assert.rejects(plugin.run());
    assert.deepEqual(listened, []);

    connectNative();
    void plugin.run();
    assert.deepEqual(listened, ["pagehide"]);
  } finally {
    Reflect.deleteProperty(globalThis, "addEventListener");
  }
});
