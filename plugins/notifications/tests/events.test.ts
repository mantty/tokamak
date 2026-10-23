import assert from "node:assert/strict";
import { test } from "node:test";

import { dispatch, setPlatform } from "@tokamakdev/plugin/testing";

import { onNotificationDismissed, onNotificationOpened, onPush } from "../src/events.js";

void test("replies to each family's push message with the notification its platform's listener returns", async () => {
  const message = { id: "m1", data: { itemId: "42" } };
  const received: unknown[] = [];
  for (const platform of ["ios", "android"] as const) {
    setPlatform(platform);
    onPush((pushed) => {
      received.push(pushed);
      assert.equal(typeof pushed.data.itemId, "string");
      return { id: `${platform}-${pushed.data.itemId as string}`, title: "New item" };
    });
  }
  setPlatform(undefined);

  const apple = await dispatch("notifications.apple.push", message);
  const android = await dispatch("notifications.android.push", message);

  assert.deepEqual(received, [message, message]);
  assert.deepEqual(apple.reply, { id: "ios-42", title: "New item" });
  assert.deepEqual(android.reply, { id: "android-42", title: "New item" });
});

void test("delivers each family's opened actions and dismissed notifications", async () => {
  const notification = { id: "n1", title: "Hi", data: {}, source: "local" as const };
  const received: unknown[] = [];
  for (const platform of ["ios", "android"] as const) {
    setPlatform(platform);
    onNotificationOpened(({ action, input }) => void received.push({ platform, action, input }));
    onNotificationDismissed(({ id }) => void received.push({ platform, id }));
  }
  setPlatform(undefined);

  for (const family of ["apple", "android"]) {
    await dispatch(`notifications.${family}.opened`, { ...notification, action: "reply", input: "On my way" });
    await dispatch(`notifications.${family}.dismissed`, notification);
  }

  assert.deepEqual(received, [
    { platform: "ios", action: "reply", input: "On my way" },
    { platform: "ios", id: "n1" },
    { platform: "android", action: "reply", input: "On my way" },
    { platform: "android", id: "n1" },
  ]);
});

void test("types the listeners", () => {
  onPush(async (message) => {
    await Promise.resolve();
    return message.title === undefined ? undefined : { id: message.id, title: message.title };
  });
  // @ts-expect-error -- a push reply is a notification.
  onPush(() => ({ title: "No id" }));
  // @ts-expect-error -- an opened notification has a source, and the event takes no reply.
  onNotificationOpened((notification) => notification.source);
  onNotificationOpened(({ action, input }) => {
    const chosen: string | undefined = action;
    const typed: string | undefined = input;
    void [chosen, typed];
  });
  // @ts-expect-error -- a dismissed notification has no action.
  onNotificationDismissed(({ action }) => void action);
});
