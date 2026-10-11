import type { AndroidCategory, NotificationContent } from "./index.js";
import {
  type Check,
  colour,
  count,
  fields,
  flag,
  nonEmptyText,
  object,
  oneOf,
  pattern,
  record,
  strings,
  text,
  time,
} from "./checks.js";

const ANDROID_CATEGORIES = {
  alarm: true,
  call: true,
  email: true,
  error: true,
  event: true,
  locationSharing: true,
  message: true,
  missedCall: true,
  navigation: true,
  progress: true,
  promo: true,
  recommendation: true,
  reminder: true,
  service: true,
  social: true,
  status: true,
  stopwatch: true,
  system: true,
  transport: true,
  workout: true,
} as const satisfies Record<AndroidCategory, true>;

const STYLE_TEXT = { title: text, summary: text };
const STYLES: Readonly<Record<string, Check>> = {
  bigText: object({ ...STYLE_TEXT, text }, ["text"]),
  inbox: object({ ...STYLE_TEXT, lines: strings }, ["lines"]),
  messaging: object(
    {
      user: text,
      conversationTitle: text,
      messages: (value, path) => {
        if (!Array.isArray(value)) throw new TypeError(`${path} must be an array`);
        const message = object({ text, at: time, sender: text }, ["text", "at"]);
        for (const [index, entry] of (value as unknown[]).entries()) {
          message(entry, `${path}[${String(index)}]`);
        }
      },
    },
    ["user", "messages"],
  ),
};

/** The check of each optional field. */
const CHECKS: Readonly<Record<string, Check>> = {
  body: text,
  data: record,
  category: nonEmptyText,
  channel: nonEmptyText,
  group: nonEmptyText,
  badge: count,
  silent: flag,
  timestamp: time,
  subtitle: text,
  interruptionLevel: oneOf("passive", "active", "timeSensitive", "critical"),
  relevanceScore: (value, path) => {
    if (!(typeof value === "number" && value >= 0 && value <= 1)) {
      throw new TypeError(`${path} must be a number from 0 to 1`);
    }
  },
  subText: text,
  color: colour,
  style: (value, path) => {
    record(value, path);
    const type = value.type as string;
    if (!Object.hasOwn(STYLES, type)) {
      throw new TypeError(`${path}.type must be one of "${Object.keys(STYLES).join('", "')}"`);
    }
    STYLES[type](value, path);
  },
  progress: (value, path) => {
    record(value, path);
    if (value.indeterminate === true) return;
    fields(value, path, { current: count, max: count }, ["current", "max"]);
    if ((value.current as number) > (value.max as number)) {
      throw new TypeError(`${path}.current must not exceed ${path}.max`);
    }
  },
  ongoing: flag,
  autoCancel: flag,
  onlyAlertOnce: flag,
  timeoutAfter: count,
  chronometer: (value, path) => {
    if (typeof value === "boolean") return;
    object({ countDown: flag }, ["countDown"])(value, path);
  },
  visibility: oneOf("public", "private", "secret"),
  publicVersion: object({ title: text, body: text }, ["title"]),
  sortKey: text,
  androidCategory: oneOf(...Object.keys(ANDROID_CATEGORIES)),
  requireInteraction: flag,
  renotify: flag,
  vibrate: pattern,
  dir: oneOf("auto", "ltr", "rtl"),
  lang: text,
  badgeImage: text,
};

/**
 * Throws `TypeError`, naming the first invalid value, unless `value` is a notification `show` or,
 * with `scheduled`, `schedule` takes.
 */
export function validateContent(value: unknown, scheduled: boolean): void {
  record(value, "notification");
  const required = scheduled ? ["id", "title", "at"] : ["id", "title"];
  fields(value, "", { ...CHECKS, id: nonEmptyText, title: text, at: time }, required);
  // Browsers reject the pair.
  if (value.silent === true && value.vibrate !== undefined) {
    throw new TypeError("vibrate must be left out of a silent notification");
  }
}

/** The content fields of a Web Push payload, without those that are invalid. */
export function pushContent(payload: Record<string, unknown>): Partial<NotificationContent> {
  const content: Record<string, unknown> = {};
  for (const [key, check] of Object.entries(CHECKS)) {
    if (payload[key] === undefined) continue;
    try {
      check(payload[key], key);
      content[key] = payload[key];
    } catch {
      // The server's mistake; the notification shows without the field.
    }
  }
  if (content.silent === true) delete content.vibrate;
  return content;
}
