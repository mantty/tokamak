import assert from "node:assert/strict";
import { test } from "node:test";
import { Plugin, type TokamakContext } from "../src/index.js";
import { decodeValues } from "../src/streams.js";
import { connectNative } from "../src/testing.js";

class Example extends Plugin {
  constructor(ctx: TokamakContext) { super(ctx, "example"); }
  run(value: unknown = null): Promise<unknown> { return this.call("run", value); }
  bytes(value: Uint8Array): Promise<Uint8Array> { return this.callBytes("bytes", value); }
  watch(): ReadableStream<Uint8Array> { return this.stream("watch", { every: 1 }); }
  watchAfter(value: Promise<unknown>): ReadableStream<Uint8Array> { return this.stream("watch", value); }
}

void test("a plugin uses the receiving invocation's connection and standard value codec", async () => {
  const first = connectNative();
  const second = connectNative();
  const result = new Example(first.ctx).run({ fast: true });
  assert.deepEqual(first.lastRequest(), { type: "call", plugin: "example", method: "run", arguments: { fast: true } });
  assert.deepEqual(second.sent, []);
  first.respond({ value: "ran" });
  assert.equal(await result, "ran");
});

void test("a plugin requires a Tokamak invocation context", () => {
  assert.throws(() => new Example({} as TokamakContext), { name: "TypeError" });
});

void test("raw binary calls preserve arbitrary bytes and view bounds", async () => {
  const native = connectNative();
  const bytes = new Uint8Array([0, 255, 128, 0]);
  const result = new Example(native.ctx).bytes(bytes.subarray(1, 3));
  assert.deepEqual(native.sent[0]?.payload, new Uint8Array([255, 128]));
  native.respondBytes(bytes);
  assert.deepEqual(await result, bytes);
});

void test("native call failures retain their name and message", async () => {
  const native = connectNative();
  const result = new Example(native.ctx).run();
  native.respond({ error: { name: "NeedsUIError", message: "Prompt needs UI" } });
  await assert.rejects(result, { name: "NeedsUIError", message: "Prompt needs UI" });
});

void test("value streams deliver values, finish and cancel their native subscription", async () => {
  const native = connectNative();
  const stream = new Example(native.ctx).watch();
  const reader = decodeValues<number>(stream);
  native.respond({ value: 1 }, false);
  assert.deepEqual(await reader.next(), { value: 1, done: false });
  await reader.return(undefined);
  assert.equal(native.sent.at(-1)?.type, "cancel");
});

void test("terminal stream failures reject the reader and release native work", async () => {
  const native = connectNative();
  const values = decodeValues(new Example(native.ctx).watch());
  native.respond({ error: { name: "NotAllowedError", message: "Denied" } });
  await assert.rejects(values.next(), { name: "NotAllowedError" });
});

void test("ordinary execution contexts do not grant native access", () => {
  // @ts-expect-error -- the native connection is required.
  assert.throws(() => new Example({ waitUntil() {} }), TypeError);
});

class Transfers extends Plugin {
  constructor(ctx: TokamakContext) { super(ctx, "transfers"); }
  upload(body: ReadableStream<Uint8Array>, signal?: AbortSignal): Promise<number> { return this.call("upload", { name: "a" }, { body, signal }); }
  samples(): ReadableStream<Uint8Array> { return this.stream("samples", null, "dropOldest"); }
}

void test("a call carries its body to the native endpoint", async () => {
  const native = connectNative();
  const body = new ReadableStream<Uint8Array>();
  const uploaded = new Transfers(native.ctx).upload(body);
  assert.equal(native.sent[0]?.body, body);
  native.respond({ value: 3 });
  assert.equal(await uploaded, 3);
});

void test("aborting a call cancels the native work and rejects with the reason", async () => {
  const native = connectNative();
  const controller = new AbortController();
  const uploaded = new Transfers(native.ctx).upload(new ReadableStream(), controller.signal);
  controller.abort(new DOMException("stopped", "AbortError"));
  await assert.rejects(uploaded, { name: "AbortError", message: "stopped" });
  assert.deepEqual(native.sent.at(-1), { type: "cancel", id: 1 });
  await assert.rejects(new Transfers(native.ctx).upload(new ReadableStream(), controller.signal), { message: "stopped" });
});

void test("a stream with a promised value subscribes once it resolves, and ends with its rejection", async () => {
  const native = connectNative();
  let resolve: (value: unknown) => void = () => undefined;
  const values = decodeValues<number>(new Example(native.ctx).watchAfter(new Promise((settle) => { resolve = settle; })));
  const first = values.next();
  await Promise.resolve();
  assert.deepEqual(native.sent, []);
  resolve({ every: 2 });
  await Promise.resolve();
  assert.deepEqual(native.lastRequest(), { type: "subscribe", plugin: "example", method: "watch", arguments: { every: 2 } });
  native.respond({ value: 1 }, false);
  assert.deepEqual(await first, { value: 1, done: false });

  const denied = decodeValues(new Example(native.ctx).watchAfter(Promise.reject(new DOMException("Denied", "NotAllowedError"))));
  await assert.rejects(denied.next(), { name: "NotAllowedError" });
});

void test("a stream passes its overflow policy to the native subscription", () => {
  const native = connectNative();
  void new Transfers(native.ctx).samples().getReader().read();
  assert.equal(native.sent[0]?.overflow, "dropOldest");
});

void test("capabilities report the operations implemented on the context's platform", () => {
  const availability = { transfers: { upload: ["ios", "android"], samples: ["ios"] } } as const;
  assert.deepEqual(new Transfers(connectNative("android", availability).ctx).capabilities, { upload: true, samples: false });
  assert.deepEqual(new Transfers(connectNative("ios", availability).ctx).capabilities, { upload: true, samples: true });
  assert.equal(connectNative("macos").ctx.platform, "macos");
});
