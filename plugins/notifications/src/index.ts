import { capabilitiesOf, platformApi, type JsonStream, type TokamakContext } from "@tokamakdev/plugin";

import { android } from "./android.generated.js";
import { apple } from "./apple.generated.js";
import type {
  DismissedNotification,
  Message,
  NotificationContent,
  NotificationSettings,
  OpenedNotification,
  OpenSettingsEvent,
  OpenSettingsOptions,
  Permission,
  PermissionOptions,
  ScheduledNotification,
  SetupConfig,
  SetupResult,
  SubscribeOptions,
  Subscription,
  Trigger,
} from "./types.js";

export type * from "./types.js";

export function notifications(ctx: TokamakContext) {
  const native = platformApi(ctx, { apple, android });
  return {
    /** Whether each operation is implemented where the app runs, without calling it. Permission is not reported. */
    get capabilities(): Readonly<Record<string, boolean>> {
      return capabilitiesOf(ctx, "notifications");
    },

    /** Applies the configuration of every platform, replacing the previous one. */
    setup: (config: SetupConfig): Promise<SetupResult> => native.setup(config),

    permission: (): Promise<Permission> => native.permission(),

    /**
     * Asks the user for permission to show notifications, when the system has not asked already.
     * Apple platforms ask for what `options` names: a provisional grant asks again without
     * `provisional`, and a grant gains `criticalAlert` or `providesAppNotificationSettings` when asked.
     * Off screen on iOS and Android, a request that would show the prompt rejects with `NeedsUIError`.
     */
    requestPermission: (options?: PermissionOptions): Promise<Permission> => native.requestPermission(options),

    /** The user's notification settings. */
    settings: (): Promise<NotificationSettings> => native.settings(),

    /**
     * Sets the app's badge count, or removes the badge with 0.
     * @platforms ios macos
     */
    setBadge: (count: number): Promise<void> => apple(ctx).setBadge(count),

    /**
     * Opens the app's notification settings in the system's. Rejects with `NeedsUIError` off screen on
     * iOS and Android. Not supported on the web.
     */
    openSettings: (options?: OpenSettingsOptions): Promise<void> => native.openSettings(options),

    /** Shows a notification now. */
    show: (notification: NotificationContent): Promise<void> => native.show(notification),

    /**
     * Shows a notification when its trigger fires: at `at`, or now when `at` has passed; once each
     * `every` interval; at the dates `on` matches; or, on iOS, on entering or leaving a `region`.
     * Not supported on the web.
     */
    schedule(notification: ScheduledNotification): Promise<void> {
      const [content, trigger] = split(notification);
      // Only iOS has region triggers; the Apple API rejects them elsewhere with NotSupportedError.
      return "region" in trigger ? apple(ctx).schedule(content, trigger) : native.schedule(content, trigger);
    },

    /** Scheduled notifications not yet shown, with the fields of the platform that shows them. */
    async getScheduled(): Promise<ScheduledNotification[]> {
      const scheduled = await native.getScheduled();
      return scheduled.map(({ content, trigger }) => ({ ...content, ...trigger }));
    },

    /** Shown notifications still in the notification centre. */
    getDelivered: (): Promise<NotificationContent[]> => native.getDelivered(),

    /** Removes a scheduled or shown notification. */
    remove: (id: string): Promise<void> => native.remove(id),

    /**
     * Subscribes to push messages, returning the existing subscription when there is one. `options`
     * apply to Web Push.
     */
    subscribe: (_options?: SubscribeOptions): Promise<Subscription> => native.subscribeToPush(),

    /** The current push subscription, or null. */
    getSubscription: (): Promise<Subscription | null> => native.getSubscription(),

    unsubscribe: (): Promise<void> => native.unsubscribe(),

    /**
     * Receives the push messages the app's server sends to `topic` through FCM.
     * @platforms android
     */
    subscribeToTopic: (topic: string): Promise<void> => android(ctx).subscribeToTopic(topic),

    /**
     * Stops receiving the push messages sent to `topic` through FCM.
     * @platforms android
     */
    unsubscribeFromTopic: (topic: string): Promise<void> => android(ctx).unsubscribeFromTopic(topic),

    /** Receives push messages that arrive while the page is loaded. */
    onMessage: (): JsonStream<Message> => native.onMessage(),

    /** Receives opened notifications, local or push, including the one that opened the app. */
    onNotificationOpened: (): JsonStream<OpenedNotification> => native.onNotificationOpened(),

    /**
     * Receives notifications the user dismissed. Apple platforms report only those of categories
     * with `customDismissAction`; Android also reports those its `timeoutAfter` removed.
     */
    onNotificationDismissed: (): JsonStream<DismissedNotification> => native.onNotificationDismissed(),

    /**
     * Receives the user's requests, from the system's notification settings, for the app's own,
     * including the request that opened the app. Apple platforms need `requestPermission`'s
     * `providesAppNotificationSettings`; Android links from the app's notification settings and from
     * a notification's `settingsText`.
     */
    onOpenSettings: (): JsonStream<OpenSettingsEvent> => native.onOpenSettings(),

    /** Receives the replacement when the push service changes the subscription. */
    onSubscriptionChange: (): JsonStream<Subscription> => native.onSubscriptionChange(),
  };
}

/** A scheduled notification's content and its trigger, which the trigger's fields name. */
function split(notification: ScheduledNotification): [NotificationContent, Trigger] {
  if ("at" in notification) {
    const { at, exact, ...content } = notification;
    return [content, { at, exact }];
  }
  if ("every" in notification) {
    const { every, exact, ...content } = notification;
    return [content, { every, exact }];
  }
  if ("on" in notification) {
    const { on, repeats, exact, ...content } = notification;
    return [content, { on, repeats, exact }];
  }
  const { region, repeats, ...content } = notification;
  return [content, { region, repeats }];
}
