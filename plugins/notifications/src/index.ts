import { Plugin, type TokamakContext } from "@tokamakdev/plugin";
import { validateContent } from "./content.js";
import { validateBadge, validateOpenSettings, validatePermissionOptions, validateTopic } from "./options.js";
import { validateSetup } from "./setup.js";
import type {
  NotificationContent,
  NotificationSettings,
  OpenSettingsOptions,
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
  constructor(ctx: TokamakContext) {
    super(ctx, "notifications");
  }

  /**
   * Applies the configuration of every platform, replacing the previous one. Rejects with
   * `TypeError`, changing nothing, when a value is invalid.
   */
  async setup(config: SetupConfig): Promise<SetupResult> {
    validateSetup(config);
    return this.call("setup", config);
  }

  permission(): Promise<Permission> {
    return this.call("permission");
  }

  /**
   * Asks the user for permission to show notifications, when the system has not asked already.
   * Apple platforms ask for what `options` names: a provisional grant asks again without
   * `provisional`, and a grant gains `criticalAlert` or `providesAppNotificationSettings` when asked.
   */
  async requestPermission(options: PermissionOptions = {}): Promise<Permission> {
    validatePermissionOptions(options);
    return this.call("requestPermission", options);
  }

  /** The user's notification settings. */
  settings(): Promise<NotificationSettings> {
    return this.call("settings");
  }

  /** Sets the app's badge count, or removes the badge with 0. Not supported on Android. */
  async setBadge(count: number): Promise<void> {
    validateBadge(count);
    return this.call("setBadge", { count });
  }

  /** Opens the app's notification settings in the system's. Not supported on the web. */
  async openSettings(options: OpenSettingsOptions = {}): Promise<void> {
    validateOpenSettings(options);
    return this.call("openSettings", options);
  }

  /** Shows a notification now. Rejects with `TypeError`, showing nothing, when a value is invalid. */
  async show(notification: NotificationContent): Promise<void> {
    validateContent(notification, false);
    return this.call("show", notification);
  }

  /**
   * Shows a notification when its trigger fires: at `at`, or now when `at` has passed; once each
   * `every` interval; at the dates `on` matches; or, on iOS, on entering or leaving a `region`.
   * Not supported on the web.
   */
  async schedule(notification: ScheduledNotification): Promise<void> {
    validateContent(notification, true);
    return this.call("schedule", notification);
  }

  /** Scheduled notifications not yet shown. */
  getScheduled(): Promise<ScheduledNotification[]> {
    return this.call("getScheduled");
  }

  /** Shown notifications still in the notification centre. */
  getDelivered(): Promise<NotificationContent[]> {
    return this.call("getDelivered");
  }

  /** Removes a scheduled or shown notification. */
  remove(id: string): Promise<void> {
    return this.call("remove", { id });
  }

  /** Subscribes to push messages, returning the existing subscription when there is one. */
  subscribe(options: SubscribeOptions = {}): Promise<Subscription> {
    return this.call("subscribe", options);
  }

  /** The current push subscription, or null. */
  getSubscription(): Promise<Subscription | null> {
    return this.call("getSubscription");
  }

  unsubscribe(): Promise<void> {
    return this.call("unsubscribe");
  }

  /** FCM only: receives the push messages the app's server sends to `topic`. */
  async subscribeToTopic(topic: string): Promise<void> {
    validateTopic(topic);
    return this.call("subscribeToTopic", { topic });
  }

  /** FCM only: stops receiving the push messages sent to `topic`. */
  async unsubscribeFromTopic(topic: string): Promise<void> {
    validateTopic(topic);
    return this.call("unsubscribeFromTopic", { topic });
  }

  /** Receives push messages that arrive while the page is loaded. */
  onMessage(): ReadableStream<Uint8Array> {
    return this.stream("onMessage");
  }

  /** Receives opened notifications, local or push, including the one that opened the app. */
  onNotificationOpened(): ReadableStream<Uint8Array> {
    return this.stream("onNotificationOpened");
  }

  /**
   * Receives notifications the user dismissed. Apple platforms report only those of categories
   * with `customDismissAction`; Android also reports those its `timeoutAfter` removed.
   */
  onNotificationDismissed(): ReadableStream<Uint8Array> {
    return this.stream("onNotificationDismissed");
  }

  /**
   * Receives the user's requests, from the system's notification settings, for the app's own,
   * including the request that opened the app. Apple platforms need `requestPermission`'s
   * `providesAppNotificationSettings`; Android links from the app's notification settings and from
   * a notification's `settingsText`.
   */
  onOpenSettings(): ReadableStream<Uint8Array> {
    return this.stream("onOpenSettings");
  }

  /** Receives the replacement when the push service changes the subscription. */
  onSubscriptionChange(): ReadableStream<Uint8Array> {
    return this.stream("onSubscriptionChange");
  }
}


export function notifications(ctx: TokamakContext): Notifications { return new Notifications(ctx); }
