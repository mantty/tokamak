import { count, flag, nonEmptyText, object } from "./checks.js";

/** Throws `TypeError` unless `options` are options `requestPermission` takes. */
export function validatePermissionOptions(options: unknown): void {
  object({
    alert: flag,
    sound: flag,
    badge: flag,
    carPlay: flag,
    criticalAlert: flag,
    provisional: flag,
    providesAppNotificationSettings: flag,
  })(options, "options");
}

/** Throws `TypeError` unless `count` is a badge count. */
export function validateBadge(value: unknown): void {
  count(value, "count");
}

/** Throws `TypeError` unless `options` are options `openSettings` takes, naming one setting. */
export function validateOpenSettings(options: unknown): void {
  object({ channel: nonEmptyText, exactAlarms: flag, fullScreenIntent: flag, doNotDisturbAccess: flag })(
    options,
    "options",
  );
  const named = Object.entries(options as Record<string, unknown>).filter(
    ([, value]) => value !== undefined && value !== false,
  );
  if (named.length > 1) throw new TypeError("options must name one setting");
}

/** Throws `TypeError` unless `topic` is an FCM topic name. */
export function validateTopic(topic: unknown): void {
  if (!(typeof topic === "string" && /^[a-zA-Z0-9\-_.~%]{1,900}$/.test(topic))) {
    throw new TypeError("topic must be 1 to 900 letters, digits and -_.~%");
  }
}
