import type { Listener } from "@tokamakdev/plugin/events";

import * as android from "./android.generated.js";
import * as apple from "./apple.generated.js";
import type { DismissedNotification, Message, NotificationContent, OpenedNotification } from "./types.js";

/**
 * Receives each data-only push message in native builds, whether the app is in the foreground,
 * in the background, or started by the message. A returned notification is shown.
 */
export function onPush(listener: Listener<Message, NotificationContent>): void {
  apple.onPush(listener);
  android.onPush(listener);
}

/** Receives each notification the user opens, local or push, including one that opens the app. */
export function onNotificationOpened(listener: Listener<OpenedNotification>): void {
  apple.onOpened(listener);
  android.onOpened(listener);
}

/**
 * Receives each notification the user dismisses, local or push. Apple platforms report only those
 * of categories with `customDismissAction`.
 */
export function onNotificationDismissed(listener: Listener<DismissedNotification>): void {
  apple.onDismissed(listener);
  android.onDismissed(listener);
}
