import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "node:test";

import { notifications, type SetupConfig } from "../src/index.js";
import { DISMISSED, DISMISSED_CACHE, OPENED, OPENED_CACHE } from "../web/protocol.js";
import {
  FakeCacheStorage,
  FakePushSubscription,
  FakeRegistration,
  replaceGlobal,
} from "./fakes.js";

let restores: (() => void)[] = [];
let registration: FakeRegistration | undefined;
let messageListeners: ((event: { data: unknown }) => void)[] = [];
let permission: NotificationPermission = "granted";
const constructed: string[] = [];

class FakeNotification {
  static get permission() {
    return permission;
  }

  static requestPermission() {
    permission = "denied";
    return Promise.resolve(permission);
  }

  constructor(readonly title: string) {
    constructed.push(title);
  }
}

beforeEach(() => {
  permission = "granted";
  registration = new FakeRegistration();
  messageListeners = [];
  constructed.length = 0;
  restores = [
    replaceGlobal("Notification", FakeNotification),
    replaceGlobal("caches", new FakeCacheStorage()),
    replaceGlobal("navigator", {
      serviceWorker: {
        getRegistration: () => Promise.resolve(registration),
        addEventListener: (_: string, listener: (event: { data: unknown }) => void) => {
          messageListeners.push(listener);
        },
        removeEventListener: (_: string, listener: (event: { data: unknown }) => void) => {
          messageListeners = messageListeners.filter((existing) => existing !== listener);
        },
      },
    }),
  ];
});

afterEach(() => {
  for (const restore of restores) restore();
});

function isDomException(name: string) {
  return (error: unknown) => error instanceof DOMException && error.name === name;
}

void test("reports and requests the browser's notification permission", async () => {
  permission = "default";
  assert.equal(await notifications.permission(), "prompt");

  assert.equal(await notifications.requestPermission(), "denied");
});

void test("shows notifications through the service worker, replacing by id", async () => {
  await notifications.show({ id: "download", title: "Done", body: "a.pdf", data: { n: 1 } });

  assert.deepEqual(registration?.shown.map(({ title, body, tag }) => ({ title, body, tag })), [
    { title: "Done", body: "a.pdf", tag: "download" },
  ]);
  assert.deepEqual(await notifications.getDelivered(), [
    { id: "download", title: "Done", body: "a.pdf", data: { n: 1 } },
  ]);
  await notifications.remove("download");
  assert.deepEqual(await notifications.getDelivered(), []);
});

void test("shows a category's actions, with Chrome's text input for an input action", async () => {
  const result = await notifications.setup({
    categories: [
      {
        id: "message",
        actions: [
          { id: "reply", title: "Reply", input: { button: "Send", placeholder: "Message" } },
          { id: "read", title: "Mark as read" },
        ],
      },
    ],
    channels: [{ id: "messages", name: "Messages" }],
  });

  await notifications.show({ id: "m1", title: "Hi", category: "message" });
  await notifications.show({ id: "m2", title: "Hi", category: "unknown" });

  const shown = registration?.shown ?? [];
  assert.deepEqual(result, { unapplied: [] });
  assert.deepEqual(shown.at(0)?.actions, [
    { action: "reply", title: "Reply", type: "text", placeholder: "Message" },
    { action: "read", title: "Mark as read" },
  ]);
  assert.deepEqual(shown.at(1)?.actions, []);
  assert.deepEqual(
    (await notifications.getDelivered()).map(({ id, category }) => ({ id, category })),
    [
      { id: "m1", category: "message" },
      { id: "m2", category: "unknown" },
    ],
  );
});

void test("rejects a setup with an invalid value before storing it", async () => {
  await assert.rejects(
    notifications.setup({ categories: [{ id: "", actions: [] }] }),
    (error: unknown) => error instanceof TypeError,
  );
  assert.deepEqual(await (await caches.open("tokamak-notifications-setup")).keys(), []);
});

void test("rejects each invalid setup value", async () => {
  const action = { id: "a", title: "A" };
  const channel = { id: "c", name: "C" };
  const configs: unknown[] = [
    null,
    { categories: [{ id: "", actions: [] }] },
    { categories: [{ id: "a" }] },
    { categories: [{ id: "a", actions: [{ id: "a" }] }] },
    { categories: [{ id: "a", actions: [action, action] }] },
    { categories: [{ id: "a", actions: [] }, { id: "a", actions: [] }] },
    { categories: [{ id: "a", actions: [{ ...action, foreground: "yes" }] }] },
    { showInForeground: "yes" },
    { showInForeground: { banner: 1 } },
    { channelGroups: [{ id: "g", name: "G" }, { id: "g", name: "G" }] },
    { channels: [channel, channel] },
    { channels: [{ id: "c" }] },
    { channels: [{ ...channel, group: "missing" }] },
    { channels: [{ ...channel, id: "tokamak.notifications" }] },
    { channels: [{ ...channel, importance: "urgent" }] },
    { channels: [{ ...channel, vibration: [] }] },
    { channels: [{ ...channel, vibration: [-1] }] },
    { channels: [{ ...channel, vibration: [100, 0.5] }] },
    // Android reads eight digits as alpha first, CSS as alpha last.
    { channels: [{ ...channel, lights: "#00ff0080" }] },
    { channels: [{ ...channel, lights: true }] },
    { defaultChannel: { description: "No name" } },
  ];
  for (const config of configs) {
    await assert.rejects(
      notifications.setup(config as SetupConfig),
      (error: unknown) => error instanceof TypeError,
      JSON.stringify(config),
    );
  }
});

void test("accepts every kind of setup value", async () => {
  const result = await notifications.setup({
    categories: [
      {
        id: "message",
        actions: [
          { id: "reply", title: "Reply", input: { button: "Send", placeholder: "Message" } },
          { id: "delete", title: "Delete", destructive: true, authenticationRequired: true, systemImage: "trash" },
          { id: "open", title: "Open", foreground: true },
        ],
        hiddenPreviewsBodyPlaceholder: "New message",
        hiddenPreviewsShowTitle: true,
        hiddenPreviewsShowSubtitle: false,
        summaryFormat: "%u more messages",
        customDismissAction: true,
        allowInCarPlay: true,
      },
    ],
    showInForeground: { banner: true, list: true, sound: false, badge: true },
    defaultChannel: { name: "General", importance: "default" },
    channelGroups: [{ id: "chats", name: "Chats", description: "Conversations" }],
    channels: [
      {
        id: "messages",
        name: "Messages",
        description: "New messages",
        group: "chats",
        importance: "high",
        sound: "default",
        vibration: [0, 250, 250, 250],
        lights: "#00ff00",
        showBadge: true,
        bypassDnd: false,
      },
      { id: "quiet", name: "Quiet", importance: "min", sound: "none", vibration: false, lights: false },
    ],
  });

  assert.deepEqual(result, { unapplied: [] });
});

void test("replaces the previous setup's categories", async () => {
  await notifications.setup({ categories: [{ id: "a", actions: [{ id: "x", title: "X" }] }] });
  await notifications.setup({ categories: [{ id: "b", actions: [{ id: "y", title: "Y" }] }] });

  await notifications.show({ id: "n", title: "Hi", category: "a" });

  assert.deepEqual(registration?.shown.at(0)?.actions, []);
});

void test("shows notifications without a service worker", async () => {
  registration = undefined;

  await notifications.show({ id: "download", title: "Done" });

  assert.deepEqual(constructed, ["Done"]);
});

void test("rejects showing and scheduling what the web cannot", async () => {
  permission = "denied";
  await assert.rejects(notifications.show({ id: "a", title: "A" }), isDomException("NotAllowedError"));
  await assert.rejects(
    notifications.schedule({ id: "a", title: "A", at: 0 }),
    isDomException("NotSupportedError"),
  );
  assert.deepEqual(await notifications.getScheduled(), []);
});

void test("subscribes with the application server key and reuses the subscription", async () => {
  const subscription = await notifications.subscribe({ applicationServerKey: "BEl6" });

  assert.deepEqual(registration?.subscribeOptions, [
    { userVisibleOnly: true, applicationServerKey: "BEl6" },
  ]);
  assert.deepEqual(subscription, {
    service: "webpush",
    endpoint: "https://push.example/new",
    keys: { p256dh: "https://push.example/new-p256dh", auth: "auth" },
  });
  assert.deepEqual(await notifications.subscribe(), subscription);
  assert.deepEqual(await notifications.getSubscription(), subscription);
  await notifications.unsubscribe();
  assert.equal(registration.subscription?.unsubscribed, true);
});

void test("rejects subscribing without what Web Push requires", async () => {
  await assert.rejects(notifications.subscribe(), isDomException("InvalidStateError"));
  registration = undefined;
  await assert.rejects(
    notifications.subscribe({ applicationServerKey: "BEl6" }),
    isDomException("InvalidStateError"),
  );
  permission = "default";
  await assert.rejects(
    notifications.subscribe({ applicationServerKey: "BEl6" }),
    isDomException("NotAllowedError"),
  );
});

void test("delivers messages and subscription changes from the service worker", () => {
  const messages: unknown[] = [];
  const changes: unknown[] = [];
  const stopMessages = notifications.onMessage((message) => messages.push(message));
  notifications.onSubscriptionChange((subscription) => changes.push(subscription));
  const subscription = new FakePushSubscription("https://push.example/changed");
  const message = { id: "1", title: null, body: null, data: {} };

  for (const listener of messageListeners) {
    listener({ data: { type: "tokamak-notifications:message", message } });
    listener({ data: { type: "tokamak-notifications:subscription", subscription } });
  }
  stopMessages();

  assert.deepEqual(messages, [message]);
  assert.deepEqual(changes, [subscription]);
  assert.equal(messageListeners.length, 1);
});

void test("takes each dismissed notification the service worker holds", async () => {
  const cache = await caches.open(DISMISSED_CACHE);
  const dismissed = { id: "1", title: "Hi", body: null, data: {}, source: "local" };
  const received: unknown[] = [];

  notifications.onNotificationDismissed((notification) => received.push(notification));
  await new Promise((resolve) => setTimeout(resolve, 10));
  await cache.put("/__tokamak/notifications/1", Response.json(dismissed));
  for (const listener of messageListeners) listener({ data: { type: DISMISSED } });
  await new Promise((resolve) => setTimeout(resolve, 10));

  assert.deepEqual(received, [dismissed]);
});

void test("takes each opened notification the service worker holds once", async () => {
  const cache = await caches.open(OPENED_CACHE);
  const opened = { id: "1", title: "Hi", body: null, data: {}, source: "push" };
  await cache.put("/__tokamak/notifications/opened/1", Response.json(opened));
  const first: unknown[] = [];
  const second: unknown[] = [];

  const stop = notifications.onNotificationOpened((notification) => first.push(notification));
  notifications.onNotificationOpened((notification) => second.push(notification));
  await new Promise((resolve) => setTimeout(resolve, 10));
  await cache.put("/__tokamak/notifications/opened/2", Response.json({ ...opened, id: "2" }));
  stop();
  for (const listener of messageListeners) listener({ data: { type: OPENED } });
  await new Promise((resolve) => setTimeout(resolve, 10));

  assert.deepEqual(first.map((notification) => (notification as { id: string }).id), ["1"]);
  assert.deepEqual(second.map((notification) => (notification as { id: string }).id), ["1", "2"]);
});
