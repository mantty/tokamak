import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "node:test";

import {
  handleNotificationClick,
  handleNotificationClose,
  handlePush,
  handleSubscriptionChange,
} from "../service-worker/index.js";
import { DISMISSED_CACHE, OPENED_CACHE, notificationData, storeSetup } from "../web/protocol.js";
import {
  FakeCacheStorage,
  FakePushSubscription,
  FakeRegistration,
  replaceGlobal,
} from "./fakes.js";

interface Client {
  readonly url: string;
  readonly posted: unknown[];
  focused: boolean;
}

let restores: (() => void)[] = [];
let registration: FakeRegistration;
let cacheStorage: FakeCacheStorage;
let windows: Client[];
let opened: string[];

function client(url: string): Client & {
  focus(): Promise<Client>;
  postMessage(message: unknown): void;
} {
  const window = {
    url,
    posted: [] as unknown[],
    focused: false,
    focus() {
      window.focused = true;
      return Promise.resolve(window);
    },
    postMessage(message: unknown) {
      window.posted.push(message);
    },
  };
  return window;
}

beforeEach(() => {
  registration = new FakeRegistration();
  cacheStorage = new FakeCacheStorage();
  windows = [];
  opened = [];
  restores = [
    replaceGlobal("registration", registration),
    replaceGlobal("caches", cacheStorage),
    replaceGlobal("clients", {
      matchAll: () => Promise.resolve(windows),
      openWindow: (url: string) => {
        opened.push(url);
        return Promise.resolve(null);
      },
    }),
  ];
});

afterEach(() => {
  for (const restore of restores) restore();
});

async function run(handler: (event: { waitUntil(promise: Promise<unknown>): void }) => unknown) {
  const pending: Promise<unknown>[] = [];
  const result = handler({ waitUntil: (promise) => pending.push(promise) });
  await Promise.all(pending);
  return result;
}

void test("shows a pushed notification and forwards the message to open pages", async () => {
  const page = client("https://app.example/");
  windows = [page];
  const payload = { id: "m1", title: "Hi", body: "There", data: { url: "/inbox" } };

  await run((event) => {
    handlePush({ ...event, data: { text: () => JSON.stringify(payload) } });
  });

  assert.deepEqual(registration.shown.map(({ title, body, tag }) => ({ title, body, tag })), [
    { title: "Hi", body: "There", tag: "m1" },
  ]);
  assert.deepEqual(registration.shown[0]?.data, notificationData("push", { url: "/inbox" }));
  assert.deepEqual(page.posted, [{ type: "tokamak-notifications:message", message: payload }]);
});

void test("shows a pushed notification with its category's actions", async () => {
  await storeSetup({ categories: [{ id: "message", actions: [{ id: "read", title: "Read" }] }] });
  const payload = { id: "m1", title: "Hi", category: "message" };

  await run((event) => {
    handlePush({ ...event, data: { text: () => JSON.stringify(payload) } });
  });

  assert.deepEqual(registration.shown[0]?.actions, [{ action: "read", title: "Read" }]);
  assert.deepEqual(registration.shown[0]?.data, notificationData("push", {}, "message"));
});

void test("shows a pushed notification with the web's options, and sets the app badge", async () => {
  const badges: number[] = [];
  restores.push(
    replaceGlobal("navigator", {
      setAppBadge: (count: number) => {
        badges.push(count);
        return Promise.resolve();
      },
    }),
  );
  // The invalid `renotify` and the `vibrate` a silent notification cannot have are dropped.
  const payload = { id: "m1", title: "Hi", silent: true, requireInteraction: true, badge: 2, renotify: "yes", vibrate: [100] };

  await run((event) => {
    handlePush({ ...event, data: { text: () => JSON.stringify(payload) } });
  });

  const options = registration.shown[0]?.options ?? {};
  assert.deepEqual(
    [options.silent, options.requireInteraction, options.renotify, options.vibrate],
    [true, true, undefined, undefined],
  );
  assert.deepEqual(badges, [2]);
});

void test("keeps vibrate on a pushed notification that is not silent", async () => {
  await run((event) => {
    handlePush({ ...event, data: { text: () => JSON.stringify({ id: "m1", title: "Hi", vibrate: [100] }) } });
  });

  assert.deepEqual(registration.shown[0]?.options.vibrate, [100]);
});

void test("normalises a push payload with missing fields", async () => {
  const page = client("https://app.example/");
  windows = [page];

  await run((event) => {
    handlePush({ ...event, data: { text: () => JSON.stringify({ title: 3, data: [1] }) } });
  });

  const posted = page.posted[0] as { message: { id: string; title: null; body: null; data: object } };
  assert.equal(typeof posted.message.id, "string");
  assert.deepEqual({ ...posted.message, id: "" }, { id: "", title: null, body: null, data: {} });
});

void test("shows a push payload that is not JSON as the body", async () => {
  await run((event) => {
    handlePush({ ...event, data: { text: () => "Plain text" } });
  });

  assert.deepEqual(registration.shown.map(({ title, body }) => ({ title, body })), [
    { title: "", body: "Plain text" },
  ]);
});

function clicked(data: unknown, body = "") {
  return {
    tag: "n1",
    title: "Hi",
    body,
    data,
    closed: false,
    close() {
      this.closed = true;
    },
  };
}

void test("holds an opened notification and focuses an open page", async () => {
  const page = client("https://app.example/");
  windows = [page];
  const notification = clicked(notificationData("local", { n: 1 }));

  // Browsers pass an empty action for a click on the notification itself.
  const handled = await run((event) =>
    handleNotificationClick({
      ...event,
      notification: notification as unknown as Notification,
      action: "",
      reply: null,
    }),
  );

  assert.equal(handled, true);
  assert.equal(notification.closed, true);
  assert.equal(page.focused, true);
  assert.deepEqual(page.posted, [{ type: "tokamak-notifications:opened" }]);
  const held = [...(cacheStorage.entries.get(OPENED_CACHE)?.values() ?? [])];
  assert.deepEqual(held.map((body) => JSON.parse(body) as unknown), [
    { id: "n1", title: "Hi", body: null, data: { n: 1 }, source: "local", action: null, input: null },
  ]);
});

void test("reports an action and its typed text, opening the app only for a foreground action", async () => {
  await storeSetup({
    categories: [
      {
        id: "message",
        actions: [
          { id: "reply", title: "Reply", input: {} },
          { id: "open", title: "Open", foreground: true },
        ],
      },
    ],
  });
  const page = client("https://app.example/");
  windows = [page];

  await run((event) =>
    handleNotificationClick({
      ...event,
      notification: clicked(notificationData("local", {}, "message")) as unknown as Notification,
      action: "reply",
      reply: "On my way",
    }),
  );
  assert.equal(page.focused, false);
  assert.deepEqual(page.posted, [{ type: "tokamak-notifications:opened" }]);
  await run((event) =>
    handleNotificationClick({
      ...event,
      notification: clicked(notificationData("local", {}, "message")) as unknown as Notification,
      action: "open",
    }),
  );

  assert.equal(page.focused, true);
  const held = [...(cacheStorage.entries.get(OPENED_CACHE)?.values() ?? [])];
  assert.deepEqual(
    held.map((body) => JSON.parse(body) as { action: string; input: string | null }),
    [
      { id: "n1", title: "Hi", body: null, data: {}, source: "local", action: "reply", input: "On my way" },
      { id: "n1", title: "Hi", body: null, data: {}, source: "local", action: "open", input: null },
    ],
  );
});

void test("holds a dismissed notification for the page", async () => {
  const page = client("https://app.example/");
  windows = [page];

  const handled = await run((event) =>
    handleNotificationClose({
      ...event,
      notification: clicked(notificationData("push", { n: 1 }), "Lunch?") as unknown as Notification,
    }),
  );

  assert.equal(handled, true);
  assert.deepEqual(page.posted, [{ type: "tokamak-notifications:dismissed" }]);
  const held = [...(cacheStorage.entries.get(DISMISSED_CACHE)?.values() ?? [])];
  assert.deepEqual(held.map((body) => JSON.parse(body) as unknown), [
    { id: "n1", title: "Hi", body: "Lunch?", data: { n: 1 }, source: "push" },
  ]);
});

void test("opens the notification's url when no page is open", async () => {
  const notification = clicked(notificationData("push", { url: "/inbox" }));

  await run((event) =>
    handleNotificationClick({ ...event, notification: notification as unknown as Notification }),
  );

  assert.deepEqual(opened, ["/inbox"]);
});

void test("leaves notifications the plugin did not show to the app", async () => {
  const notification = clicked({ other: true }) as unknown as Notification;
  const handled = await run((event) => handleNotificationClick({ ...event, notification }));
  const closed = await run((event) => handleNotificationClose({ ...event, notification }));

  assert.equal(handled, false);
  assert.equal(closed, false);
  assert.equal(cacheStorage.entries.size, 0);
});

void test("resubscribes and forwards a changed subscription", async () => {
  const page = client("https://app.example/");
  windows = [page];

  await run((event) => {
    handleSubscriptionChange({
      ...event,
      oldSubscription: new FakePushSubscription("https://push.example/old") as unknown as PushSubscription,
    });
  });

  assert.equal(registration.subscribeOptions.length, 1);
  assert.deepEqual(page.posted, [
    {
      type: "tokamak-notifications:subscription",
      subscription: {
        service: "webpush",
        endpoint: "https://push.example/new",
        keys: { p256dh: "https://push.example/new-p256dh", auth: "auth" },
      },
    },
  ]);
});
