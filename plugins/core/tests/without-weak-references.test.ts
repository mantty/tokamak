import assert from "node:assert/strict";
import { test } from "node:test";
import type { NativeReference, TokamakContext } from "../src/index.js";

// Workers lack FinalizationRegistry before compatibility date 2025-05-05 or with `disable_weak_ref`.
Reflect.deleteProperty(globalThis, "FinalizationRegistry");
const { Plugin } = await import("../src/index.js");
const { connectNative } = await import("../src/testing.js");

class Producer extends Plugin {
  constructor(ctx: TokamakContext) { super(ctx, "producer"); }
  frame(): Promise<NativeReference> { return this.resource("frame"); }
}

void test("native references load and release without FinalizationRegistry", async () => {
  const native = connectNative();
  const pending = new Producer(native.ctx).frame();
  native.respond({ value: "frame-1" }, false);
  const frame = await pending;
  frame.close();
  assert.equal(native.sent.filter((request) => request.type === "cancel").length, 1);
});
