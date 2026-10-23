import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "node:test";

import { defineEvent, type EventContext } from "../src/events.js";
import { dispatch, eventCodec } from "../src/registry.js";
import * as testing from "../src/testing.js";
import { createNativeContext } from "../src/context.js";

declare global {
  // eslint-disable-next-line @typescript-eslint/no-namespace -- test events merge into Tokamak.Events.
  namespace Tokamak {
    interface Events {
      "test.run": { value: number };
      "test.reply": null;
      "test.quiet": null;
      "test.fail": null;
      "test.listed": null;
      "test.also-listed": null;
      "test.testing": null;
      "test.typed": { count: number };
      "test.void": null;
      "test.binary": Uint8Array;
      "test.mobile": null;
      "test.anywhere": null;
    }
    interface EventReplies {
      "test.reply": string;
      "test.fail": string;
      "test.typed": { doubled: number };
      "test.binary": Uint8Array;
    }
  }
}

const env = {};
const ctx: EventContext = createNativeContext({ waitUntil: () => undefined });
let reported: unknown[] = [];

beforeEach(() => {
  reported = [];
  globalThis.reportError = (error: unknown) => reported.push(error);
});

afterEach(() => {
  Reflect.deleteProperty(globalThis, "reportError");
});

void test("runs every listener with the event, env and ctx", async () => {
  const onRun = defineEvent("test.run");
  const received: unknown[][] = [];
  onRun((...arguments_) => void received.push(arguments_));
  onRun(async (...arguments_) => {
    await Promise.resolve();
    received.push(arguments_);
  });

  await dispatch("test.run", { value: 1 }, env, ctx);

  assert.deepEqual(received, [
    [{ value: 1 }, env, ctx],
    [{ value: 1 }, env, ctx],
  ]);
});

void test("replies with the first value, in registration order, other than undefined", async () => {
  const onReply = defineEvent("test.reply");
  onReply(() => undefined);
  onReply(async () => {
    await new Promise((resolve) => setTimeout(resolve, 10));
    return "second";
  });
  onReply(() => "third");

  assert.equal((await dispatch("test.reply", null, env, ctx)).reply, "second");
});

void test("replies undefined when no listener returns a value", async () => {
  const onQuiet = defineEvent("test.quiet");
  onQuiet(() => undefined);

  assert.equal((await dispatch("test.quiet", null, env, ctx)).reply, undefined);
  assert.equal((await dispatch("test.unlistened", null, env, ctx)).reply, undefined);
});

void test("reports a throwing listener and still runs the others", async () => {
  const onFail = defineEvent("test.fail");
  const thrown = new Error("sync");
  const rejected = new Error("async");
  onFail(() => {
    throw thrown;
  });
  onFail(() => Promise.reject(rejected));
  onFail(() => "replied");

  const { reply } = await dispatch("test.fail", null, env, ctx);

  assert.equal(reply, "replied");
  assert.deepEqual(reported, [thrown, rejected]);
});

void test("reports the names of the events that have listeners", async () => {
  defineEvent("test.listed")(() => undefined);
  defineEvent("test.also-listed")(() => undefined);

  const { listened } = await dispatch("test.other", null, env, ctx);

  assert.ok(listened.includes("test.listed"));
  assert.ok(listened.includes("test.also-listed"));
  assert.ok(!listened.includes("test.other"));
});

void test("collects reported errors in the testing dispatch", async () => {
  Reflect.deleteProperty(globalThis, "reportError");
  const thrown = new Error("listener");
  defineEvent("test.testing")(() => {
    throw thrown;
  });

  const dispatched = await testing.dispatch("test.testing", null);

  assert.deepEqual(dispatched.reported, [thrown]);
  assert.equal(Reflect.get(globalThis, "reportError"), undefined);
});

void test("types listeners from defineEvent", () => {
  const onTyped = defineEvent("test.typed");

  onTyped((event) => ({ doubled: event.count * 2 }));
  onTyped((event, _env, context) => {
    context.waitUntil(Promise.resolve(event.count));
    return undefined;
  });
  // @ts-expect-error -- the event is `{ count: number }`.
  onTyped((event: { name: string }) => ({ doubled: event.name.length }));
  // @ts-expect-error -- the reply must be `{ doubled: number }`.
  onTyped(() => "wrong");
  // @ts-expect-error -- an event without a reply type takes no reply.
  defineEvent("test.void")(() => 1);
});

void test("events travel as JSON unless their definition gives a codec", () => {
  const json = eventCodec("test.quiet");
  assert.deepEqual(json.decode(new TextEncoder().encode('{"a":1}')), { a: 1 });
  assert.deepEqual(json.encode({ a: 1 }), new TextEncoder().encode('{"a":1}'));
  assert.deepEqual(json.encode(undefined), new Uint8Array(), "no reply is an empty payload");

  const bytes = { decode: (payload: Uint8Array) => payload, encode: (reply: Uint8Array) => reply };
  defineEvent("test.binary", { codec: bytes });
  assert.equal(eventCodec("test.binary"), bytes);
});

void test("registers listeners only on the platforms that emit the event", async () => {
  const onMobile = defineEvent("test.mobile", { platforms: ["ios", "android"] });
  const onAnywhere = defineEvent("test.anywhere");
  const received: string[] = [];
  for (const platform of [undefined, "macos", "android"] as const) {
    testing.setPlatform(platform);
    onMobile(() => void received.push(`mobile ${platform ?? "no host"}`));
    onAnywhere(() => void received.push(`anywhere ${platform ?? "no host"}`));
  }
  testing.setPlatform(undefined);

  await dispatch("test.mobile", null, env, ctx);
  await dispatch("test.anywhere", null, env, ctx);

  assert.deepEqual(received, ["mobile android", "anywhere no host", "anywhere macos", "anywhere android"]);
});
