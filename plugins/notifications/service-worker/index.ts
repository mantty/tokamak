import type { JsonValue } from "@tokamakdev/plugin";
import type {
  DismissedNotification,
  Message,
  NotificationContent,
  OpenedNotification,
} from "../src/index.js";
import { pushContent } from "../src/content.js";
import {
  DISMISSED,
  DISMISSED_CACHE,
  MESSAGE,
  type NotificationData,
  OPENED,
  OPENED_CACHE,
  SUBSCRIPTION,
  type ServiceWorkerMessage,
  notificationData,
  pluginData,
  setAppBadge,
  storedCategory,
  webActions,
  webOptions,
  webPushSubscription,
} from "../web/protocol.js";

/** The parts of the service worker events and global scope the helpers use. */
interface ExtendableEvent {
  waitUntil(promise: Promise<unknown>): void;
}

interface PushEvent extends ExtendableEvent {
  readonly data: { text(): string } | null;
}

interface NotificationEvent extends ExtendableEvent {
  readonly notification: Notification;
  /** The chosen action's id; empty or absent for the notification itself. */
  readonly action?: string;
  /** Chrome only: the text typed into an action. */
  readonly reply?: string | null;
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
 * Shows the notification in a push message's JSON payload, with the `setup` category's actions,
 * and forwards the message to open pages. The payload has the fields `show` takes, all optional.
 */
export function handlePush(event: PushEvent): void {
  const fields = payload(event.data?.text());
  event.waitUntil(showAndForward(message(fields), isRecord(fields) ? pushContent(fields) : {}));
}

/**
 * Handles a click on a notification the plugin showed, or on one of its actions: holds it for
 * the page's `onNotificationOpened`. The notification itself and actions with `foreground` then
 * focus an open page or open `data.url` (default `/`). Returns false for other notifications.
 */
export function handleNotificationClick(event: NotificationEvent): boolean {
  const data = pluginData(event.notification.data);
  if (!data) return false;
  event.notification.close();
  const opened: OpenedNotification = {
    ...notification(event.notification, data),
    ...(event.action ? { action: event.action } : {}),
    ...(event.reply == null ? {} : { input: event.reply }),
  };
  event.waitUntil(open(opened, data.category));
  return true;
}

/**
 * Holds a notification the plugin showed, which the user dismissed, for the page's
 * `onNotificationDismissed`. Returns false for other notifications.
 */
export function handleNotificationClose(event: NotificationEvent): boolean {
  const data = pluginData(event.notification.data);
  if (!data) return false;
  event.waitUntil(dismiss(notification(event.notification, data)));
  return true;
}

/** Forwards the replacement subscription to open pages when the push service changes it. */
export function handleSubscriptionChange(event: PushSubscriptionChangeEvent): void {
  event.waitUntil(forwardSubscription(event));
}

async function showAndForward(pushed: Message, content: Partial<NotificationContent>): Promise<void> {
  await scope.registration.showNotification(pushed.title ?? "", {
    ...webOptions(content),
    body: pushed.body,
    tag: pushed.id,
    data: notificationData("push", pushed.data, content.category),
    actions: webActions(await storedCategory(content.category)),
  } as NotificationOptions);
  await post({ type: MESSAGE, message: pushed });
  await setAppBadge(content.badge);
}

function notification(shown: Notification, data: NotificationData["tokamak"]): DismissedNotification {
  return {
    id: shown.tag,
    title: shown.title,
    ...(shown.body ? { body: shown.body } : {}),
    data: data.data,
    source: data.source,
  };
}

async function open(opened: OpenedNotification, categoryId: string | undefined): Promise<void> {
  await hold(OPENED_CACHE, opened);
  if (opened.action === undefined || (await foreground(opened.action, categoryId))) {
    const client = (await windows()).at(0);
    if (client) {
      await client.focus();
    } else {
      await scope.clients.openWindow(typeof opened.data.url === "string" ? opened.data.url : "/");
    }
  }
  await post({ type: OPENED });
}

/** Whether the `setup` action opens the app. */
async function foreground(actionId: string, categoryId: string | undefined): Promise<boolean> {
  const action = (await storedCategory(categoryId))?.actions.find(({ id }) => id === actionId);
  return action?.foreground ?? false;
}

async function dismiss(dismissed: DismissedNotification): Promise<void> {
  await hold(DISMISSED_CACHE, dismissed);
  await post({ type: DISMISSED });
}

/** Keeps `value` in `cacheName` until a page takes it. */
async function hold(cacheName: string, value: unknown): Promise<void> {
  const cache = await caches.open(cacheName);
  await cache.put(`/__tokamak/notifications/${crypto.randomUUID()}`, Response.json(value));
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

/** A push payload's JSON, or its text as the body when it is not JSON. */
function payload(text: string | undefined): unknown {
  if (!text) return undefined;
  try {
    return JSON.parse(text);
  } catch {
    return { body: text };
  }
}

/** The message in a push payload; every field is optional. */
function message(payload: unknown): Message {
  const { id, title, body, data } = (payload ?? {}) as Partial<Record<keyof Message, unknown>>;
  return {
    id: typeof id === "string" ? id : crypto.randomUUID(),
    ...(typeof title === "string" ? { title } : {}),
    ...(typeof body === "string" ? { body } : {}),
    data: isRecord(data) ? data : {},
  };
}

function isRecord(value: unknown): value is Record<string, JsonValue> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
