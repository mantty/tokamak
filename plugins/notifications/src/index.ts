import { Plugin } from "@tokamakdev/plugin";
import * as web from "../web/index.js";
import { validateContent } from "./content.js";
import { validateBadge, validateOpenSettings, validatePermissionOptions, validateTopic } from "./options.js";
import { validateSetup } from "./setup.js";
import type {
  ErrorListener,
  Listener,
  Message,
  DismissedNotification,
  NotificationContent,
  NotificationSettings,
  OpenSettingsEvent,
  OpenSettingsOptions,
  OpenedNotification,
  Permission,
  PermissionOptions,
  ScheduledNotification,
  SetupConfig,
  SetupResult,
  SubscribeOptions,
  Subscription,
} from "./types.js";

export type * from "./types.js";

class Notifications extends Plugin {
  constructor() {
    super("notifications");
  }

  /**
   * Applies the configuration of every platform, replacing the previous one. Rejects with
   * `TypeError`, changing nothing, when a value is invalid.
   */
  async setup(config: SetupConfig): Promise<SetupResult> {
    validateSetup(config);
    return this.hasNativeTransport ? this.call("setup", config) : web.setup(config);
  }

  permission(): Promise<Permission> {
    return this.hasNativeTransport ? this.call("permission") : web.permission();
  }

  /**
   * Asks the user for permission to show notifications, when the system has not asked already.
   * Apple platforms ask for what `options` names: a provisional grant asks again without
   * `provisional`, and a grant gains `criticalAlert` or `providesAppNotificationSettings` when asked.
   */
  async requestPermission(options: PermissionOptions = {}): Promise<Permission> {
    validatePermissionOptions(options);
    return this.hasNativeTransport
      ? this.call("requestPermission", options)
      : web.requestPermission();
  }

  /** The user's notification settings. */
  settings(): Promise<NotificationSettings> {
    return this.hasNativeTransport ? this.call("settings") : web.settings();
  }

  /** Sets the app's badge count, or removes the badge with 0. Not supported on Android. */
  async setBadge(count: number): Promise<void> {
    validateBadge(count);
    return this.hasNativeTransport ? this.call("setBadge", { count }) : web.setBadge(count);
  }

  /** Opens the app's notification settings in the system's. Not supported on the web. */
  async openSettings(options: OpenSettingsOptions = {}): Promise<void> {
    validateOpenSettings(options);
    return this.hasNativeTransport ? this.call("openSettings", options) : web.openSettings();
  }

  /** Shows a notification now. Rejects with `TypeError`, showing nothing, when a value is invalid. */
  async show(notification: NotificationContent): Promise<void> {
    validateContent(notification, false);
    return this.hasNativeTransport ? this.call("show", notification) : web.show(notification);
  }

  /**
   * Shows a notification when its trigger fires: at `at`, or now when `at` has passed; once each
   * `every` interval; at the dates `on` matches; or, on iOS, on entering or leaving a `region`.
   * Not supported on the web.
   */
  async schedule(notification: ScheduledNotification): Promise<void> {
    validateContent(notification, true);
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
    return this.hasNativeTransport ? this.call("subscribe") : web.subscribe(options);
  }

  /** The current push subscription, or null. */
  getSubscription(): Promise<Subscription | null> {
    return this.hasNativeTransport ? this.call("getSubscription") : web.getSubscription();
  }

  unsubscribe(): Promise<void> {
    return this.hasNativeTransport ? this.call("unsubscribe") : web.unsubscribe();
  }

  /** FCM only: receives the push messages the app's server sends to `topic`. */
  async subscribeToTopic(topic: string): Promise<void> {
    validateTopic(topic);
    return this.hasNativeTransport ? this.call("subscribeToTopic", { topic }) : web.topics();
  }

  /** FCM only: stops receiving the push messages sent to `topic`. */
  async unsubscribeFromTopic(topic: string): Promise<void> {
    validateTopic(topic);
    return this.hasNativeTransport ? this.call("unsubscribeFromTopic", { topic }) : web.topics();
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

  /**
   * Receives notifications the user dismissed. Apple platforms report only those of categories
   * with `customDismissAction`; Android also reports those its `timeoutAfter` removed.
   */
  onNotificationDismissed(
    listener: Listener<DismissedNotification>,
    error: ErrorListener = report,
  ): () => void {
    if (!this.hasNativeTransport) return web.onNotificationDismissed(listener);
    return this.listen("onNotificationDismissed", listener, error);
  }

  /**
   * Receives the user's requests, from the system's notification settings, for the app's own,
   * including the request that opened the app. Apple platforms need `requestPermission`'s
   * `providesAppNotificationSettings`; Android links from the app's notification settings and from
   * a notification's `settingsText`.
   */
  onOpenSettings(listener: Listener<OpenSettingsEvent>, error: ErrorListener = report): () => void {
    if (!this.hasNativeTransport) return () => undefined;
    return this.listen("onOpenSettings", listener, error);
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
