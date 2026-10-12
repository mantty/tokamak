import { Plugin } from "@tokamakdev/plugin";
import * as web from "../web/index.js";
import { validateContent } from "./content.js";
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
  /** Apple and Android: shows the notifications that name the same group together. */
  readonly group?: string;
  /**
   * An image, from an `https` URL or a path the app serves: Apple attaches it, Android shows it in
   * the big picture style, and the web shows it. A failed download shows the notification without
   * it.
   */
  readonly image?: string;
  /** Android and web: a large icon, from an `https` URL or a path the app serves. */
  readonly icon?: string;
  /** Apple only: images, audio and movies shown with the notification, after `image`. */
  readonly attachments?: readonly Attachment[];
  /** The app's badge count once the notification shows. */
  readonly badge?: number;
  /** Shows the notification without sound or vibration. */
  readonly silent?: boolean;
  /** Android and web: the time the notification shows, in milliseconds since the epoch. */
  readonly timestamp?: number;
  /** Apple only. */
  readonly subtitle?: string;
  /**
   * Apple only: how urgently the notification interrupts. `timeSensitive` and `critical` need
   * their entitlements.
   */
  readonly interruptionLevel?: "passive" | "active" | "timeSensitive" | "critical";
  /** Apple only: from 0 to 1, how the notification ranks in the notification summary. */
  readonly relevanceScore?: number;
  /** Android only: a line shown with the app's name. */
  readonly subText?: string;
  /** Android only: the accent colour, as `#rrggbb`. */
  readonly color?: string;
  /** Android only. */
  readonly style?: AndroidStyle;
  /** Android only: a progress bar, which `indeterminate` animates without a value. */
  readonly progress?:
    | { readonly current: number; readonly max: number }
    | { readonly indeterminate: true };
  /** Android only: keeps the notification from being dismissed; from Android 14 the user can still swipe it away on an unlocked device. */
  readonly ongoing?: boolean;
  /** Android only: whether opening the notification removes it; true by default. */
  readonly autoCancel?: boolean;
  /** Android only: a replacement with the same id does not alert again. */
  readonly onlyAlertOnce?: boolean;
  /** Android only: removes the notification after this many milliseconds. */
  readonly timeoutAfter?: number;
  /** Android only: shows the time since `timestamp`, or until it with `countDown`. */
  readonly chronometer?: boolean | { readonly countDown: boolean };
  /** Android only: how much of the notification shows on the lock screen. */
  readonly visibility?: "public" | "private" | "secret";
  /** Android only: what the lock screen shows instead, with `visibility` `"private"`. */
  readonly publicVersion?: { readonly title: string; readonly body?: string };
  /** Android only: orders the notification among others of its group. */
  readonly sortKey?: string;
  /** Android only: what kind of notification this is, which ranking and Do Not Disturb use. */
  readonly androidCategory?: AndroidCategory;
  /** Web only: the notification stays until the user acts on it. */
  readonly requireInteraction?: boolean;
  /** Web only: a replacement with the same id alerts again. */
  readonly renotify?: boolean;
  /** Web only: a vibration pattern in milliseconds, alternating on and off. */
  readonly vibrate?: readonly number[];
  /** Web only: the text direction. */
  readonly dir?: "auto" | "ltr" | "rtl";
  /** Web only: the language of the text, as a BCP 47 tag. */
  readonly lang?: string;
  /** Web only: the URL of a small monochrome image that represents the app. */
  readonly badgeImage?: string;
}

/** A file Apple platforms show with a notification. */
export interface Attachment {
  /**
   * An `https` URL or a path the app serves: an image up to 10 MB, audio up to 5 MB or a movie up
   * to 50 MB.
   */
  readonly url: string;
  readonly hideThumbnail?: boolean;
  /** The part of an image the thumbnail shows, as fractions of its width and height. */
  readonly thumbnailClippingRect?: {
    readonly x: number;
    readonly y: number;
    readonly width: number;
    readonly height: number;
  };
  /** The time, in seconds, of the frame of a movie or animated image the thumbnail shows. */
  readonly thumbnailTime?: number;
}

/** An Android notification style; Android shows the collapsed notification without it. */
export type AndroidStyle =
  | {
      readonly type: "bigText";
      readonly text: string;
      readonly title?: string;
      readonly summary?: string;
    }
  | {
      /** Shows `image`; the collapsed notification shows it too with `showWhenCollapsed`. */
      readonly type: "bigPicture";
      readonly title?: string;
      readonly summary?: string;
      readonly showWhenCollapsed?: boolean;
    }
  | {
      readonly type: "inbox";
      readonly lines: readonly string[];
      readonly title?: string;
      readonly summary?: string;
    }
  | {
      readonly type: "messaging";
      /** The name of the device's user, who sends the messages that name no sender. */
      readonly user: string;
      /** A group conversation's title. */
      readonly conversationTitle?: string;
      readonly messages: readonly {
        readonly text: string;
        /** Milliseconds since the epoch. */
        readonly at: number;
        readonly sender?: string;
      }[];
    };

/** The kinds of Android notification, as `Notification.CATEGORY_*` names them. */
export type AndroidCategory =
  | "alarm"
  | "call"
  | "email"
  | "error"
  | "event"
  | "locationSharing"
  | "message"
  | "missedCall"
  | "navigation"
  | "progress"
  | "promo"
  | "recommendation"
  | "reminder"
  | "service"
  | "social"
  | "status"
  | "stopwatch"
  | "system"
  | "transport"
  | "workout";

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
  /**
   * Web and Android: an image for the action, from an `https` URL or a path the app serves. Android
   * shows it on wearables only.
   */
  readonly icon?: string;
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

  /** Shows a notification now. Rejects with `TypeError`, showing nothing, when a value is invalid. */
  async show(notification: NotificationContent): Promise<void> {
    validateContent(notification, false);
    return this.hasNativeTransport ? this.call("show", notification) : web.show(notification);
  }

  /** Shows a notification at `at`, or now when `at` has passed. Not supported on the web. */
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
