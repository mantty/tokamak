import assert from "node:assert/strict";
import { setImmediate } from "node:timers/promises";
import { test } from "node:test";
import { Bindings, NativeObject, NativeReference, applicationContext, referenceOf, release, type Classes, type TokamakContext } from "../src/bindings.js";
import { connectNative, type FakeNativeTransport } from "../src/testing.js";

interface Point { x: number; y: number }

const bindings = new Bindings("kit/apple", () => classes);

class KTWidget extends NativeObject {
  static init(ctx: TokamakContext): Promise<KTWidget> { return bindings.call(ctx, "KTWidget.init()", []); }
  title(): Promise<string> { return bindings.call(this, "KTWidget.title", []); }
  move(to: Point, at: Date, data: Uint8Array, near: KTWidget | null): Promise<unknown> {
    return bindings.call(this, "KTWidget.move(to:at:data:near:)", [to, at, data, near]);
  }
  readings(): ReadableStream<Point> { return bindings.stream(this, "KTWidget.readings", []); }
}
class KTFancyWidget extends KTWidget {}

interface KTDelegateMethods { widgetDidMove?(widget: KTWidget, point: Point): void | Promise<void> }
class KTDelegate extends NativeObject {
  static implement(ctx: TokamakContext, handlers: KTDelegateMethods): Promise<KTDelegate> {
    return bindings.implement(ctx, KTDelegate, "KTDelegate", ["widgetDidMove"], handlers);
  }
}

const classes: Classes = { KTWidget, KTFancyWidget, KTDelegate };

/** Answers the subscription the invocation's first binding call opens for its scope. */
async function openScope(native: FakeNativeTransport): Promise<void> {
  await setImmediate();
  assert.deepEqual(native.lastRequest(), { type: "subscribe", plugin: "tokamak", method: "resource.scope", arguments: null });
  native.respond({ value: "scope-1" }, false);
  await setImmediate();
}

async function widget(native: FakeNativeTransport, types = ["KTWidget", "NSObject"]): Promise<KTWidget> {
  const pending = KTWidget.init(native.ctx);
  await openScope(native);
  native.respond({ value: { "#": "object", id: "w1", types } });
  return pending;
}

void test("calls run in the invocation's one scope and wrap objects in their most specific bound class", async () => {
  const native = connectNative();
  const pending = KTWidget.init(native.ctx);
  await openScope(native);
  assert.deepEqual(native.lastRequest(), { type: "call", plugin: "kit/apple", method: "KTWidget.init()", arguments: { scope: "scope-1", arguments: [] } });
  native.respond({ value: { "#": "object", id: "w1", types: ["KTFancyWidget", "KTWidget", "NSObject"] } });
  const fancy = await pending;
  assert.ok(fancy instanceof KTFancyWidget);

  const title = fancy.title();
  await setImmediate();
  assert.deepEqual(native.lastRequest(), { type: "call", plugin: "kit/apple", method: "KTWidget.title", arguments: { scope: "scope-1", target: { "#": "object", id: "w1" }, arguments: [] } });
  native.respond({ value: "Dial" });
  assert.equal(await title, "Dial");
  assert.equal(native.sent.filter((request) => request.type === "subscribe").length, 1);
});

void test("values JSON lacks travel tagged, and records may use the tag's key", async () => {
  const native = connectNative();
  const dial = await widget(native);
  const moved = dial.move({ x: 1, y: 2 }, new Date(5), new Uint8Array([1, 2]), dial);
  await setImmediate();
  assert.deepEqual((native.lastRequest().arguments as { arguments: unknown }).arguments, [
    { x: 1, y: 2 },
    { "#": "date", value: 5 },
    { "#": "bytes", value: "AQI=" },
    { "#": "object", id: "w1" },
  ]);
  native.respond({ value: { "#": "record", value: { "#": "tag", at: { "#": "date", value: 9 }, speed: { "#": "number", value: "NaN" } } } });
  assert.deepEqual(await moved, { "#": "tag", at: new Date(9), speed: NaN });
});

void test("platform errors reject with their details as the cause", async () => {
  const native = connectNative();
  const dial = await widget(native);
  const title = dial.title();
  await setImmediate();
  native.respond({ error: { name: "OperationError", message: "Denied", cause: { domain: "kCLErrorDomain", code: 1 } } });
  await assert.rejects(title, (error: DOMException) => {
    assert.equal(error.name, "OperationError");
    assert.deepEqual(error.cause, { domain: "kCLErrorDomain", code: 1 });
    return true;
  });
});

void test("disposing any view of an object releases it natively once", async () => {
  const native = connectNative();
  const dial = await widget(native);
  const view = new KTFancyWidget(referenceOf(dial));
  release(view);
  release(dial);
  const releases = native.sent.filter((request) => request.method === "resource.release");
  assert.equal(releases.length, 1);
  assert.deepEqual(native.lastRequest(), { type: "call", plugin: "tokamak", method: "resource.release", arguments: "w1" });
  await assert.rejects(dial.title(), { name: "InvalidStateError" });
});

void test("an implementation calls the handlers it has with decoded arguments until it is disposed", async () => {
  const native = connectNative();
  const moves: [KTWidget, Point][] = [];
  const pending = KTDelegate.implement(native.ctx, { widgetDidMove(moved, point) { moves.push([moved, point]); } });
  await openScope(native);
  assert.deepEqual(native.lastRequest(), { type: "subscribe", plugin: "kit/apple", method: "KTDelegate", arguments: { scope: "scope-1", methods: ["widgetDidMove"] } });
  native.respond({ value: "d1" }, false);
  const delegate = await pending;
  assert.ok(delegate instanceof KTDelegate);
  native.respond({ value: { method: "widgetDidMove", arguments: [{ "#": "object", id: "w2", types: ["KTWidget"] }, { x: 3, y: 4 }] } }, false);
  assert.equal(moves.length, 1);
  assert.ok(moves[0]?.[0] instanceof KTWidget);
  assert.deepEqual(moves[0]?.[1], { x: 3, y: 4 });
  release(delegate);
  assert.equal(native.sent.at(-1)?.type, "cancel");
});

void test("a stream decodes elements as they arrive and ends with the sequence", async () => {
  const native = connectNative();
  const dial = await widget(native);
  const reader = dial.readings().getReader();
  const first = reader.read();
  await setImmediate();
  assert.deepEqual(native.lastRequest(), { type: "subscribe", plugin: "kit/apple", method: "KTWidget.readings", arguments: { scope: "scope-1", target: { "#": "object", id: "w1" }, arguments: [] } });
  native.respond({ value: { x: 1, y: 1 } }, false);
  assert.deepEqual(await first, { value: { x: 1, y: 1 }, done: false });
  native.respondBytes(new Uint8Array(), true);
  assert.deepEqual(await reader.read(), { value: undefined, done: true });
});

void test("an overload is the first whose parameters take the arguments", async () => {
  const dial = await widget(connectNative());
  const overloads = [
    ["start(String)", [["string"]]],
    ["start(int, Widget)", [["number"], [KTWidget, "null"]]],
    ["start(Object)", [["any"]]],
  ] as const;
  assert.equal(bindings.overload(["fast"], overloads), "start(String)");
  assert.equal(bindings.overload([1, dial], overloads), "start(int, Widget)");
  assert.equal(bindings.overload([1, null], overloads), "start(int, Widget)");
  assert.equal(bindings.overload([[1]], overloads), "start(Object)");
  assert.throws(() => bindings.overload([1, "dial"], overloads), TypeError);
});

void test("app objects are retained in the invocation's scope", async () => {
  const native = connectNative("android");
  const context = applicationContext(native.ctx);
  await openScope(native);
  assert.deepEqual(native.lastRequest(), { type: "call", plugin: "tokamak", method: "resource.root", arguments: { scope: "scope-1", name: "context" } });
  native.respond({ value: "app" });
  assert.ok(await context instanceof NativeReference);
});
