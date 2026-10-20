import { decodeValues } from "@tokamakdev/plugin/streams";
import assert from "node:assert/strict";
import { test } from "node:test";

import { connectNative } from "@tokamakdev/plugin/testing";

import { notifications as createApi, type Subscription } from "../src/index.js";


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
  const notifications = createApi(native.ctx);
  const cases: [() => Promise<unknown>, string, unknown][] = [
    [() => notifications.permission(), "permission", null],
    [() => notifications.requestPermission(), "requestPermission", {}],
    [
      () => notifications.requestPermission({ provisional: true, criticalAlert: true }),
      "requestPermission",
      { provisional: true, criticalAlert: true },
    ],
    [() => notifications.settings(), "settings", null],
    [() => notifications.setBadge(3), "setBadge", { count: 3 }],
    [() => notifications.openSettings(), "openSettings", {}],
    [() => notifications.openSettings({ channel: "messages" }), "openSettings", { channel: "messages" }],
    [() => notifications.subscribeToTopic("news"), "subscribeToTopic", { topic: "news" }],
    [() => notifications.unsubscribeFromTopic("news"), "unsubscribeFromTopic", { topic: "news" }],
    [
      () => notifications.schedule({ id: "hourly", title: "Stretch", every: { hours: 1 } }),
      "schedule",
      { id: "hourly", title: "Stretch", every: { hours: 1 } },
    ],
    [
      () => notifications.schedule({ id: "monday", title: "Plan", on: { weekday: 2, hour: 9 }, repeats: true }),
      "schedule",
      { id: "monday", title: "Plan", on: { weekday: 2, hour: 9 }, repeats: true },
    ],
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

void test("passes every notification field to the native plugin", async () => {
  const native = connectNative();
  const notifications = createApi(native.ctx);
  const notification = {
    id: "n1",
    title: "Ada",
    group: "chats",
    badge: 3,
    silent: true,
    style: { type: "inbox", lines: ["Lunch?", "At one?"], summary: "2 messages" },
    progress: { indeterminate: true },
    chronometer: true,
  } as const;

  const shown = notifications.show(notification);
  assert.deepEqual(native.lastRequest().arguments, notification);
  native.respond({ value: null });
  await shown;
});

void test("passes subscription options to the native endpoint", async () => {
  const native = connectNative();
  const notifications = createApi(native.ctx);
  const subscription: Subscription = { service: "fcm", fid: "fid" };

  const subscribed = notifications.subscribe({ applicationServerKey: "web only" });
  assert.deepEqual(native.lastRequest().arguments, { applicationServerKey: "web only" });
  native.respond({ value: subscription });

  assert.deepEqual(await subscribed, subscription);
});

void test("streams dismissed notifications", async () => {
  const native = connectNative();
  const values = decodeValues<{ id: string }>(createApi(native.ctx).onNotificationDismissed());
  assert.equal(native.lastRequest().method, "onNotificationDismissed");
  native.respond({ value: { id: "1", title: "Hi", data: {}, source: "local" } }, true);
  assert.equal((await values.next()).value?.id, "1");
  assert.equal((await values.next()).done, true);
});

void test("streams opened notifications until cancelled", async () => {
  const native = connectNative();
  const values = decodeValues(createApi(native.ctx).onNotificationOpened());
  assert.equal(native.lastRequest().method, "onNotificationOpened");
  native.respond({ value: { id: "1", source: "local" } }, false);
  assert.deepEqual((await values.next()).value, { id: "1", source: "local" });
  await values.return(undefined);
  assert.equal(native.sent.at(-1)?.type, "cancel");
});

void test("native message failures reject the stream reader", async () => {
  const native = connectNative();
  const values = decodeValues(createApi(native.ctx).onMessage());
  native.respond({ error: { name: "NotSupportedError", message: "unsupported" } });
  await assert.rejects(values.next(), { name: "NotSupportedError" });
});
