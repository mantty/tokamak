import type { Message, OpenedNotification, Subscription } from "../src/index.js";

/** Messages the service worker helper posts to pages. */
export const MESSAGE = "tokamak-notifications:message";
export const OPENED = "tokamak-notifications:opened";
export const SUBSCRIPTION = "tokamak-notifications:subscription";

/** Holds opened notifications until a page takes them. */
export const OPENED_CACHE = "tokamak-notifications-opened";

/** What the plugin stores in `Notification.data`. */
export interface NotificationData {
  readonly tokamak: {
    readonly source: OpenedNotification["source"];
    readonly data: Record<string, unknown>;
  };
}

export type ServiceWorkerMessage =
  | { readonly type: typeof MESSAGE; readonly message: Message }
  | { readonly type: typeof OPENED }
  | { readonly type: typeof SUBSCRIPTION; readonly subscription: Subscription };

export function notificationData(
  source: OpenedNotification["source"],
  data: Record<string, unknown> = {},
): NotificationData {
  return { tokamak: { source, data } };
}

/** The plugin's data on a notification it showed, or undefined for others. */
export function pluginData(value: unknown): NotificationData["tokamak"] | undefined {
  return (value as Partial<NotificationData> | null)?.tokamak;
}

export function webPushSubscription(subscription: PushSubscription): Subscription {
  const { keys } = subscription.toJSON();
  return {
    service: "webpush",
    endpoint: subscription.endpoint,
    keys: { p256dh: keys?.p256dh ?? "", auth: keys?.auth ?? "" },
  };
}
