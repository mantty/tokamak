import assert from "node:assert/strict";
import { test } from "node:test";

import { defineEvent } from "../src/events.js";
import type { NativeConnection } from "../src/context.js";
import { createNativeContext, nativeConnection, TokamakEntrypoint, type TokamakEvent } from "../src/worker.js";

declare global {
  // eslint-disable-next-line @typescript-eslint/no-namespace -- test events merge into Tokamak.Events.
  namespace Tokamak {
    interface Events {
      "worker.reply": string;
    }
    interface EventReplies {
      "worker.reply": string;
    }
  }
}

void test("entrypoint listeners receive the invocation context and receiving bindings", async () => {
  const tasks: Promise<unknown>[] = [];
  const ctx = createNativeContext({ waitUntil: (task) => tasks.push(task) });
  const env = { label: "receiving worker" };
  const task = Promise.resolve();
  defineEvent("worker.reply")((value, bindings, context) => {
    assert.equal(bindings, env);
    assert.equal(context, ctx);
    context.waitUntil(task);
    return value.toUpperCase();
  });
  const worker = new TokamakEntrypoint(ctx, env);
  assert.equal(await worker.event({ type: "worker.reply", data: "payload" }), "PAYLOAD");
  assert.deepEqual(tasks, [task]);
});

void test("a native context uses the connection the host attached to the execution context", async () => {
  const ordinary = { waitUntil: () => undefined };
  // @ts-expect-error -- ordinary execution contexts do not contain a native connection.
  new TokamakEntrypoint(ordinary, {});
  const offline = createNativeContext({ waitUntil: () => undefined });
  await assert.rejects(offline[nativeConnection].call("example", "run", new Uint8Array()), { name: "NotSupportedError" });
  const connection: NativeConnection = {
    call: () => Promise.resolve(new Uint8Array([42])),
    listen: () => () => undefined,
  };
  Reflect.set(globalThis, nativeConnection, connection);
  try {
    const unattached = createNativeContext({ waitUntil: () => undefined });
    await assert.rejects(unattached[nativeConnection].call("example", "run", new Uint8Array()), { name: "NotSupportedError" });
  } finally {
    Reflect.deleteProperty(globalThis, nativeConnection);
  }
  const attached = createNativeContext({ waitUntil: () => undefined, [nativeConnection]: connection });
  assert.equal(attached[nativeConnection], connection);
  assert.deepEqual(await attached[nativeConnection].call("example", "run", new Uint8Array()), new Uint8Array([42]));
});

void test("an event override receives typed events and can reply synchronously", () => {
  class Native extends TokamakEntrypoint<{ label: string }> {
    override event(event: TokamakEvent): string {
      if (event.type === "start") return `${this.env.label}:${String(event.data.foreground)}`;
      return `${this.env.label}:${event.type}`;
    }
  }
  const worker = new Native(createNativeContext({ waitUntil: () => undefined }), { label: "device" });
  assert.equal(worker.event({ type: "start", data: { foreground: true } }), "device:true");
  assert.equal(worker.event({ type: "resume", data: {} }), "device:resume");
  // @ts-expect-error -- events have declared types.
  worker.event({ type: "push", data: {} });
});
