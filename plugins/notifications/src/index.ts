import { Plugin } from "@tokamakdev/plugin";
import * as web from "../web/index.js";
import { validateSetup } from "./setup.js";

/** Whether the app may show notifications. */
export type Permission = "granted" | "denied" | "prompt";

export interface NotificationContent {
  /** Showing or scheduling another notification with the same id replaces this one. */
  readonly id: string;
  readonly title: string;
  readonly body?: string;
  readonly data?: Record<string, unknown>;
  /** A `setup` category, whose actions the notification shows. */
  readonly category?: string;
  /** Android only: a `setup` channel. Notifications that name none use `defaultChannel`. */
  readonly channel?: string;
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

export interface DismissedNotification extends Message {
  readonly source: "local" | "push";
}

export interface OpenedNotification extends DismissedNotification {
  /** The action the user chose, or null for the notification itself. */
  readonly action: string | null;
  /** The text the user typed into the action, or null. */
  readonly input: string | null;
}

/** An action shown with each notification of a category. */
export interface NotificationAction {
  readonly id: string;
  readonly title: string;
  /**
   * A text field, whose text arrives as the opened notification's `input`. Chrome is the only
   * browser with one; others show the action as a button.
   */
  readonly input?: {
    /** Apple only: the send button's title. */
    readonly button?: string;
    readonly placeholder?: string;
  };
  /** Opens the app. Otherwise the action reaches listeners without opening it. */
  readonly foreground?: boolean;
  /** Apple only: shows the action as destructive. */
  readonly destructive?: boolean;
  /** Apple only: requires the device to be unlocked. */
  readonly authenticationRequired?: boolean;
  /** Apple only: an SF Symbol shown with the action. */
  readonly systemImage?: string;
}

/** The actions of the notifications that name the category, with Apple's options for them. */
export interface Category {
  readonly id: string;
  readonly actions: readonly NotificationAction[];
  /** Apple only: replaces the body while previews are hidden. */
  readonly hiddenPreviewsBodyPlaceholder?: string;
  /** Apple only: shows the title while previews are hidden. */
  readonly hiddenPreviewsShowTitle?: boolean;
  /** Apple only: shows the subtitle while previews are hidden. */
  readonly hiddenPreviewsShowSubtitle?: boolean;
  /** Apple only: summarises a stack of the category's notifications, such as "%u more messages". */
  readonly summaryFormat?: string;
  /** Apple only: reports dismissals of the category's notifications. */
  readonly customDismissAction?: boolean;
  /** iOS only: shows the category's notifications in CarPlay. */
  readonly allowInCarPlay?: boolean;
}

/** The parts of a notification Apple platforms show while the app is on screen. */
export interface ForegroundPresentation {
  readonly banner?: boolean;
  readonly list?: boolean;
  readonly sound?: boolean;
  readonly badge?: boolean;
}

/** An Android notification channel's importance. */
export type Importance = "none" | "min" | "low" | "default" | "high";

/** An Android notification channel's settings. */
export interface ChannelSettings {
  readonly name: string;
  readonly description?: string;
  /** A `channelGroups` id. */
  readonly group?: string;
  /** Defaults to `"default"`. */
  readonly importance?: Importance;
  /** `"default"`, `"none"`, or the name of a raw resource the app ships. */
  readonly sound?: string;
  /** Whether to vibrate, or a non-empty pattern of whole milliseconds alternating off and on. */
  readonly vibration?: boolean | readonly number[];
  /** The light's colour as `#rrggbb`, such as `"#00ff00"`, or false. */
  readonly lights?: string | false;
  readonly showBadge?: boolean;
  /** Takes effect only for apps the user gave Do Not Disturb access. */
  readonly bypassDnd?: boolean;
}

export interface Channel extends ChannelSettings {
  readonly id: string;
}

export interface ChannelGroup {
  readonly id: string;
  readonly name: string;
  readonly description?: string;
}

/**
 * The notification configuration of every platform; each ignores the keys of others. `setup`
 * applies the whole of it, removing what an earlier configuration had and this one leaves out.
 */
export interface SetupConfig {
  readonly categories?: readonly Category[];
  /**
   * Whether notifications show while the app is on screen, true by default. Apple platforms show
   * the parts an object names; the web always shows notifications.
   */
  readonly showInForeground?: boolean | ForegroundPresentation;
  /** Android only: the channel of notifications that name none, named after the app by default. */
  readonly defaultChannel?: ChannelSettings;
  /** Android only. */
  readonly channelGroups?: readonly ChannelGroup[];
  /** Android only. */
  readonly channels?: readonly Channel[];
}

/**
 * Settings of an existing Android channel `setup` could not change: Android keeps most of them
 * as first created or as the user changed them, so a new behaviour needs a new channel id.
 */
export interface Unapplied {
  readonly channel: string;
  readonly fields: readonly string[];
}

export interface SetupResult {
  readonly unapplied: readonly Unapplied[];
}

/** Where the app's server sends push messages for this installation. */
export type Subscription =
  | {
      readonly service: "apns";
      readonly token: string;
      readonly environment: "development" | "production";
    }
  /** FCM sends to the Firebase Installation ID, as an HTTP v1 message's `fid`. */
  | { readonly service: "fcm"; readonly fid: string }
  | {
      readonly service: "webpush";
      readonly endpoint: string;
      readonly keys: { readonly p256dh: string; readonly auth: string };
    };

export interface SubscribeOptions {
  /** Web Push only: the app's VAPID public key, base64url-encoded. */
  readonly applicationServerKey?: string;
}

export type Listener<T> = (value: T) => void;
export type ErrorListener = (error: DOMException) => void;

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

  /** Asks the user for permission to show notifications. */
  requestPermission(): Promise<Permission> {
    return this.hasNativeTransport ? this.call("requestPermission") : web.requestPermission();
  }

  /** Shows a notification now. */
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
    return this.hasNativeTransport ? this.call("subscribe") : web.subscribe(options);
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

  /**
   * Receives notifications the user dismissed. Apple platforms report only those of categories
   * with `customDismissAction`.
   */
  onNotificationDismissed(
    listener: Listener<DismissedNotification>,
    error: ErrorListener = report,
  ): () => void {
    if (!this.hasNativeTransport) return web.onNotificationDismissed(listener);
    return this.listen("onNotificationDismissed", listener, error);
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
