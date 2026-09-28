import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "node:test";

import {
  handleNotificationClick,
  handlePush,
  handleSubscriptionChange,
} from "../service-worker/index.js";
import { OPENED_CACHE, notificationData } from "../web/protocol.js";
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

function clicked(data: unknown) {
  return {
    tag: "n1",
    title: "Hi",
    body: "",
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

  const handled = await run((event) =>
    handleNotificationClick({ ...event, notification: notification as unknown as Notification }),
  );

  assert.equal(handled, true);
  assert.equal(notification.closed, true);
  assert.equal(page.focused, true);
  assert.deepEqual(page.posted, [{ type: "tokamak-notifications:opened" }]);
  const held = [...(cacheStorage.entries.get(OPENED_CACHE)?.values() ?? [])];
  assert.deepEqual(held.map((body) => JSON.parse(body) as unknown), [
    { id: "n1", title: "Hi", body: null, data: { n: 1 }, source: "local" },
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
  const handled = await run((event) =>
    handleNotificationClick({
      ...event,
      notification: clicked({ other: true }) as unknown as Notification,
    }),
  );

  assert.equal(handled, false);
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
