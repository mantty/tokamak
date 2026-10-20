let connection;
const nativeCall = async (plugin, method, value = null) => JSON.parse(new TextDecoder().decode(await connection.call(plugin, method, new TextEncoder().encode(JSON.stringify(value)))));
const nativeListen = (plugin, method, value, next, error) => connection.listen(plugin, method, new TextEncoder().encode(JSON.stringify(value)), { next: payload => next(JSON.parse(new TextDecoder().decode(payload))), error });
import { decodeDevice, encodeDevice } from "./fixtures/native-device.mjs";
import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import WebSocket from "ws";

import { SESSION_HEADER, openDevSocket } from "./dist/dev-socket.mjs";
import { connectDevice as relayConnection } from "./dist/native.mjs";

const TOKEN = "token";
let opened;

afterEach(async () => {
  await opened?.close();
  opened = undefined;
});

/**
 * A dev socket, with a development invocation's native connection through it, and the promises
 * its `waitUntil` received.
 */
async function devSocket() {
  opened = await openDevSocket("app", TOKEN);
  const waited = [];
  connection = relayConnection(opened.port, TOKEN, { waitUntil: (promise) => waited.push(promise) });
  return { port: opened.port, waited };
}

/** A device on the dev socket at `port` that passes each message to `answer(message, send)`. */
async function connectDevice(port, answer) {
  const socket = new WebSocket(`ws://127.0.0.1:${port}`, { headers: { [SESSION_HEADER]: TOKEN } });
  const send = (message) => socket.send(encodeDevice(message));
  socket.on("message", (data) => answer(decodeDevice(data), send));
  await new Promise((resolve, reject) => {
    socket.once("open", resolve);
    socket.once("error", reject);
  });
  return socket;
}

/** Collects what a listener receives, resolving `ended` once `count` items have arrived. */
function collector(count) {
  const received = [];
  let resolve;
  const ended = new Promise((done) => (resolve = done));
  const add = (item) => {
    received.push(item);
    if (received.length === count) resolve();
  };
  return { received, ended, next: (value) => add(value), error: (error) => add(`${error.name}: ${error.message}`) };
}

test("calls a plugin on the device and resolves with its value", async () => {
  const { port } = await devSocket();
  const requests = [];
  await connectDevice(port, (message, send) => {
    requests.push(message);
    send({ type: "result", id: message.id, result: { value: { latitude: 51.5 }, done: true } });
  });

  const value = await nativeCall("location", "getCurrentPosition", { timeout: 1 });

  assert.deepEqual(value, { latitude: 51.5 });
  assert.deepEqual(requests, [
    { type: "call", id: requests[0].id, plugin: "location", method: "getCurrentPosition", arguments: { timeout: 1 } },
  ]);
});

test("cancels an abandoned native call while allowing another call to complete", { timeout: 5000 }, async () => {
  const { port } = await devSocket();
  let started, cancelled;
  const called = new Promise((resolve) => (started = resolve));
  const stopped = new Promise((resolve) => (cancelled = resolve));
  await connectDevice(port, (message, send) => {
    if (message.type === "cancel") {
      cancelled(message.id);
      send({ type: "result", id: message.id, result: { value: "late", done: true } });
    } else if (message.method === "hold") {
      started(message.id);
    } else {
      send({ type: "result", id: message.id, result: { value: "retained", done: true } });
    }
  });
  const controller = new AbortController();
  const response = fetch(`http://127.0.0.1:${port}/call`, {
    method: "POST",
    headers: { [SESSION_HEADER]: TOKEN },
    body: encodeDevice({ type: "call", plugin: "test", method: "hold", arguments: null }),
    signal: controller.signal,
  });
  const failure = assert.rejects(response, { name: "AbortError" });
  const id = await called;
  controller.abort();
  await failure;
  assert.equal(await stopped, id);
  assert.equal(await nativeCall("test", "read", null), "retained");
});

test("rejects with the DOMException the plugin names", async () => {
  const { port } = await devSocket();
  await connectDevice(port, (message, send) =>
    send({ type: "result", id: message.id, result: { error: { name: "NeedsUIError", message: "Off screen" }, done: true } }),
  );

  await assert.rejects(
    nativeCall("notifications", "requestPermission", null),
    (error) => error instanceof DOMException && error.name === "NeedsUIError" && error.message === "Off screen",
  );
});

test("waits for the device to connect", async () => {
  const { port } = await devSocket();

  const value = nativeCall("tokamak", "lifecycleStage", null);
  await new Promise((resolve) => setTimeout(resolve, 100));
  await connectDevice(port, (message, send) =>
    send({ type: "result", id: message.id, result: { value: "background", done: true } }),
  );

  assert.equal(await value, "background");
});

test("streams subscription results to a listener until it is removed", async () => {
  const { port, waited } = await devSocket();
  const messages = [];
  let cancelled;
  const removed = new Promise((resolve) => (cancelled = resolve));
  await connectDevice(port, (message, send) => {
    messages.push(message.type);
    if (message.type === "cancel") cancelled(message.id);
    if (message.type !== "subscribe") return;
    send({ type: "result", id: message.id, result: { value: 1, done: false } });
    send({ type: "result", id: message.id, result: { error: { name: "NotReadableError", message: "No fix" }, done: false } });
    send({ type: "result", id: message.id, result: { value: 2, done: false } });
  });
  const listener = collector(3);

  const stop = nativeListen("location", "watchPosition", null, listener.next, listener.error);
  await listener.ended;
  stop();
  await removed;
  await Promise.all(waited);

  assert.deepEqual(listener.received, [1, "NotReadableError: No fix", 2]);
  assert.deepEqual(messages, ["subscribe", "cancel"]);
  assert.equal(waited.length, 1);
});

test("ends a listener at a done result", async () => {
  const { port, waited } = await devSocket();
  await connectDevice(port, (message, send) => {
    send({ type: "result", id: message.id, result: { error: { name: "NotSupportedError", message: "No" }, done: true } });
  });
  const listener = collector(1);

  nativeListen("location", "watch", null, listener.next, listener.error);
  await Promise.all(waited);

  assert.deepEqual(listener.received, ["NotSupportedError: No"]);
});

test("fails calls and listeners with NetworkError when the device disconnects", async () => {
  const { port, waited } = await devSocket();
  const device = await connectDevice(port, (message) => {
    if (message.type === "subscribe") device.close();
  });
  const listener = collector(1);

  const call = nativeCall("location", "getCurrentPosition", null);
  const failure = assert.rejects(call, (error) => error instanceof DOMException && error.name === "NetworkError");
  nativeListen("location", "watchPosition", null, listener.next, listener.error);
  await Promise.all(waited);

  await failure;
  assert.deepEqual(listener.received, ["NetworkError: The device disconnected"]);
});

test("refuses plugin calls without the session token", async () => {
  const { port } = await devSocket();

  const response = await fetch(`http://127.0.0.1:${port}/call`, { method: "POST", body: "{}" });
  connection = relayConnection(port, "wrong", { waitUntil: () => undefined });

  assert.equal(response.status, 401);
  await assert.rejects(
    nativeCall("location", "getCurrentPosition", null),
    (error) => error instanceof DOMException && error.name === "NetworkError" && /HTTP 401/.test(error.message),
  );
});

test("cancels on the dev WebSocket that carried the subscription", async () => {
  const { port } = await devSocket();
  const first = [];
  const second = [];
  let subscribed;
  const subscription = new Promise((resolve) => (subscribed = resolve));
  let cancelled;
  const removed = new Promise((resolve) => (cancelled = resolve));
  await connectDevice(port, (message) => {
    first.push(message.type);
    if (message.type === "subscribe") subscribed();
    if (message.type === "cancel") cancelled();
  });

  const stop = nativeListen("location", "watchPosition", null, () => undefined, () => undefined);
  await subscription;
  await connectDevice(port, (message) => second.push(message.type));
  stop();
  await removed;

  assert.deepEqual(first, ["subscribe", "cancel"]);
  assert.deepEqual(second, []);
});

test("byte endpoints preserve binary calls and streams including subarray offsets", { timeout: 5000 }, async () => {
  const { encodeMessage, decodeMessage } = await import("@tokamakdev/plugin/native-protocol");
  const { port, waited } = await devSocket();
  const device = new WebSocket(`ws://127.0.0.1:${port}`, { headers: { [SESSION_HEADER]: TOKEN } });
  await new Promise(resolve => device.once("open", resolve));
  const received = [];
  device.on("message", bytes => {
    const { header, payload } = decodeMessage(bytes);
    received.push(Array.from(payload));
    device.send(encodeMessage({ type: "result", id: header.id, done: true }, payload));
  });
  const native = connection;
  const input = new Uint8Array([99, 0, 255, 192, 175, 88]).subarray(1, 5);
  const expected = new Uint8Array(input);
  const pending = native.call("bytes", "echo", input);
  input.fill(42);
  assert.deepEqual(await pending, expected);
  const values = [];
  native.listen("bytes", "watch", expected, { next: (value, done) => values.push([Array.from(value), done]), error: assert.fail });
  await Promise.all(waited);
  assert.deepEqual(received, [[0, 255, 192, 175], [0, 255, 192, 175]]);
  assert.deepEqual(values, [[[0, 255, 192, 175], true]]);
});

test("listener exceptions are reported without becoming native failures", async () => {
  const { port, waited } = await devSocket();
  await connectDevice(port, (message, send) => send({ type: "result", id: message.id, result: { value: 1, done: true } }));
  const previous = globalThis.reportError;
  const reported = [];
  const failure = new Error("consumer failed");
  globalThis.reportError = error => reported.push(error);
  try {
    connection.listen("test", "watch", new Uint8Array(), { next: () => { throw failure; }, error: assert.fail });
    await Promise.all(waited);
    assert.deepEqual(reported, [failure]);
  } finally { globalThis.reportError = previous; }
});

test("malformed routing closes only the offending device connection", async () => {
  const { encodeMessage } = await import("@tokamakdev/plugin/native-protocol");
  const { port } = await devSocket();
  const broken = new WebSocket(`ws://127.0.0.1:${port}`, { headers: { [SESSION_HEADER]: TOKEN } });
  await new Promise(resolve => broken.once("open", resolve));
  const closed = new Promise(resolve => broken.once("close", resolve));
  broken.send(encodeMessage(null));
  assert.equal(await closed, 1002);
  await connectDevice(port, (message, send) => send({ type: "result", id: message.id, result: { value: "alive", done: true } }));
  assert.equal(await nativeCall("test", "read"), "alive");
});

test("error replies retain recoverable and terminal completion status", async () => {
  const { port, waited } = await devSocket();
  await connectDevice(port, (message, send) => {
    for (const done of [false, true]) send({ type: "result", id: message.id, result: { error: { name: "NotReadableError", message: "No fix" }, done } });
  });
  const failures = [];
  connection.listen("location", "watch", new Uint8Array(), { next: assert.fail, error: (error, done) => failures.push([error.name, done]) });
  await Promise.all(waited);
  assert.deepEqual(failures, [["NotReadableError", false], ["NotReadableError", true]]);
});

test("a call's body reaches the device one chunk per pull", { timeout: 5000 }, async () => {
  const { encodeMessage, decodeMessage } = await import("@tokamakdev/plugin/native-protocol");
  const { port } = await devSocket();
  const device = new WebSocket(`ws://127.0.0.1:${port}`, { headers: { [SESSION_HEADER]: TOKEN } });
  await new Promise(resolve => device.once("open", resolve));
  const received = [];
  device.on("message", bytes => {
    const { header, payload } = decodeMessage(bytes);
    received.push([header.type, Array.from(payload), header.done === true]);
    if (header.type === "chunk" && header.done) {
      device.send(encodeMessage({ type: "result", id: header.id, done: true }, new TextEncoder().encode("3")));
    } else device.send(encodeMessage({ type: "pull", id: header.id }));
  });
  const body = new ReadableStream({
    start(controller) {
      controller.enqueue(new Uint8Array([1, 2]));
      controller.enqueue(new Uint8Array([3]));
      controller.close();
    },
  });
  const reply = await connection.call("files", "write", new TextEncoder().encode("{}"), { body });
  assert.equal(new TextDecoder().decode(reply), "3");
  assert.deepEqual(received, [
    ["callWithBody", Array.from(new TextEncoder().encode("{}")), false],
    ["chunk", [1, 2], false],
    ["chunk", [3], false],
    ["chunk", [], true],
  ]);
});

test("aborting a call cancels it on the device and rejects with the reason", { timeout: 5000 }, async () => {
  const { port } = await devSocket();
  const types = [];
  let cancelled;
  const cancel = new Promise(resolve => (cancelled = resolve));
  await connectDevice(port, (message) => {
    types.push(message.type);
    if (message.type === "cancel") cancelled();
  });
  const controller = new AbortController();
  const pending = connection.call("test", "hang", new Uint8Array(), { signal: controller.signal });
  await new Promise(resolve => setTimeout(resolve, 100));
  controller.abort(new DOMException("stopped", "AbortError"));
  await assert.rejects(pending, { name: "AbortError", message: "stopped" });
  await cancel;
  assert.deepEqual(types, ["call", "cancel"]);
});

test("a subscription's overflow policy reaches the device", { timeout: 5000 }, async () => {
  const { port, waited } = await devSocket();
  let policy;
  await connectDevice(port, (message, send) => {
    policy = message.overflow;
    send({ type: "result", id: message.id, result: { value: 1, done: true } });
  });
  connection.listen("sensor", "watch", new Uint8Array(), { next: () => undefined, error: assert.fail, overflow: "dropOldest" });
  await Promise.all(waited);
  assert.equal(policy, "dropOldest");
});
