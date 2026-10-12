import type { AndroidCategory, NotificationContent } from "./index.js";
import {
  type Check,
  array,
  colour,
  count,
  fields,
  flag,
  fraction,
  nonEmptyText,
  object,
  oneOf,
  pattern,
  record,
  source,
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
  bigPicture: object({ ...STYLE_TEXT, showWhenCollapsed: flag }),
  inbox: object({ ...STYLE_TEXT, lines: array(text) }, ["lines"]),
  messaging: object(
    {
      user: text,
      conversationTitle: text,
      messages: array(object({ text, at: time, sender: text }, ["text", "at"])),
    },
    ["user", "messages"],
  ),
};

/** A part of an image, in fractions of its width and height. */
const CLIPPING_RECT = object(
  { x: fraction, y: fraction, width: fraction, height: fraction },
  ["x", "y", "width", "height"],
);

const ATTACHMENT = object(
  {
    url: source,
    hideThumbnail: flag,
    thumbnailClippingRect: (value, path) => {
      CLIPPING_RECT(value, path);
      const { x, y, width, height } = value as Record<"x" | "y" | "width" | "height", number>;
      if (width === 0 || height === 0 || x + width > 1 || y + height > 1) {
        throw new TypeError(`${path} must lie within the image, with a width and height above 0`);
      }
    },
    thumbnailTime: (value, path) => {
      if (!(typeof value === "number" && Number.isFinite(value) && value >= 0)) {
        throw new TypeError(`${path} must be a number of seconds of at least 0`);
      }
    },
  },
  ["url"],
);

/** The check of each optional field. */
const CHECKS: Readonly<Record<string, Check>> = {
  body: text,
  data: record,
  category: nonEmptyText,
  channel: nonEmptyText,
  group: nonEmptyText,
  image: source,
  icon: source,
  attachments: array(ATTACHMENT),
  badge: count,
  silent: flag,
  timestamp: time,
  subtitle: text,
  interruptionLevel: oneOf("passive", "active", "timeSensitive", "critical"),
  relevanceScore: fraction,
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
  if ((value.style as { type?: unknown } | undefined)?.type === "bigPicture" && value.image === undefined) {
    throw new TypeError("image must be given with the bigPicture style");
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
