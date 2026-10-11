import type {
  DismissedNotification,
  Listener,
  Message,
  NotificationContent,
  OpenedNotification,
  Permission,
  SetupConfig,
  SetupResult,
  SubscribeOptions,
  Subscription,
} from "../src/index.js";
import {
  DISMISSED,
  DISMISSED_CACHE,
  MESSAGE,
  OPENED,
  OPENED_CACHE,
  SUBSCRIPTION,
  type ServiceWorkerMessage,
  notificationData,
  pluginData,
  storeSetup,
  storedCategory,
  webActions,
  webPushSubscription,
} from "./protocol.js";

/** Stores the configuration for the service worker. The web uses only its categories. */
export async function setup(config: SetupConfig): Promise<SetupResult> {
  await storeSetup(config);
  return { unapplied: [] };
}

export function permission(): Promise<Permission> {
  return new Promise((resolve) => {
    resolve(fromBrowserPermission(notificationApi().permission));
  });
}

export async function requestPermission(): Promise<Permission> {
  return fromBrowserPermission(await notificationApi().requestPermission());
}

export async function show(notification: NotificationContent): Promise<void> {
  const api = notificationApi();
  if (api.permission !== "granted") throw notAllowed();
  const options = {
    body: notification.body,
    tag: notification.id,
    data: notificationData("local", notification.data, notification.category),
  };
  const registration = await serviceWorkerRegistration();
  if (registration) {
    const actions = webActions(await storedCategory(notification.category));
    await registration.showNotification(notification.title, {
      ...options,
      actions,
    } as NotificationOptions);
    return;
  }
  try {
    new api(notification.title, options);
  } catch {
    throw new DOMException("Showing notifications needs a service worker here", "NotSupportedError");
  }
}

export function schedule(): Promise<void> {
  return Promise.reject(
    new DOMException("Scheduled notifications are not supported on the web", "NotSupportedError"),
  );
}

export async function getDelivered(): Promise<NotificationContent[]> {
  const registration = await serviceWorkerRegistration();
  const shown = (await registration?.getNotifications()) ?? [];
  return shown.flatMap((notification) => {
    const data = pluginData(notification.data);
    if (!data) return [];
    return [{
      id: notification.tag,
      title: notification.title,
      body: notification.body,
      data: data.data,
      ...(data.category === undefined ? {} : { category: data.category }),
    }];
  });
}

export async function remove(id: string): Promise<void> {
  const registration = await serviceWorkerRegistration();
  for (const notification of (await registration?.getNotifications({ tag: id })) ?? []) {
    notification.close();
  }
}

export async function subscribe(options: SubscribeOptions): Promise<Subscription> {
  if (notificationApi().permission !== "granted") throw notAllowed();
  const registration = await serviceWorkerRegistration();
  if (!registration) {
    throw new DOMException("Web Push needs a registered service worker", "InvalidStateError");
  }
  const existing = await registration.pushManager.getSubscription();
  if (existing) return webPushSubscription(existing);
  if (!options.applicationServerKey) {
    throw new DOMException("Web Push needs the applicationServerKey option", "InvalidStateError");
  }
  const subscription = await registration.pushManager.subscribe({
    userVisibleOnly: true,
    applicationServerKey: options.applicationServerKey,
  });
  return webPushSubscription(subscription);
}

export async function getSubscription(): Promise<Subscription | null> {
  const registration = await serviceWorkerRegistration();
  const subscription = await registration?.pushManager.getSubscription();
  return subscription ? webPushSubscription(subscription) : null;
}

export async function unsubscribe(): Promise<void> {
  const registration = await serviceWorkerRegistration();
  await (await registration?.pushManager.getSubscription())?.unsubscribe();
}

export function onMessage(listener: Listener<Message>): () => void {
  return onServiceWorkerMessage((message) => {
    if (message.type === MESSAGE) listener(message.message);
  });
}

export function onSubscriptionChange(listener: Listener<Subscription>): () => void {
  return onServiceWorkerMessage((message) => {
    if (message.type === SUBSCRIPTION) listener(message.subscription);
  });
}

const openedListeners = new Set<Listener<OpenedNotification>>();
const dismissedListeners = new Set<Listener<DismissedNotification>>();

/** Delivers opened notifications the service worker holds, then each one it reports. */
export function onNotificationOpened(listener: Listener<OpenedNotification>): () => void {
  return onHeld(OPENED, OPENED_CACHE, openedListeners, listener);
}

/** Delivers dismissed notifications the service worker holds, then each one it reports. */
export function onNotificationDismissed(listener: Listener<DismissedNotification>): () => void {
  return onHeld(DISMISSED, DISMISSED_CACHE, dismissedListeners, listener);
}

/** Adds `listener` to `listeners`, which take what the service worker holds in `cacheName`. */
function onHeld<T>(
  type: typeof OPENED | typeof DISMISSED,
  cacheName: string,
  listeners: Set<Listener<T>>,
  listener: Listener<T>,
): () => void {
  listeners.add(listener);
  const stop = onServiceWorkerMessage((message) => {
    if (message.type === type) void takeHeld(cacheName, listeners);
  });
  void takeHeld(cacheName, listeners);
  return () => {
    listeners.delete(listener);
    stop();
  };
}

/** Takes each held notification once, even when several pages take them at once. */
async function takeHeld<T>(cacheName: string, listeners: Set<Listener<T>>): Promise<void> {
  if (listeners.size === 0 || !("caches" in globalThis)) return;
  const cache = await caches.open(cacheName);
  for (const request of await cache.keys()) {
    const response = await cache.match(request);
    if (!response || !(await cache.delete(request))) continue;
    const held = (await response.json()) as T;
    for (const listener of listeners) listener(held);
  }
}

function onServiceWorkerMessage(listener: Listener<ServiceWorkerMessage>): () => void {
  const container = serviceWorkerContainer();
  if (!container) return () => undefined;
  const receive = (event: MessageEvent<ServiceWorkerMessage>) => {
    listener(event.data);
  };
  container.addEventListener("message", receive);
  return () => {
    container.removeEventListener("message", receive);
  };
}

function notificationApi(): typeof Notification {
  const api = (globalThis as { Notification?: typeof Notification }).Notification;
  if (api) return api;
  throw new DOMException("Notifications are not supported here", "NotSupportedError");
}

function serviceWorkerContainer(): ServiceWorkerContainer | undefined {
  return (globalThis as { navigator?: { serviceWorker?: ServiceWorkerContainer } }).navigator
    ?.serviceWorker;
}

async function serviceWorkerRegistration(): Promise<ServiceWorkerRegistration | undefined> {
  return serviceWorkerContainer()?.getRegistration();
}

function fromBrowserPermission(permission: NotificationPermission): Permission {
  return permission === "default" ? "prompt" : permission;
}

function notAllowed(): DOMException {
  return new DOMException("Notification permission has not been granted", "NotAllowedError");
}
