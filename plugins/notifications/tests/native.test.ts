import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { connectNative, disconnectNative } from "@tokamakdev/plugin/testing";

import { notifications } from "../src/index.js";

afterEach(disconnectNative);

void test("calls the native plugin with each operation's arguments", async () => {
  const native = connectNative();
  const cases: [() => Promise<unknown>, string, unknown][] = [
    [() => notifications.permission(), "permission", null],
    [() => notifications.requestPermission(), "requestPermission", null],
    [
      () => notifications.show({ id: "download", title: "Done", data: { file: "a.pdf" } }),
      "show",
      { id: "download", title: "Done", data: { file: "a.pdf" } },
    ],
    [
      () => notifications.schedule({ id: "reminder", title: "Check in", at: 1_800_000_000_000 }),
      "schedule",
      { id: "reminder", title: "Check in", at: 1_800_000_000_000 },
    ],
    [() => notifications.getScheduled(), "getScheduled", null],
    [() => notifications.getDelivered(), "getDelivered", null],
    [() => notifications.remove("reminder"), "remove", { id: "reminder" }],
    [() => notifications.getSubscription(), "getSubscription", null],
    [() => notifications.unsubscribe(), "unsubscribe", null],
  ];

  for (const [operation, method, arguments_] of cases) {
    const result = operation();
    assert.deepEqual(native.lastRequest(), {
      type: "call",
      plugin: "notifications",
      method,
      arguments: arguments_,
    });
    native.respond({ value: "native result" });
    assert.equal(await result, "native result");
  }
});

void test("subscribes without showing push notifications in the foreground by default", async () => {
  const native = connectNative();
  const subscription = { service: "fcm", token: "token" };

  const subscribed = notifications.subscribe({ applicationServerKey: "web only" });
  assert.deepEqual(native.lastRequest().arguments, { showInForeground: false });
  native.respond({ value: subscription });

  assert.deepEqual(await subscribed, subscription);
  void notifications.subscribe({ showInForeground: true });
  assert.deepEqual(native.lastRequest().arguments, { showInForeground: true });
});

void test("delivers native events to listeners until they stop", () => {
  const native = connectNative();
  const opened: unknown[] = [];

  const stop = notifications.onNotificationOpened((notification) => opened.push(notification));
  assert.deepEqual(native.lastRequest(), {
    type: "subscribe",
    plugin: "notifications",
    method: "onNotificationOpened",
    arguments: null,
  });
  const subscription = native.sent.at(-1);
  native.respond({ value: { id: "1", source: "local" } }, false);
  stop();

  assert.deepEqual(opened, [{ id: "1", source: "local" }]);
  assert.deepEqual(native.sent.at(-1), {
    type: "cancel",
    session: subscription?.session,
    id: subscription?.id,
  });
});

void test("reports native listener failures", () => {
  const native = connectNative();
  const failures: string[] = [];

  notifications.onMessage(
    () => assert.fail("message received"),
    (error) => failures.push(error.name),
  );
  native.respond({ error: { name: "NotSupportedError", message: "unsupported" } });

  assert.deepEqual(failures, ["NotSupportedError"]);
});
