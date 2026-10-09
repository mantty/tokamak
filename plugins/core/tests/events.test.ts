import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "node:test";

import { defineEvent, type EventContext } from "../src/events.js";
import { dispatch } from "../src/registry.js";
import * as testing from "../src/testing.js";

const env = {};
const ctx: EventContext = { waitUntil: () => undefined };
let reported: unknown[] = [];

beforeEach(() => {
  reported = [];
  globalThis.reportError = (error: unknown) => reported.push(error);
});

afterEach(() => {
  Reflect.deleteProperty(globalThis, "reportError");
});

void test("runs every listener with the event, env and ctx", async () => {
  const onRun = defineEvent<{ value: number }>("test.run");
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
  const onReply = defineEvent<null, string>("test.reply");
  onReply(() => undefined);
  onReply(async () => {
    await new Promise((resolve) => setTimeout(resolve, 10));
    return "second";
  });
  onReply(() => "third");

  assert.equal((await dispatch("test.reply", null, env, ctx)).reply, "second");
});

void test("replies undefined when no listener returns a value", async () => {
  const onQuiet = defineEvent<null>("test.quiet");
  onQuiet(() => undefined);

  assert.equal((await dispatch("test.quiet", null, env, ctx)).reply, undefined);
  assert.equal((await dispatch("test.unlistened", null, env, ctx)).reply, undefined);
});

void test("reports a throwing listener and still runs the others", async () => {
  const onFail = defineEvent<null, string>("test.fail");
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
  const onTyped = defineEvent<{ count: number }, { doubled: number }>("test.typed");

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
  defineEvent<null>("test.void")(() => 1);
});
