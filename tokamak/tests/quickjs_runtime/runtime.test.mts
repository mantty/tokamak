import assert from "node:assert/strict";
import test from "node:test";

test("ReadableStream pulls chunks and TransformStream pipes them", async () => {
  // Host-side smoke test for the vendored stream engine. Native extensions
  // are exercised by the compiled Workerd differential suite.
  const { ReadableStream, TransformStream, WritableStream } = await import("../../src/streams/web-standard.js");
  let next = 0;
  const source = new ReadableStream({
    pull(controller) {
      next += 1;
      if (next === 3) controller.close();
      else controller.enqueue(new Uint8Array([next]));
    },
  });
  const transform = new TransformStream({
    transform(chunk, controller) {
      controller.enqueue(new Uint8Array([chunk[0] + 10]));
    },
  });
  const received: number[] = [];
  const writable = new WritableStream({ write(value) { received.push(value[0]); } });
  await source.pipeThrough(transform).pipeTo(writable);
  assert.deepEqual(received, [11, 12]);
});

test("Readable emits end after pushed data", async () => {
  const { Readable } = await import("../../src/streams/node.js");
  const events: string[] = [];
  const stream = new Readable();
  stream.on("data", (value: unknown) => events.push(`data:${value}`));
  stream.on("end", () => events.push("end"));
  stream.push("value");
  stream.push(null);
  await new Promise(resolve => stream.once("end", resolve));
  assert.deepEqual(events, ["data:value", "end"]);

  const bufferedEvents: string[] = [];
  const buffered = new Readable();
  buffered.push("buffered");
  buffered.push(null);
  buffered.on("end", () => bufferedEvents.push("end"));
  buffered.on("data", (value: unknown) => bufferedEvents.push(`data:${value}`));
  await new Promise(resolve => buffered.once("end", resolve));
  assert.deepEqual(bufferedEvents, ["data:buffered", "end"]);

  const ended = new Readable();
  const lateValues: unknown[] = [];
  const failure = new Promise<NodeJS.ErrnoException>(resolve => ended.once("error", resolve));
  ended.on("data", (value: unknown) => lateValues.push(value));
  ended.push(null);
  assert.equal(ended.push("late"), false);
  assert.equal((await failure).code, "ERR_STREAM_PUSH_AFTER_EOF");
  assert.deepEqual(lateValues, []);
});
