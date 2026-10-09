import { defineEvent } from "@tokamakdev/plugin/events";

import type { Message, NotificationContent, OpenedNotification } from "../src/index.js";

/**
 * Receives each data-only push message in native builds, whether the app is in the foreground,
 * in the background, or started by the message. A returned notification is shown.
 */
export const onPush = defineEvent<Message, NotificationContent>("notifications.push");

/** Receives each notification the user opens, local or push, including one that opens the app. */
export const onNotificationOpened = defineEvent<OpenedNotification>("notifications.opened");
