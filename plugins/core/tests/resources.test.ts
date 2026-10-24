import assert from "node:assert/strict";
import { setImmediate } from "node:timers/promises";
import { test } from "node:test";
import v8 from "node:v8";
import vm from "node:vm";
import { NativeEndpoint, type NativeReference, type TokamakContext } from "../src/index.js";
import { connectNative, type FakeNativeTransport } from "../src/testing.js";

const producer = (ctx: TokamakContext) => ({
  frame: (): Promise<NativeReference> => new NativeEndpoint(ctx, "producer", ["ios"]).resource("frame"),
});
const consumer = (ctx: TokamakContext) => {
  const endpoint = new NativeEndpoint(ctx, "consumer", ["ios"]);
  return { encode: (frame: NativeReference): Promise<number> => endpoint.call("encode", { frame: endpoint.reference(frame) }) };
};

async function acquire(native: FakeNativeTransport, id = "frame-1"): Promise<NativeReference> {
  const pending = producer(native.ctx).frame();
  native.respond({ value: id }, false);
  return pending;
}

const cancellations = (native: FakeNativeTransport) => native.sent.filter((message) => message.type === "cancel").length;

void test("a native reference passes its ID between plugins and reads bytes only on request", async () => {
  const native = connectNative();
  const frame = await acquire(native);
  assert.deepEqual(native.lastRequest(), { type: "subscribe", plugin: "producer", method: "frame", arguments: {} });
  const encoded = consumer(native.ctx).encode(frame);
  assert.deepEqual(native.lastRequest(), { type: "call", plugin: "consumer", method: "encode", arguments: { frame: "frame-1" } });
  native.respond({ value: 512 });
  assert.equal(await encoded, 512);
  const bytes = frame.bytes();
  assert.deepEqual(native.lastRequest(), { type: "call", plugin: "tokamak", method: "resource.read", arguments: "frame-1" });
  native.respondBytes(new Uint8Array([0, 255]));
  assert.deepEqual(await bytes, new Uint8Array([0, 255]));
});

void test("closing releases the native object once and later use fails", async () => {
  const native = connectNative();
  const frame = await acquire(native);
  frame.close();
  frame.close();
  assert.equal(cancellations(native), 1);
  assert.throws(() => consumer(native.ctx).encode(frame), { name: "InvalidStateError" });
  await assert.rejects(frame.bytes(), { name: "InvalidStateError" });
});

void test("a native reference cannot be used by another invocation", async () => {
  const frame = await acquire(connectNative());
  assert.throws(() => consumer(connectNative().ctx).encode(frame), { name: "InvalidStateError", message: /another invocation/ });
});

void test("acquisition failures reject and release the subscription", async () => {
  const failed = connectNative();
  const denied = producer(failed.ctx).frame();
  failed.respond({ error: { name: "NotAllowedError", message: "Permission denied" } }, false);
  await assert.rejects(denied, { name: "NotAllowedError" });
  assert.equal(cancellations(failed), 1);

  const ended = connectNative();
  const released = producer(ended.ctx).frame();
  ended.respond({ value: "frame-1" }, true);
  await assert.rejects(released, { name: "InvalidStateError", message: /not retained/ });

  const malformed = connectNative();
  const unreadable = producer(malformed.ctx).frame();
  malformed.respondBytes(new Uint8Array([255]), false);
  await assert.rejects(unreadable, TypeError);
  assert.equal(cancellations(malformed), 1);
});

void test("aborting an endpoint's signal stops acquisition and releases the subscription", async () => {
  const native = connectNative();
  const controller = new AbortController();
  const endpoint = new NativeEndpoint(native.ctx, "producer", ["ios"], controller.signal);
  const frame = endpoint.resource("frame");
  controller.abort(new DOMException("stopped", "AbortError"));
  await assert.rejects(frame, { name: "AbortError", message: "stopped" });
  assert.equal(cancellations(native), 1);
  await assert.rejects(endpoint.resource("frame"), { message: "stopped" });
  assert.equal(native.sent.length, 2);
});

void test("aborting an endpoint's signal after acquisition leaves the reference open", async () => {
  const native = connectNative();
  const controller = new AbortController();
  const pending = new NativeEndpoint(native.ctx, "producer", ["ios"], controller.signal).resource("frame");
  native.respond({ value: "frame-1" }, false);
  const frame = await pending;
  controller.abort();
  assert.equal(cancellations(native), 0);
  assert.equal(frame.idFor(native.ctx), "frame-1");
});

void test("garbage collection releases an unclosed native reference", async () => {
  v8.setFlagsFromString("--expose-gc");
  const gc = vm.runInNewContext("gc") as () => void;
  const native = connectNative();
  await acquire(native);
  for (let attempt = 0; attempt < 50 && cancellations(native) === 0; attempt++) {
    gc();
    await setImmediate();
  }
  assert.equal(cancellations(native), 1);
});
