import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { defineEvent } from "../src/events.js";
import type { NativeConnection } from "../src/context.js";
import { createNativeContext, nativeConnection, TokamakEntrypoint, type TokamakEvent } from "../src/worker.js";

afterEach(() => {
  Reflect.deleteProperty(globalThis, nativeConnection);
});

void test("entrypoint listeners receive the invocation context and receiving bindings", async () => {
  const tasks: Promise<unknown>[] = [];
  const ctx = createNativeContext({ waitUntil: (task) => tasks.push(task) });
  const env = { label: "receiving worker" };
  const task = Promise.resolve();
  defineEvent<string, string>("worker.reply")((value, bindings, context) => {
    assert.equal(bindings, env);
    assert.equal(context, ctx);
    context.waitUntil(task);
    return value.toUpperCase();
  });
  const worker = new TokamakEntrypoint(ctx, env);
  assert.equal(await worker.event({ type: "worker.reply", data: "payload" }), "PAYLOAD");
  assert.deepEqual(tasks, [task]);
});

void test("native contexts carry the host connection while ordinary contexts cannot supply one", async () => {
  const ordinary = { waitUntil: () => undefined };
  // @ts-expect-error -- ordinary execution contexts do not contain a native connection.
  new TokamakEntrypoint(ordinary, {});
  const offline = createNativeContext(ordinary);
  await assert.rejects(offline[nativeConnection].call("example", "run", new Uint8Array()), { name: "NotSupportedError" });
  const connection: NativeConnection = {
    call: () => Promise.resolve(new Uint8Array([42])),
    listen: () => () => undefined,
  };
  Reflect.set(globalThis, nativeConnection, connection);
  const ctx = createNativeContext({ waitUntil: () => undefined });
  assert.equal(ctx[nativeConnection], connection);
  assert.deepEqual(await ctx[nativeConnection].call("example", "run", new Uint8Array()), new Uint8Array([42]));
  await assert.rejects(offline[nativeConnection].call("example", "run", new Uint8Array()), { name: "NotSupportedError" });
});

void test("an event override can return a reply synchronously", () => {
  class Native extends TokamakEntrypoint<{ label: string }> {
    override event(event: TokamakEvent): string {
      return `${this.env.label}:${event.type}`;
    }
  }
  const worker = new Native(createNativeContext({ waitUntil: () => undefined }), { label: "device" });
  assert.equal(worker.event({ type: "push", data: {} }), "device:push");
});
