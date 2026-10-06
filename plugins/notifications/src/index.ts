import { FrontendPlugin } from "@tokamakdev/plugin";
import * as web from "../web/index.js";

/** Whether the app may show notifications. */
export type Permission = "granted" | "denied" | "prompt";

export interface NotificationContent {
  /** Showing or scheduling another notification with the same id replaces this one. */
  readonly id: string;
  readonly title: string;
  readonly body?: string;
  readonly data?: Record<string, unknown>;
}

export interface ScheduledNotification extends NotificationContent {
  /** Milliseconds since the epoch; the earliest time the notification is shown. */
  readonly at: number;
}

/** A received push message. A data-only message has no title or body. */
export interface Message<Data extends object = Record<string, unknown>> {
  readonly id: string;
  readonly title: string | null;
  readonly body: string | null;
  readonly data: Data;
}

export interface OpenedNotification extends Message {
  readonly source: "local" | "push";
}

/** Where the app's server sends push messages for this installation. */
export type Subscription =
  | {
      readonly service: "apns";
      readonly token: string;
      readonly environment: "development" | "production";
    }
  | { readonly service: "fcm"; readonly token: string }
  | {
      readonly service: "webpush";
      readonly endpoint: string;
      readonly keys: { readonly p256dh: string; readonly auth: string };
    };

export interface SubscribeOptions {
  /** Web Push only: the app's VAPID public key, base64url-encoded. */
  readonly applicationServerKey?: string;
  /** Also show push notifications that arrive while the page is visible. Defaults to false. */
  readonly showInForeground?: boolean;
}

export type Listener<T> = (value: T) => void;
export type ErrorListener = (error: DOMException) => void;

class Notifications extends FrontendPlugin {
  constructor() {
    super("notifications");
  }

  permission(): Promise<Permission> {
    return this.hasNativeTransport ? this.call("permission") : web.permission();
  }

  /** Asks the user for permission to show notifications. */
  requestPermission(): Promise<Permission> {
    return this.hasNativeTransport ? this.call("requestPermission") : web.requestPermission();
  }

  /** Shows a notification now, even while the page is visible. */
  show(notification: NotificationContent): Promise<void> {
    return this.hasNativeTransport ? this.call("show", notification) : web.show(notification);
  }

  /** Shows a notification at `at`, or now when `at` has passed. Not supported on the web. */
  schedule(notification: ScheduledNotification): Promise<void> {
    return this.hasNativeTransport ? this.call("schedule", notification) : web.schedule();
  }

  /** Scheduled notifications not yet shown. */
  getScheduled(): Promise<ScheduledNotification[]> {
    return this.hasNativeTransport ? this.call("getScheduled") : Promise.resolve([]);
  }

  /** Shown notifications still in the notification centre. */
  getDelivered(): Promise<NotificationContent[]> {
    return this.hasNativeTransport ? this.call("getDelivered") : web.getDelivered();
  }

  /** Removes a scheduled or shown notification. */
  remove(id: string): Promise<void> {
    return this.hasNativeTransport ? this.call("remove", { id }) : web.remove(id);
  }

  /** Subscribes to push messages, returning the existing subscription when there is one. */
  subscribe(options: SubscribeOptions = {}): Promise<Subscription> {
    if (!this.hasNativeTransport) return web.subscribe(options);
    return this.call("subscribe", { showInForeground: options.showInForeground ?? false });
  }

  /** The current push subscription, or null. */
  getSubscription(): Promise<Subscription | null> {
    return this.hasNativeTransport ? this.call("getSubscription") : web.getSubscription();
  }

  unsubscribe(): Promise<void> {
    return this.hasNativeTransport ? this.call("unsubscribe") : web.unsubscribe();
  }

  /** Receives push messages that arrive while the page is loaded. */
  onMessage(listener: Listener<Message>, error: ErrorListener = report): () => void {
    if (!this.hasNativeTransport) return web.onMessage(listener);
    return this.listen("onMessage", listener, error);
  }

  /** Receives opened notifications, local or push, including the one that opened the app. */
  onNotificationOpened(
    listener: Listener<OpenedNotification>,
    error: ErrorListener = report,
  ): () => void {
    if (!this.hasNativeTransport) return web.onNotificationOpened(listener);
    return this.listen("onNotificationOpened", listener, error);
  }

  /** Receives the replacement when the push service changes the subscription. */
  onSubscriptionChange(
    listener: Listener<Subscription>,
    error: ErrorListener = report,
  ): () => void {
    if (!this.hasNativeTransport) return web.onSubscriptionChange(listener);
    return this.listen("onSubscriptionChange", listener, error);
  }
}

function report(error: DOMException): void {
  console.error(error);
}

export const notifications = new Notifications();
