import assert from "node:assert/strict";
import { setImmediate } from "node:timers/promises";
import { test } from "node:test";

import type { NativePlatform } from "@tokamakdev/plugin";
import { decodeValues } from "@tokamakdev/plugin/streams";
import { connectNative, type Answer, type FakeNativeTransport, type Received } from "@tokamakdev/plugin/testing";

import { location, type Position } from "../src/index.js";

type Answers = Record<string, Answer | ((request: Received) => Answer | undefined)>;

const position: Position = {
  coords: { latitude: 52, longitude: 0.2, accuracy: 5, heading: 90 },
  timestamp: 1000,
};

/** A device on `platform` that answers the plugin's calls from `answers`, by method. */
function device(platform: NativePlatform, answers: Answers): FakeNativeTransport {
  const native = connectNative(platform);
  const common: Answers = {
    servicesEnabled: { value: true },
    permission: { value: { inUse: "granted", background: "prompt", precision: "precise" } },
  };
  native.answer((request) => {
    const answer = answers[request.method ?? ""] ?? common[request.method ?? ""];
    return typeof answer === "function" ? answer(request) : answer;
  });
  return native;
}

/** Waits for `condition`, failing after a thousand turns of the event loop. */
async function until(condition: () => boolean): Promise<void> {
  for (let turn = 0; !condition(); turn++) {
    if (turn === 1000) assert.fail("The plugin did not make the expected call");
    await setImmediate();
  }
}

const calls = (native: FakeNativeTransport, method: string) => native.sent.filter((request) => request.method === method);

void test("a recent native position answers without a new fix, even with a zero timeout", async () => {
  const asked: unknown[] = [];
  const native = device("ios", {
    recentPosition: (request) => {
      asked.push(request.arguments);
      return { value: position };
    },
  });
  assert.deepEqual(await location(native.ctx).getCurrentPosition({ maximumAge: 1000, timeout: 0 }), position);
  assert.deepEqual(await location(native.ctx).getCurrentPosition({ maximumAge: Infinity }), position);
  assert.deepEqual(asked, [{ maximumAge: 1000 }, { maximumAge: Number.MAX_VALUE }]);
  assert.equal(calls(native, "currentPosition").length, 0);
});

void test("without a recent position, the native endpoint takes a new fix", async () => {
  let asked: unknown;
  const native = device("android", {
    recentPosition: (request) => {
      asked = request.arguments;
      return { value: null };
    },
    currentPosition: { value: position },
  });
  assert.deepEqual(await location(native.ctx).getCurrentPosition(), position);
  assert.deepEqual(asked, { maximumAge: 0 });
});

void test("the timeout starts once a new fix is needed, not while recent positions are checked", async () => {
  const native = device("ios", { recentPosition: () => undefined, currentPosition: { value: position } });
  const located = location(native.ctx).getCurrentPosition({ timeout: 20 });
  await until(() => calls(native, "recentPosition").length === 1);
  await new Promise((resolve) => setTimeout(resolve, 40));
  const [recent] = calls(native, "recentPosition");
  native.deliver(recent.id, { value: null });
  assert.deepEqual(await located, position);
});

void test("location asks for in-use permission it lacks, and rejects without it", async () => {
  const native = device("ios", {
    permission: { value: { inUse: "prompt", background: "prompt", precision: "precise" } },
    requestPermission: { value: { inUse: "denied", background: "denied", precision: "precise" } },
  });
  await assert.rejects(location(native.ctx).getCurrentPosition(), { name: "NotAllowedError" });
  assert.equal(calls(native, "requestPermission").length, 1);
  assert.equal(calls(native, "recentPosition").length, 0);

  const off = device("android", { servicesEnabled: { value: false } });
  await assert.rejects(location(off.ctx).getCurrentPosition(), { name: "NotReadableError", message: "Location services are disabled" });
});

void test("no position in time rejects with TimeoutError and cancels the native request", async () => {
  const native = device("android", { recentPosition: { value: null }, currentPosition: () => undefined });
  await assert.rejects(location(native.ctx).getCurrentPosition({ timeout: 10 }), { name: "TimeoutError" });
  const [request] = calls(native, "currentPosition");
  assert.ok(native.sent.some(({ type, id }) => type === "cancel" && id === request.id));
});

void test("no position in the background reports missing background permission", async () => {
  const native = device("android", {
    recentPosition: { value: null },
    currentPosition: { value: null },
    lifecycleStage: { value: "background" },
  });
  await assert.rejects(location(native.ctx).getCurrentPosition(), { name: "NotAllowedError", message: /background/ });

  const foreground = device("android", { recentPosition: { value: null }, currentPosition: { value: null }, lifecycleStage: { value: "foreground" } });
  await assert.rejects(location(foreground.ctx).getCurrentPosition(), { name: "NotReadableError" });
});

void test("watching subscribes after the permission checks, keeps the newest for a slow reader, and ends on cancel", async () => {
  const native = device("macos", { watchPosition: () => undefined });
  const values = decodeValues<Position>(location(native.ctx).watchPosition());
  const first = values.next();
  await until(() => calls(native, "watchPosition").length === 1);
  const [subscription] = calls(native, "watchPosition");
  assert.ok(native.sent.findIndex(({ method }) => method === "permission") < native.sent.indexOf(subscription));
  assert.equal(subscription.overflow, "dropOldest");
  native.deliver(subscription.id, { value: position, done: false });
  assert.deepEqual((await first).value, position);
  await values.return(undefined);
  assert.ok(native.sent.some(({ type, id }) => type === "cancel" && id === subscription.id));
});

void test("watching without permission fails the stream without subscribing", async () => {
  const native = device("android", { permission: { value: { inUse: "denied", background: "denied", precision: "precise" } } });
  await assert.rejects(decodeValues(location(native.ctx).watchPosition()).next(), { name: "NotAllowedError" });
  assert.equal(calls(native, "watchPosition").length, 0);
});

void test("watching asks for permission the user has not given, then subscribes", async () => {
  const native = device("ios", {
    permission: { value: { inUse: "prompt", background: "prompt", precision: "precise" } },
    requestPermission: { value: { inUse: "granted", background: "prompt", precision: "precise" } },
    watchPosition: () => undefined,
  });
  const values = decodeValues<Position>(location(native.ctx).watchPosition());
  const first = values.next();
  await until(() => calls(native, "watchPosition").length === 1);
  const [asked] = calls(native, "requestPermission");
  const [subscription] = calls(native, "watchPosition");
  assert.ok(native.sent.indexOf(asked) < native.sent.indexOf(subscription));
  native.deliver(subscription.id, { value: position, done: false });
  assert.deepEqual((await first).value, position);
  await values.return(undefined);
});
