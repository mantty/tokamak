import type { JsonValue } from "@tokamakdev/plugin";
import type {
  Category,
  Message,
  NotificationContent,
  OpenedNotification,
  SetupConfig,
  Subscription,
} from "../src/index.js";

/** Messages the service worker helper posts to pages. */
export const MESSAGE = "tokamak-notifications:message";
export const OPENED = "tokamak-notifications:opened";
export const DISMISSED = "tokamak-notifications:dismissed";
export const SUBSCRIPTION = "tokamak-notifications:subscription";

/** Hold opened and dismissed notifications until a page takes them. */
export const OPENED_CACHE = "tokamak-notifications-opened";
export const DISMISSED_CACHE = "tokamak-notifications-dismissed";

/** Holds the configuration `setup` applied, which the service worker reads. */
export const SETUP_CACHE = "tokamak-notifications-setup";
const SETUP_URL = "/__tokamak/notifications/setup";

/** What the plugin stores in `Notification.data`. */
export interface NotificationData {
  readonly tokamak: {
    readonly source: OpenedNotification["source"];
    readonly data: Record<string, JsonValue>;
    readonly category?: string;
  };
}

/** An action as `showNotification` takes it, with Chrome's text input. */
export interface WebAction {
  readonly action: string;
  readonly title: string;
  readonly type?: "text";
  readonly placeholder?: string;
  readonly icon?: string;
}

export type ServiceWorkerMessage =
  | { readonly type: typeof MESSAGE; readonly message: Message }
  | { readonly type: typeof OPENED }
  | { readonly type: typeof DISMISSED }
  | { readonly type: typeof SUBSCRIPTION; readonly subscription: Subscription };

export function notificationData(
  source: OpenedNotification["source"],
  data: Record<string, JsonValue> = {},
  category?: string,
): NotificationData {
  return { tokamak: category === undefined ? { source, data } : { source, data, category } };
}

/** The plugin's data on a notification it showed, or undefined for others. */
export function pluginData(value: unknown): NotificationData["tokamak"] | undefined {
  return (value as Partial<NotificationData> | null)?.tokamak;
}

export async function storeSetup(config: SetupConfig): Promise<void> {
  if (!("caches" in globalThis)) return;
  await (await caches.open(SETUP_CACHE)).put(SETUP_URL, Response.json(config));
}

/** The stored configuration's category `id`, or undefined. */
export async function storedCategory(id: string | undefined): Promise<Category | undefined> {
  if (id === undefined || !("caches" in globalThis)) return undefined;
  const stored = await (await caches.open(SETUP_CACHE)).match(SETUP_URL);
  const config = stored ? ((await stored.json()) as SetupConfig) : {};
  return config.categories?.find((category) => category.id === id);
}

/** The options of `content` that `showNotification` takes besides its body, tag, data and actions. */
export function webOptions(content: Partial<NotificationContent>): NotificationOptions {
  return {
    silent: content.silent,
    timestamp: content.timestamp,
    requireInteraction: content.requireInteraction,
    renotify: content.renotify,
    vibrate: content.vibrate,
    dir: content.dir,
    lang: content.lang,
    badge: content.badgeImage,
    image: content.image,
    icon: content.icon,
  } as NotificationOptions;
}

/**
 * Sets the app badge where the Badging API exists. Browsers badge installed web apps only and may
 * refuse elsewhere; the notification shows either way.
 */
export async function setAppBadge(count: number | undefined): Promise<void> {
  if (count === undefined || !("setAppBadge" in navigator)) return;
  await navigator.setAppBadge(count).catch(() => undefined);
}

/** A category's actions as `showNotification` takes them. */
export function webActions(category: Category | undefined): WebAction[] {
  return (category?.actions ?? []).map(({ id, title, input, icon }) => {
    const action = { action: id, title, ...(icon === undefined ? {} : { icon }) };
    if (!input) return action;
    const placeholder = input.placeholder === undefined ? {} : { placeholder: input.placeholder };
    return { ...action, type: "text", ...placeholder };
  });
}

export function webPushSubscription(subscription: PushSubscription): Subscription {
  const { keys } = subscription.toJSON();
  return {
    service: "webpush",
    endpoint: subscription.endpoint,
    keys: { p256dh: keys?.p256dh ?? "", auth: keys?.auth ?? "" },
  };
}
