import assert from "node:assert/strict";
import { test } from "node:test";
import { capabilitiesOf, NativeEndpoint, platformApi, type TokamakContext } from "../src/index.js";
import { createNativeContext } from "../src/context.js";
import { decodeValues } from "../src/streams.js";
import { connectNative } from "../src/testing.js";

const example = (ctx: TokamakContext) => new NativeEndpoint(ctx, "example", ["ios", "macos"]);

void test("an endpoint uses the receiving invocation's connection and encodes arguments as JSON", async () => {
  const first = connectNative();
  const second = connectNative();
  const result = example(first.ctx).call("run", { fast: true });
  assert.deepEqual(first.lastRequest(), { type: "call", plugin: "example", method: "run", arguments: { fast: true } });
  assert.deepEqual(second.sent, []);
  first.respond({ value: "ran" });
  assert.equal(await result, "ran");
});

void test("an endpoint requires a Tokamak invocation context", () => {
  assert.throws(() => example({} as TokamakContext), { name: "TypeError" });
  // @ts-expect-error -- the native connection is required.
  assert.throws(() => example({ waitUntil() {} }), TypeError);
});

void test("raw binary calls preserve arbitrary bytes and view bounds", async () => {
  const native = connectNative();
  const bytes = new Uint8Array([0, 255, 128, 0]);
  const result = example(native.ctx).callBytes("bytes", bytes.subarray(1, 3));
  assert.deepEqual(native.sent[0]?.payload, new Uint8Array([255, 128]));
  native.respondBytes(bytes);
  assert.deepEqual(await result, bytes);
});

void test("native call failures retain their name and message", async () => {
  const native = connectNative();
  const result = example(native.ctx).call("run");
  native.respond({ error: { name: "NeedsUIError", message: "Prompt needs UI" } });
  await assert.rejects(result, { name: "NeedsUIError", message: "Prompt needs UI" });
});

void test("off its platforms an endpoint rejects without reaching the native connection", async () => {
  const native = connectNative("android");
  await assert.rejects(example(native.ctx).call("run"), { name: "NotSupportedError", message: "example.run is not supported on android" });
  await assert.rejects(decodeValues(example(native.ctx).stream("watch")).next(), { name: "NotSupportedError" });
  await assert.rejects(example(native.ctx).resource("frame"), { name: "NotSupportedError" });
  assert.deepEqual(native.sent, []);
});

void test("without a native host an endpoint says so", async () => {
  const unreachable = () => assert.fail("the endpoint reached the connection");
  const ctx = createNativeContext({ waitUntil: () => undefined }, { call: unreachable, listen: unreachable });
  await assert.rejects(example(ctx).call("run"), { name: "NotSupportedError", message: "example.run is not supported without a native host" });
});

void test("value streams deliver values, finish and cancel their native subscription", async () => {
  const native = connectNative();
  const reader = decodeValues<number>(example(native.ctx).stream("watch", { every: 1 }));
  native.respond({ value: 1 }, false);
  assert.deepEqual(await reader.next(), { value: 1, done: false });
  assert.deepEqual(native.lastRequest(), { type: "subscribe", plugin: "example", method: "watch", arguments: { every: 1 } });
  await reader.return(undefined);
  assert.equal(native.sent.at(-1)?.type, "cancel");
});

void test("a stream ends when its subscription ends without a value", async () => {
  const native = connectNative();
  const values = decodeValues<number>(example(native.ctx).stream("watch"));
  native.respond({ value: 1 }, false);
  native.respondBytes(new Uint8Array(), true);
  assert.deepEqual(await values.next(), { value: 1, done: false });
  assert.equal((await values.next()).done, true);
});

void test("terminal stream failures reject the reader", async () => {
  const native = connectNative();
  const values = decodeValues(example(native.ctx).stream("watch"));
  native.respond({ error: { name: "NotAllowedError", message: "Denied" } });
  await assert.rejects(values.next(), { name: "NotAllowedError" });
});

void test("a call carries its body to the native endpoint", async () => {
  const native = connectNative();
  const body = new ReadableStream<Uint8Array>();
  const uploaded = example(native.ctx).call("upload", { name: "a" }, body);
  assert.equal(native.sent[0]?.body, body);
  native.respond({ value: 3 });
  assert.equal(await uploaded, 3);
});

void test("aborting an endpoint's signal cancels its calls and rejects with the reason", async () => {
  const native = connectNative();
  const controller = new AbortController();
  const endpoint = new NativeEndpoint(native.ctx, "example", ["ios"], controller.signal);
  const uploaded = endpoint.call("upload");
  controller.abort(new DOMException("stopped", "AbortError"));
  await assert.rejects(uploaded, { name: "AbortError", message: "stopped" });
  assert.deepEqual(native.sent.at(-1), { type: "cancel", id: 1 });
  await assert.rejects(endpoint.call("upload"), { message: "stopped" });
});

void test("aborting an endpoint's signal ends its streams with the reason", async () => {
  const native = connectNative();
  const controller = new AbortController();
  const endpoint = new NativeEndpoint(native.ctx, "example", ["ios"], controller.signal);
  const values = decodeValues(endpoint.stream("watch"));
  controller.abort(new DOMException("stopped", "AbortError"));
  await assert.rejects(values.next(), { name: "AbortError", message: "stopped" });
  assert.deepEqual(native.sent.at(-1), { type: "cancel", id: 1 });
  await assert.rejects(decodeValues(endpoint.stream("watch")).next(), { message: "stopped" });
  assert.equal(native.sent.length, 2);
});

void test("aborting an endpoint's signal after a stream ends keeps its unread values", async () => {
  const native = connectNative();
  const controller = new AbortController();
  const values = decodeValues<number>(new NativeEndpoint(native.ctx, "example", ["ios"], controller.signal).stream("watch"));
  native.respond({ value: 1 }, false);
  native.respondBytes(new Uint8Array(), true);
  controller.abort();
  assert.deepEqual(await values.next(), { value: 1, done: false });
  assert.equal((await values.next()).done, true);
});

void test("a stream passes its overflow policy to the native subscription", () => {
  const native = connectNative();
  void example(native.ctx).stream("samples", {}, "dropOldest").getReader().read();
  assert.equal(native.sent[0]?.overflow, "dropOldest");
});

void test("platformApi creates the API of the running platform's family", async () => {
  const apis = {
    apple: (ctx: TokamakContext, signal?: AbortSignal) => ({ family: "apple", signal, run: () => new NativeEndpoint(ctx, "example", ["ios", "macos"]).call("run") }),
    android: (ctx: TokamakContext, signal?: AbortSignal) => ({ family: "android", signal, run: () => new NativeEndpoint(ctx, "example", ["android"]).call("run") }),
  };
  const { signal } = new AbortController();
  assert.equal(platformApi(connectNative("ios").ctx, apis, signal).signal, signal);
  assert.equal(platformApi(connectNative("macos").ctx, apis).family, "apple");
  assert.equal(platformApi(connectNative("ios").ctx, apis).family, "apple");
  assert.equal(platformApi(connectNative("android").ctx, apis).family, "android");
  const windows = connectNative("windows");
  await assert.rejects(platformApi(windows.ctx, apis).run(), { name: "NotSupportedError" });
  await assert.rejects(platformApi(connectNative("android").ctx, { apple: apis.apple }).run(), { name: "NotSupportedError" });
  assert.deepEqual(windows.sent, []);
});

void test("capabilities report the operations implemented on the context's platform", () => {
  const availability = { transfers: { upload: ["ios", "android"], samples: ["ios"] } } as const;
  assert.deepEqual(capabilitiesOf(connectNative("android", availability).ctx, "transfers"), { upload: true, samples: false });
  assert.deepEqual(capabilitiesOf(connectNative("ios", availability).ctx, "transfers"), { upload: true, samples: true });
  assert.equal(connectNative("macos").ctx.platform, "macos");
});
