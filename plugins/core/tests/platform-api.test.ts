import assert from "node:assert/strict";
import { test } from "node:test";
import { createNativeContext } from "../src/context.js";
import { dispatch } from "../src/registry.js";
import { decodeValues } from "../src/streams.js";
import { connectNative, setPlatform } from "../src/testing.js";
import { apple, onAsked, onEntered, onVisited } from "./fixtures/apple.generated.js";

// The fixture is what `tok plugin generate` writes for fixtures/description.json.

void test("a generated call sends its parameters by name and resolves to the result", async () => {
  const native = connectNative("macos");
  const added = apple(native.ctx).addRegion({ id: "home", radius: 50, tags: [], counts: {} });
  assert.deepEqual(native.lastRequest(), {
    type: "call",
    plugin: "example",
    method: "addRegion",
    arguments: { region: { id: "home", radius: 50, tags: [], counts: {} } },
  });
  native.respond({ value: null });
  await added;
});

void test("a generated API's calls are cancelled by its signal", async () => {
  const native = connectNative("ios");
  const controller = new AbortController();
  const regions = apple(native.ctx, controller.signal).regions();
  controller.abort(new DOMException("Timed out", "TimeoutError"));
  await assert.rejects(regions, { name: "TimeoutError" });
  assert.equal(native.sent.at(-1)?.type, "cancel");
});

void test("generated streams subscribe with their overflow and decode values", async () => {
  const native = connectNative("ios");
  const positions = decodeValues(apple(native.ctx).watchPosition(5));
  native.respond({ value: { id: "a" } }, false);
  assert.deepEqual(await positions.next(), { value: { id: "a" }, done: false });
  assert.deepEqual(native.sent[0]?.overflow, "dropOldest");
  assert.deepEqual(native.lastRequest().arguments, { every: 5 });
});

void test("native references, bodies and bytes keep their transport", async () => {
  const native = connectNative("ios");
  const api = apple(native.ctx);
  const capturing = api.capture(1);
  native.respond({ value: "frame-1" }, false);
  const frame = await capturing;

  const encoded = api.encode(frame);
  assert.deepEqual(native.lastRequest().arguments, { frame: "frame-1" });
  native.respond({ value: 7 });
  assert.equal(await encoded, 7);

  const file = new ReadableStream<Uint8Array>();
  const uploaded = api.upload("a.txt", file);
  assert.equal(native.sent.at(-1)?.body, file);
  assert.deepEqual(native.lastRequest().arguments, { name: "a.txt" });
  native.respond({ value: 3 });
  assert.equal(await uploaded, 3);

  const digest = api.digest(new Uint8Array([1, 2]));
  assert.deepEqual(native.sent.at(-1)?.payload, new Uint8Array([1, 2]));
  native.respondBytes(new TextEncoder().encode('"0102"'));
  assert.equal(await digest, "0102");

  const read = api.read("a.txt");
  assert.deepEqual(native.lastRequest().arguments, { path: "a.txt" });
  native.respondBytes(new Uint8Array([9]));
  assert.deepEqual(await read, new Uint8Array([9]));
});

void test("a generated API rejects off its family's platforms", async () => {
  const native = connectNative("android");
  await assert.rejects(apple(native.ctx).regions(), { name: "NotSupportedError" });
  assert.deepEqual(native.sent, []);
});

void test("generated listeners receive their family's events and reply", async () => {
  const ctx = createNativeContext({ waitUntil: () => undefined });
  const region = { id: "home", radius: 50, tags: [], counts: {} };
  const entered: unknown[] = [];
  setPlatform("macos");
  onEntered((event) => void entered.push(event));
  onAsked((event) => (event.id === "home" ? "precise" : undefined));
  onVisited(() => undefined);
  setPlatform(undefined);

  await dispatch("example.apple.entered", region, {}, ctx);
  assert.deepEqual(entered, [region]);
  const { reply, listened } = await dispatch("example.apple.asked", region, {}, ctx);
  assert.equal(reply, "precise");
  assert.ok(!listened.includes("example.apple.visited"), "visits are an iOS event");
});
