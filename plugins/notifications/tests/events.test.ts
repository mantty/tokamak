import assert from "node:assert/strict";
import { test } from "node:test";

import { dispatch } from "@tokamakdev/plugin/testing";

import { onNotificationOpened, onPush } from "../events/index.js";

void test("replies to a push message with the notification a listener returns", async () => {
  const message = { id: "m1", title: null, body: null, data: { itemId: "42" } };
  const received: unknown[] = [];
  onPush((pushed) => {
    received.push(pushed);
    return { id: `item-${String(pushed.data.itemId)}`, title: "New item" };
  });

  const { reply } = await dispatch("notifications.push", message);

  assert.deepEqual(received, [message]);
  assert.deepEqual(reply, { id: "item-42", title: "New item" });
});

void test("delivers opened notifications", async () => {
  const opened = { id: "n1", title: "Hi", body: null, data: {}, source: "local" as const };
  const received: unknown[] = [];
  onNotificationOpened((notification) => void received.push(notification.source));

  await dispatch("notifications.opened", opened);

  assert.deepEqual(received, ["local"]);
});

void test("types the listeners", () => {
  onPush(async (message) => {
    await Promise.resolve();
    return message.title === null ? undefined : { id: message.id, title: message.title };
  });
  // @ts-expect-error -- a push reply is a notification.
  onPush(() => ({ title: "No id" }));
  // @ts-expect-error -- an opened notification has a source, and the event takes no reply.
  onNotificationOpened((notification) => notification.source);
});
