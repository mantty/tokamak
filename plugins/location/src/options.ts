import { atLeastZero, flag, nonEmptyText, object, oneOf, text, wholeAtLeastZero } from "./checks.js";

/** Throws `TypeError` unless `options` are options `requestPermission` takes. */
export const validatePermissionRequest = object({
  background: flag,
  precision: oneOf("precise", "approximate"),
  purpose: nonEmptyText,
});

/** Throws `TypeError` unless `options` are options `startUpdates` takes. */
export const validateUpdateOptions = object({
  activity: oneOf("default", "automotiveNavigation", "otherNavigation", "fitness", "airborne", "maritime"),
  accuracy: oneOf("high", "balanced", "low"),
  distance: atLeastZero,
  interval: wholeAtLeastZero,
  foregroundService: object({ title: nonEmptyText, body: text, channel: nonEmptyText }, ["title", "body", "channel"]),
});
