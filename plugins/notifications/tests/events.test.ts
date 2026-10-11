import assert from "node:assert/strict";
import { test } from "node:test";

import { dispatch } from "@tokamakdev/plugin/testing";

import { onNotificationDismissed, onNotificationOpened, onPush } from "../events/index.js";

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

void test("delivers opened actions and dismissed notifications", async () => {
  const notification = { id: "n1", title: "Hi", body: null, data: {}, source: "local" as const };
  const received: unknown[] = [];
  onNotificationOpened(({ action, input }) => void received.push({ action, input }));
  onNotificationDismissed(({ id }) => void received.push(id));

  await dispatch("notifications.opened", { ...notification, action: "reply", input: "On my way" });
  await dispatch("notifications.dismissed", notification);

  assert.deepEqual(received, [{ action: "reply", input: "On my way" }, "n1"]);
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
  onNotificationOpened(({ action, input }) => {
    const chosen: string | null = action;
    const typed: string | null = input;
    void [chosen, typed];
  });
  // @ts-expect-error -- a dismissed notification has no action.
  onNotificationDismissed(({ action }) => void action);
});
