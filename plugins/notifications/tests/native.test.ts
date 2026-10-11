import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { connectNative, disconnectNative } from "@tokamakdev/plugin/testing";

import { notifications, type Subscription } from "../src/index.js";

afterEach(disconnectNative);

const setupConfig = {
  categories: [
    {
      id: "message",
      actions: [
        { id: "reply", title: "Reply", input: { button: "Send", placeholder: "Message" } },
        { id: "open", title: "Open", foreground: true },
      ],
      customDismissAction: true,
    },
  ],
  showInForeground: { banner: true, list: true },
  defaultChannel: { name: "General" },
  channelGroups: [{ id: "chats", name: "Chats" }],
  channels: [{ id: "messages", name: "Messages", group: "chats", importance: "high" as const }],
};

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
    [() => notifications.setup(setupConfig), "setup", setupConfig],
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

void test("rejects an invalid setup without calling the native plugin", async () => {
  const native = connectNative();

  const setup = notifications.setup({ channels: [{ id: "c", name: "C", group: "missing" }] });

  assert.deepEqual(native.sent, []);
  await assert.rejects(setup, (error: unknown) => error instanceof TypeError);
});

void test("subscribes without the web's options", async () => {
  const native = connectNative();
  const subscription: Subscription = { service: "fcm", fid: "fid" };

  const subscribed = notifications.subscribe({ applicationServerKey: "web only" });
  assert.equal(native.lastRequest().arguments, null);
  native.respond({ value: subscription });

  assert.deepEqual(await subscribed, subscription);
});

void test("listens for dismissed notifications", () => {
  const native = connectNative();
  const dismissed: unknown[] = [];

  notifications.onNotificationDismissed((notification) => dismissed.push(notification.id));
  assert.equal(native.lastRequest().method, "onNotificationDismissed");
  native.respond({ value: { id: "1", title: "Hi", body: null, data: {}, source: "local" } }, false);

  assert.deepEqual(dismissed, ["1"]);
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
