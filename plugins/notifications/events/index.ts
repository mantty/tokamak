import { defineEvent } from "@tokamakdev/plugin/events";

import "../src/types.js";

/**
 * Receives each data-only push message in native builds, whether the app is in the foreground,
 * in the background, or started by the message. A returned notification is shown.
 */
export const onPush = defineEvent("notifications.push");

/** Receives each notification the user opens, local or push, including one that opens the app. */
export const onNotificationOpened = defineEvent("notifications.opened");

/**
 * Receives each notification the user dismisses, local or push. Apple platforms report only those
 * of categories with `customDismissAction`.
 */
export const onNotificationDismissed = defineEvent("notifications.dismissed");
