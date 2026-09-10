import assert from "node:assert/strict";
import test from "node:test";

test("ReadableStream pulls chunks and TransformStream pipes them", async () => {
  const { ReadableStream, TransformStream } = await import("../../src/streams/web.mjs");
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
  const received = [];
  const writable = {
    getWriter() {
      return {
        write(value) { received.push(value[0]); },
        close() {},
        abort() {},
        releaseLock() {},
      };
    },
  };
  await source.pipeTo(transform.writable);
  await transform.readable.pipeTo(writable);
  assert.deepEqual(received, [11, 12]);
});

test("Readable emits end after pushed data", async () => {
  const { Readable } = await import("../../src/streams/node.mjs?end-event");
  const events = [];
  const stream = new Readable();
  stream.on("data", (value) => events.push(`data:${value}`));
  stream.on("end", () => events.push("end"));
  stream.push("value");
  stream.push(null);
  assert.deepEqual(events, ["data:value", "end"]);

  const bufferedEvents = [];
  const buffered = new Readable();
  buffered.push("buffered");
  buffered.push(null);
  buffered.on("end", () => bufferedEvents.push("end"));
  buffered.on("data", (value) => bufferedEvents.push(`data:${value}`));
  assert.deepEqual(bufferedEvents, ["data:buffered", "end"]);

  const ended = new Readable();
  const lateValues = [];
  ended.on("data", (value) => lateValues.push(value));
  ended.push(null);
  assert.equal(ended.push("late"), false);
  assert.deepEqual(lateValues, []);
});

test("WebSocketPair delivers Worker messages to the native bridge", async () => {
  const { WebSocketPair } = await import("../../src/network/websocket.mjs");
  const pair = new WebSocketPair();
  const client = pair[0];
  const server = pair[1];
  server.accept();
  server.addEventListener("message", (event) => server.send(`pong ${event.data}`));

  server.__tokamak_receive("ping 42", false);

  assert.deepEqual(client.__tokamak_outbox, [{ type: "message", binary: false, data: "pong ping 42" }]);
});
