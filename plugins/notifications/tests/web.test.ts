import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "node:test";

import { notifications } from "../web/client.js";
import { decodeValues } from "@tokamakdev/plugin/streams";
import { DISMISSED_CACHE, OPENED, OPENED_CACHE, SETUP_CACHE } from "../web/protocol.js";
import {
  FakeCacheStorage,
  FakeRegistration,
  replaceGlobal,
} from "./fakes.js";

let restores: (() => void)[] = [];
let registration: FakeRegistration | undefined;
let messageListeners: ((event: { data: unknown }) => void)[] = [];
let permission: NotificationPermission = "granted";
const constructed: string[] = [];
const constructedOptions: Record<string, unknown>[] = [];
const badges: number[] = [];

class FakeNotification {
  static get permission() {
    return permission;
  }

  static requestPermission() {
    permission = "denied";
    return Promise.resolve(permission);
  }

  constructor(
    readonly title: string,
    options: Record<string, unknown> = {},
  ) {
    constructed.push(title);
    constructedOptions.push(options);
  }
}

beforeEach(() => {
  permission = "granted";
  registration = new FakeRegistration();
  messageListeners = [];
  constructed.length = 0;
  constructedOptions.length = 0;
  badges.length = 0;
  restores = [
    replaceGlobal("Notification", FakeNotification),
    replaceGlobal("caches", new FakeCacheStorage()),
    replaceGlobal("navigator", {
      setAppBadge: (count: number) => {
        badges.push(count);
        return Promise.resolve();
      },
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

const fullNotification = {
  id: "n1",
  title: "Ada",
  body: "Lunch?",
  data: { thread: 7 },
  category: "message",
  channel: "messages",
  group: "chats",
  badge: 3,
  silent: false,
  timestamp: 1_800_000_000_000,
  subtitle: "Team",
  interruptionLevel: "timeSensitive",
  relevanceScore: 0.5,
  subText: "Work",
  color: "#00FF00",
  style: {
    type: "messaging",
    user: "Me",
    conversationTitle: "Team",
    messages: [{ text: "Lunch?", at: 1_800_000_000_000, sender: "Ada" }],
  },
  progress: { current: 2, max: 5 },
  ongoing: true,
  autoCancel: false,
  onlyAlertOnce: true,
  timeoutAfter: 60_000,
  chronometer: { countDown: true },
  visibility: "private",
  publicVersion: { title: "New message", body: "Open to read" },
  sortKey: "a",
  androidCategory: "message",
  requireInteraction: true,
  renotify: true,
  vibrate: [200, 100, 200],
  dir: "ltr",
  lang: "en-GB",
  badgeImage: "/badge.png",
  image: "/photo.png",
  icon: "https://example.com/ada.png",
} as const;

void test("passes the web's options to the browser, and sets the app badge", async () => {
  await notifications.show(fullNotification);

  const options = registration?.shown.at(0)?.options ?? {};
  const keys = [
    "silent",
    "timestamp",
    "requireInteraction",
    "renotify",
    "vibrate",
    "dir",
    "lang",
    "badge",
    "image",
    "icon",
  ];
  assert.deepEqual(Object.fromEntries(keys.map((key) => [key, options[key]])), {
    silent: false,
    timestamp: 1_800_000_000_000,
    requireInteraction: true,
    renotify: true,
    vibrate: [200, 100, 200],
    dir: "ltr",
    lang: "en-GB",
    badge: "/badge.png",
    image: "/photo.png",
    icon: "https://example.com/ada.png",
  });
  assert.deepEqual(badges, [3]);
});

void test("shows nothing and stores nothing for an invalid notification or setup", async () => {
  await assert.rejects(notifications.show({ id: "n1", title: "Hi", badge: -1 }), TypeError);
  await assert.rejects(notifications.setup({ categories: [{ id: "", actions: [] }] }), TypeError);

  assert.deepEqual(registration?.shown, []);
  assert.deepEqual(await (await caches.open(SETUP_CACHE)).keys(), []);
});

void test("sets the app badge only for a notification with a badge, including 0", async () => {
  await notifications.show({ id: "n1", title: "Hi" });
  await notifications.show({ id: "n2", title: "Hi", badge: 0 });

  assert.deepEqual(badges, [0]);
});

void test("shows a badged notification where the browser has no badge or refuses it", async () => {
  delete (navigator as { setAppBadge?: unknown }).setAppBadge;
  await notifications.show({ id: "n1", title: "Hi", badge: 3 });
  (navigator as { setAppBadge?: unknown }).setAppBadge = () => Promise.reject(new DOMException("", "NotAllowedError"));
  await notifications.show({ id: "n2", title: "Hi", badge: 3 });

  assert.equal(registration?.shown.length, 2);
});

void test("passes the web's options to a notification shown without a service worker", async () => {
  registration = undefined;

  await notifications.show({ id: "n1", title: "Hi", requireInteraction: true, lang: "en-GB" });

  const options = constructedOptions.at(0) ?? {};
  assert.deepEqual([options.requireInteraction, options.lang], [true, "en-GB"]);
});

void test("shows a category's actions, with Chrome's text input for an input action", async () => {
  const result = await notifications.setup({
    categories: [
      {
        id: "message",
        actions: [
          { id: "reply", title: "Reply", input: { button: "Send", placeholder: "Message" }, icon: "/reply.png" },
          { id: "read", title: "Mark as read", icon: "/read.png" },
          { id: "note", title: "Note", input: {} },
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
    { action: "reply", title: "Reply", icon: "/reply.png", type: "text", placeholder: "Message" },
    { action: "read", title: "Mark as read", icon: "/read.png" },
    { action: "note", title: "Note", type: "text" },
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

void test("accepts every kind of setup value", async () => {
  const result = await notifications.setup({
    categories: [
      {
        id: "message",
        actions: [
          { id: "reply", title: "Reply", input: { button: "Send", placeholder: "Message" } },
          { id: "delete", title: "Delete", destructive: true, authenticationRequired: true, systemImage: "trash" },
          { id: "open", title: "Open", foreground: true, icon: "https://example.com/open.png" },
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
        lights: "#00FF00",
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

  await notifications.show({ id: "a", title: "Hi", category: "a" });
  await notifications.show({ id: "b", title: "Hi", category: "b" });

  assert.deepEqual(
    registration?.shown.map(({ actions }) => actions),
    [[], [{ action: "y", title: "Y" }]],
  );
});

void test("shows notifications without a service worker", async () => {
  registration = undefined;

  await notifications.show({ id: "download", title: "Done" });

  assert.deepEqual(constructed, ["Done"]);
});

void test("rejects showing notifications without permission", async () => {
  permission = "denied";
  await assert.rejects(notifications.show({ id: "a", title: "A" }), isDomException("NotAllowedError"));
});

void test("reports the permission as the settings, and sets the badge through the Badging API", async () => {
  assert.deepEqual(await notifications.settings(), { permission: "granted" });
  await notifications.setBadge(4);
  await notifications.setBadge(0);
  assert.deepEqual(badges, [4, 0]);

  delete (navigator as { setAppBadge?: unknown }).setAppBadge;
  await assert.rejects(notifications.setBadge(1), isDomException("NotSupportedError"));
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

void test("streams messages and subscription changes from the service worker", async () => {
  const messages = decodeValues(await notifications.onMessage());
  const changes = decodeValues(await notifications.onSubscriptionChange());
  const subscription = { service: "webpush", endpoint: "https://push.example/changed", keys: { auth: "a", p256dh: "b" } };
  const message = { id: "1", title: null, body: null, data: {} };
  for (const listener of messageListeners) {
    listener({ data: { type: "tokamak-notifications:message", message } });
    listener({ data: { type: "tokamak-notifications:subscription", subscription } });
  }
  assert.deepEqual((await messages.next()).value, message);
  assert.deepEqual((await changes.next()).value, subscription);
  await messages.return(undefined);
  assert.equal(messageListeners.length, 1);
  await changes.return(undefined);
  assert.equal(messageListeners.length, 0);
});

void test("streams dismissed notifications held by the service worker", async () => {
  const cache = await caches.open(DISMISSED_CACHE);
  const dismissed = { id: "1", title: "Hi", body: null, data: {}, source: "local" };
  await cache.put("/__tokamak/notifications/1", Response.json(dismissed));
  const values = decodeValues(await notifications.onNotificationDismissed());
  assert.deepEqual((await values.next()).value, dismissed);
  await values.return(undefined);
});

void test("delivers held notifications once to each active stream", async () => {
  const cache = await caches.open(OPENED_CACHE);
  const opened = { id: "1", title: "Hi", body: null, data: {}, source: "push" };
  const firstStream = await notifications.onNotificationOpened();
  const secondStream = await notifications.onNotificationOpened();
  const first = decodeValues(firstStream);
  const second = decodeValues(secondStream);
  await cache.put("/__tokamak/notifications/1", Response.json(opened));
  for (const listener of messageListeners) listener({ data: { type: OPENED } });
  assert.deepEqual((await first.next()).value, opened);
  assert.deepEqual((await second.next()).value, opened);
  await first.return(undefined);
  await cache.put("/__tokamak/notifications/2", Response.json({ ...opened, id: "2" }));
  for (const listener of messageListeners) listener({ data: { type: OPENED } });
  assert.deepEqual((await second.next()).value, { ...opened, id: "2" });
  await second.return(undefined);
  assert.deepEqual(await cache.keys(), []);
});
