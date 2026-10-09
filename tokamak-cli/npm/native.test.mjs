import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import WebSocket from "ws";

import { SESSION_HEADER, openDevSocket } from "./dist/dev-socket.mjs";
import { defineNativeCalls } from "./dist/native.mjs";

const TOKEN = "token";
let opened;

afterEach(async () => {
  await opened?.close();
  opened = undefined;
});

/**
 * A dev socket, with the development Worker's native calls defined on it, and the promises
 * `waitUntil` received.
 */
async function devSocket() {
  opened = await openDevSocket("app", TOKEN);
  const waited = [];
  defineNativeCalls(opened.port, TOKEN, (promise) => waited.push(promise));
  return { port: opened.port, waited };
}

/** A device on the dev socket at `port` that passes each message to `answer(message, send)`. */
async function connectDevice(port, answer) {
  const socket = new WebSocket(`ws://127.0.0.1:${port}`, { headers: { [SESSION_HEADER]: TOKEN } });
  const send = (message) => socket.send(JSON.stringify(message));
  socket.on("message", (data) => answer(JSON.parse(String(data)), send));
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

  const value = await globalThis.__tokamakNativeCall("location", "getCurrentPosition", { timeout: 1 });

  assert.deepEqual(value, { latitude: 51.5 });
  assert.deepEqual(requests, [
    { type: "call", id: requests[0].id, plugin: "location", method: "getCurrentPosition", arguments: { timeout: 1 } },
  ]);
});

test("rejects with the DOMException the plugin names", async () => {
  const { port } = await devSocket();
  await connectDevice(port, (message, send) =>
    send({ type: "result", id: message.id, result: { error: { name: "NeedsUIError", message: "Off screen" }, done: true } }),
  );

  await assert.rejects(
    globalThis.__tokamakNativeCall("notifications", "requestPermission", null),
    (error) => error instanceof DOMException && error.name === "NeedsUIError" && error.message === "Off screen",
  );
});

test("waits for the device to connect", async () => {
  const { port } = await devSocket();

  const value = globalThis.__tokamakNativeCall("tokamak", "lifecycleStage", null);
  await new Promise((resolve) => setTimeout(resolve, 100));
  await connectDevice(port, (message, send) =>
    send({ type: "result", id: message.id, result: { value: "background", done: true } }),
  );

  assert.equal(await value, "background");
});

test("streams subscription results to a listener until it is removed", async () => {
  const { port, waited } = await devSocket();
  const messages = [];
  let unsubscribed;
  const removed = new Promise((resolve) => (unsubscribed = resolve));
  await connectDevice(port, (message, send) => {
    messages.push(message.type);
    if (message.type === "unsubscribe") unsubscribed(message.id);
    if (message.type !== "subscribe") return;
    send({ type: "result", id: message.id, result: { value: 1, done: false } });
    send({ type: "result", id: message.id, result: { error: { name: "NotReadableError", message: "No fix" }, done: false } });
    send({ type: "result", id: message.id, result: { value: 2, done: false } });
  });
  const listener = collector(3);

  const stop = globalThis.__tokamakNativeListen("location", "watchPosition", null, listener.next, listener.error);
  await listener.ended;
  stop();
  await removed;
  await Promise.all(waited);

  assert.deepEqual(listener.received, [1, "NotReadableError: No fix", 2]);
  assert.deepEqual(messages, ["subscribe", "unsubscribe"]);
  assert.equal(waited.length, 1);
});

test("ends a listener at a done result", async () => {
  const { port, waited } = await devSocket();
  await connectDevice(port, (message, send) => {
    send({ type: "result", id: message.id, result: { error: { name: "NotSupportedError", message: "No" }, done: true } });
  });
  const listener = collector(1);

  globalThis.__tokamakNativeListen("location", "watch", null, listener.next, listener.error);
  await Promise.all(waited);

  assert.deepEqual(listener.received, ["NotSupportedError: No"]);
});

test("fails calls and listeners with NetworkError when the device disconnects", async () => {
  const { port, waited } = await devSocket();
  const device = await connectDevice(port, (message) => {
    if (message.type === "subscribe") device.close();
  });
  const listener = collector(1);

  const call = globalThis.__tokamakNativeCall("location", "getCurrentPosition", null);
  globalThis.__tokamakNativeListen("location", "watchPosition", null, listener.next, listener.error);
  await Promise.all(waited);

  await assert.rejects(call, (error) => error instanceof DOMException && error.name === "NetworkError");
  assert.deepEqual(listener.received, ["NetworkError: The device disconnected"]);
});

test("refuses plugin calls without the session token", async () => {
  const { port } = await devSocket();

  const response = await fetch(`http://127.0.0.1:${port}/call`, { method: "POST", body: "{}" });
  defineNativeCalls(port, "wrong", () => undefined);

  assert.equal(response.status, 401);
  await assert.rejects(
    globalThis.__tokamakNativeCall("location", "getCurrentPosition", null),
    (error) => error instanceof DOMException && error.name === "NetworkError" && /HTTP 401/.test(error.message),
  );
});

test("unsubscribes on the dev WebSocket that carried the subscription", async () => {
  const { port } = await devSocket();
  const first = [];
  const second = [];
  let subscribed;
  const subscription = new Promise((resolve) => (subscribed = resolve));
  let unsubscribed;
  const removed = new Promise((resolve) => (unsubscribed = resolve));
  await connectDevice(port, (message) => {
    first.push(message.type);
    if (message.type === "subscribe") subscribed();
    if (message.type === "unsubscribe") unsubscribed();
  });

  const stop = globalThis.__tokamakNativeListen("location", "watchPosition", null, () => undefined, () => undefined);
  await subscription;
  await connectDevice(port, (message) => second.push(message.type));
  stop();
  await removed;

  assert.deepEqual(first, ["subscribe", "unsubscribe"]);
  assert.deepEqual(second, []);
});
