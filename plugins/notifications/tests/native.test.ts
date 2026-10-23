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

/** An operation, and the method and arguments of the request it sends. */
type Call = [() => Promise<unknown>, string, unknown];

/** Runs each operation, checking the request it sends and that it resolves to the native reply. */
async function assertCalls(native: ReturnType<typeof connectNative>, cases: readonly Call[]): Promise<void> {
  for (const [operation, method, arguments_] of cases) {
    const result = operation();
    assert.deepEqual(native.lastRequest(), { type: "call", plugin: "notifications", method, arguments: arguments_ });
    native.respond({ value: "native result" });
    assert.equal(await result, "native result");
  }
}

void test("calls the native plugin with each operation's arguments", async () => {
  const native = connectNative();
  const notifications = createApi(native.ctx);
  const hourly = { id: "hourly", title: "Stretch", every: { hours: 1 } };
  const monday = { id: "monday", title: "Plan", on: { weekday: 2, hour: 9 }, repeats: true };
  const download = { id: "download", title: "Done", data: { file: "a.pdf" } };
  const reminder = { id: "reminder", title: "Check in", at: 1_800_000_000_000 };
  const cases: Call[] = [
    [() => notifications.permission(), "permission", {}],
    [() => notifications.requestPermission(), "requestPermission", {}],
    [
      () => notifications.requestPermission({ provisional: true, criticalAlert: true }),
      "requestPermission",
      { options: { provisional: true, criticalAlert: true } },
    ],
    [() => notifications.settings(), "settings", {}],
    [() => notifications.setBadge(3), "setBadge", { count: 3 }],
    [() => notifications.openSettings(), "openSettings", {}],
    // A scheduled notification's trigger travels apart from its content.
    [
      () => notifications.schedule(hourly),
      "schedule",
      { notification: { id: "hourly", title: "Stretch" }, trigger: { every: { hours: 1 } } },
    ],
    [
      () => notifications.schedule(monday),
      "schedule",
      { notification: { id: "monday", title: "Plan" }, trigger: { on: { weekday: 2, hour: 9 }, repeats: true } },
    ],
    [() => notifications.show(download), "show", { notification: download }],
    [
      () => notifications.schedule(reminder),
      "schedule",
      { notification: { id: "reminder", title: "Check in" }, trigger: { at: 1_800_000_000_000 } },
    ],
    [() => notifications.getDelivered(), "getDelivered", {}],
    [() => notifications.remove("reminder"), "remove", { id: "reminder" }],
    [() => notifications.subscribe(), "subscribeToPush", {}],
    [() => notifications.getSubscription(), "getSubscription", {}],
    [() => notifications.unsubscribe(), "unsubscribe", {}],
    [() => notifications.setup(setupConfig), "setup", { config: setupConfig }],
  ];

  await assertCalls(native, cases);
});

void test("returns each scheduled notification with its trigger's fields", async () => {
  const native = connectNative();
  const scheduled = createApi(native.ctx).getScheduled();
  assert.equal(native.lastRequest().method, "getScheduled");
  native.respond({ value: [{ content: { id: "reminder", title: "Check in" }, trigger: { at: 1 } }] });
  assert.deepEqual(await scheduled, [{ id: "reminder", title: "Check in", at: 1 }]);
});

void test("schedules region triggers on Apple platforms only", async () => {
  const region = { latitude: 51.5, longitude: -0.1, radius: 100 };
  const home = { id: "home", title: "Home", region, repeats: true };
  const native = connectNative("ios");
  const scheduled = createApi(native.ctx).schedule(home);
  assert.deepEqual(native.lastRequest().arguments, {
    notification: { id: "home", title: "Home" },
    trigger: { region, repeats: true },
  });
  native.respond({ value: null });
  await scheduled;
  await assert.rejects(createApi(connectNative("android").ctx).schedule(home), { name: "NotSupportedError" });
});

void test("calls Android's own methods on Android", async () => {
  const native = connectNative("android");
  const notifications = createApi(native.ctx);
  const cases: Call[] = [
    [() => notifications.openSettings({ channel: "messages" }), "openSettings", { options: { channel: "messages" } }],
    [() => notifications.subscribeToTopic("news"), "subscribeToTopic", { topic: "news" }],
    [() => notifications.unsubscribeFromTopic("news"), "unsubscribeFromTopic", { topic: "news" }],
  ];
  await assertCalls(native, cases);
  await assert.rejects(notifications.setBadge(1), { name: "NotSupportedError" });
});

void test("reports which operations the platform implements", () => {
  const availability = { notifications: { setBadge: ["ios", "macos"], subscribeToTopic: ["android"] } } as const;
  const { capabilities } = createApi(connectNative("android", availability).ctx);
  assert.deepEqual(capabilities, { setBadge: false, subscribeToTopic: true });
});

void test("rejects Android's topics on Apple platforms", async () => {
  const notifications = createApi(connectNative("ios").ctx);
  await assert.rejects(notifications.subscribeToTopic("news"), { name: "NotSupportedError" });
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
  assert.deepEqual(native.lastRequest().arguments, { notification });
  native.respond({ value: null });
  await shown;
});

void test("subscribes to push without the Web Push options", async () => {
  const native = connectNative("android");
  const notifications = createApi(native.ctx);
  const subscription: Subscription = { service: "fcm", fid: "fid" };

  const subscribed = notifications.subscribe({ applicationServerKey: "web only" });
  assert.deepEqual(native.lastRequest(), { type: "call", plugin: "notifications", method: "subscribeToPush", arguments: {} });
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
