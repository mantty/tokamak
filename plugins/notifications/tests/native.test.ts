import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { notifications } from "../src/index.js";

type Message = Record<string, unknown>;

afterEach(() => {
  Reflect.deleteProperty(globalThis, "__tokamakNative");
  Reflect.deleteProperty(globalThis, "__tokamakReceive");
});

function connectNative(): Message[] {
  const sent: Message[] = [];
  globalThis.__tokamakNative = {
    onmessage: null,
    postMessage(message) {
      sent.push(JSON.parse(message) as Message);
    },
  };
  return sent;
}

function lastRequest(sent: Message[]): Message {
  const { type, plugin, method, arguments: arguments_ } = sent.at(-1) ?? {};
  return { type, plugin, method, arguments: arguments_ };
}

function respond(sent: Message[], value: unknown, done = true): void {
  const request = sent.at(-1);
  globalThis.__tokamakReceive?.({
    session: request?.session as string,
    id: request?.id as number,
    done,
    value,
  });
}

void test("calls the native plugin with each operation's arguments", async () => {
  const sent = connectNative();
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
    assert.deepEqual(lastRequest(sent), {
      type: "call",
      plugin: "notifications",
      method,
      arguments: arguments_,
    });
    respond(sent, "native result");
    assert.equal(await result, "native result");
  }
});

void test("subscribes without showing push notifications in the foreground by default", async () => {
  const sent = connectNative();
  const subscription = { service: "fcm", token: "token" };

  const subscribed = notifications.subscribe({ applicationServerKey: "web only" });
  assert.deepEqual(lastRequest(sent).arguments, { showInForeground: false });
  respond(sent, subscription);

  assert.deepEqual(await subscribed, subscription);
  void notifications.subscribe({ showInForeground: true });
  assert.deepEqual(lastRequest(sent).arguments, { showInForeground: true });
});

void test("delivers native events to listeners until they stop", () => {
  const sent = connectNative();
  const opened: unknown[] = [];

  const stop = notifications.onNotificationOpened((notification) => opened.push(notification));
  assert.deepEqual(lastRequest(sent), {
    type: "subscribe",
    plugin: "notifications",
    method: "onNotificationOpened",
    arguments: null,
  });
  const subscription = sent.at(-1);
  respond(sent, { id: "1", source: "local" }, false);
  stop();

  assert.deepEqual(opened, [{ id: "1", source: "local" }]);
  assert.deepEqual(sent.at(-1), {
    type: "cancel",
    session: subscription?.session,
    id: subscription?.id,
  });
});

void test("reports native listener failures", () => {
  const sent = connectNative();
  const failures: string[] = [];

  notifications.onMessage(
    () => assert.fail("message received"),
    (error) => failures.push(error.name),
  );
  const request = sent.at(-1);
  globalThis.__tokamakReceive?.({
    session: request?.session as string,
    id: request?.id as number,
    done: true,
    error: { name: "NotSupportedError", message: "unsupported" },
  });

  assert.deepEqual(failures, ["NotSupportedError"]);
});
