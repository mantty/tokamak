import type { Message, OpenedNotification } from "../src/index.js";
import {
  MESSAGE,
  OPENED,
  OPENED_CACHE,
  SUBSCRIPTION,
  type ServiceWorkerMessage,
  notificationData,
  pluginData,
  webPushSubscription,
} from "../web/protocol.js";

/** The parts of the service worker events and global scope the helpers use. */
interface ExtendableEvent {
  waitUntil(promise: Promise<unknown>): void;
}

interface PushEvent extends ExtendableEvent {
  readonly data: { json(): unknown } | null;
}

interface NotificationEvent extends ExtendableEvent {
  readonly notification: Notification;
}

interface PushSubscriptionChangeEvent extends ExtendableEvent {
  readonly newSubscription?: PushSubscription | null;
  readonly oldSubscription?: PushSubscription | null;
}

interface WindowClient {
  readonly url: string;
  focus(): Promise<WindowClient>;
  postMessage(message: ServiceWorkerMessage): void;
}

interface ServiceWorkerScope {
  readonly registration: ServiceWorkerRegistration;
  readonly clients: {
    matchAll(options: { type: "window"; includeUncontrolled: boolean }): Promise<WindowClient[]>;
    openWindow(url: string): Promise<WindowClient | null>;
  };
}

const scope = globalThis as unknown as ServiceWorkerScope;

/**
 * Shows the notification in a push message's `{ id, title, body, data }` JSON payload and
 * forwards the message to open pages.
 */
export function handlePush(event: PushEvent): void {
  event.waitUntil(showAndForward(message(event.data?.json())));
}

/**
 * Handles a click on a notification the plugin showed: holds it for the page's
 * `onNotificationOpened`, then focuses an open page or opens `data.url` (default `/`).
 * Returns false for other notifications.
 */
export function handleNotificationClick(event: NotificationEvent): boolean {
  const data = pluginData(event.notification.data);
  if (!data) return false;
  event.notification.close();
  const opened: OpenedNotification = {
    id: event.notification.tag,
    title: event.notification.title,
    body: event.notification.body || null,
    data: data.data,
    source: data.source,
  };
  event.waitUntil(open(opened));
  return true;
}

/** Forwards the replacement subscription to open pages when the push service changes it. */
export function handleSubscriptionChange(event: PushSubscriptionChangeEvent): void {
  event.waitUntil(forwardSubscription(event));
}

async function showAndForward(pushed: Message): Promise<void> {
  await scope.registration.showNotification(pushed.title ?? "", {
    body: pushed.body ?? undefined,
    tag: pushed.id,
    data: notificationData("push", pushed.data),
  });
  await post({ type: MESSAGE, message: pushed });
}

async function open(opened: OpenedNotification): Promise<void> {
  const cache = await caches.open(OPENED_CACHE);
  await cache.put(`/__tokamak/notifications/opened/${crypto.randomUUID()}`, Response.json(opened));
  const client = (await windows()).at(0);
  if (client) {
    await client.focus();
  } else {
    await scope.clients.openWindow(typeof opened.data.url === "string" ? opened.data.url : "/");
  }
  await post({ type: OPENED });
}

async function forwardSubscription(event: PushSubscriptionChangeEvent): Promise<void> {
  const options = event.oldSubscription?.options;
  const subscription =
    event.newSubscription ??
    (options ? await scope.registration.pushManager.subscribe(options) : null);
  if (subscription) {
    await post({ type: SUBSCRIPTION, subscription: webPushSubscription(subscription) });
  }
}

async function post(message: ServiceWorkerMessage): Promise<void> {
  for (const client of await windows()) client.postMessage(message);
}

function windows(): Promise<WindowClient[]> {
  return scope.clients.matchAll({ type: "window", includeUncontrolled: true });
}

/** The message in a push payload; every field is optional. */
function message(payload: unknown): Message {
  const fields = (payload ?? {}) as Partial<Record<keyof Message, unknown>>;
  return {
    id: typeof fields.id === "string" ? fields.id : crypto.randomUUID(),
    title: typeof fields.title === "string" ? fields.title : null,
    body: typeof fields.body === "string" ? fields.body : null,
    data: isRecord(fields.data) ? fields.data : {},
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
