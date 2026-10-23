import type { JsonValue } from "@tokamakdev/plugin";

// Notification content and configuration reach native code as JSON documents. They are type
// aliases, not interfaces, so that TypeScript checks them against `JsonValue`.

/** Whether the app may show notifications. */
export type Permission = "granted" | "denied" | "prompt";

export type NotificationContent = {
  /** Showing or scheduling another notification with the same id replaces this one. */
  readonly id: string;
  readonly title: string;
  readonly body?: string;
  readonly data?: Record<string, JsonValue>;
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
  /**
   * Apple only: the sound played. `name` is a file of the `notifications.sounds` build setting;
   * critical sounds, which play through the mute switch and Do Not Disturb, need the critical
   * alerts entitlement.
   */
  readonly sound?: Sound;
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
  /** Android only: a progress bar, which an `indeterminate` of true animates without a value. */
  readonly progress?:
    | { readonly current: number; readonly max: number }
    | { readonly indeterminate: boolean };
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
  /**
   * Android only: the text of a link in the notification's long-press menu that opens the app's
   * notification settings, which `onOpenSettings` receives.
   */
  readonly settingsText?: string;
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
};

/** An Apple notification sound. */
export type Sound =
  | "default"
  | "defaultCritical"
  | {
      /** A file of the `notifications.sounds` build setting, such as `"chime.caf"`. */
      readonly name: string;
      readonly critical?: boolean;
      /** A critical sound's volume, from 0 to 1. */
      readonly volume?: number;
    };

/** A file Apple platforms show with a notification. */
export type Attachment = {
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
};

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

/** When a scheduled notification shows. */
export type Trigger =
  | {
      /** Milliseconds since the epoch; the earliest time the notification is shown. */
      readonly at: number;
      /** Android only: shows the notification at the time, which needs the exact alarm permission. */
      readonly exact?: boolean;
    }
  | {
      /** Shows the notification once each interval, the first time one interval from now. */
      readonly every: Interval;
      readonly exact?: boolean;
    }
  | {
      /** Shows the notification at the next local time that matches. */
      readonly on: DateMatch;
      /** Shows it at every match. */
      readonly repeats?: boolean;
      readonly exact?: boolean;
    }
  | {
      /** iOS only: shows the notification when the device enters or leaves a region. */
      readonly region: Region;
      readonly repeats?: boolean;
    };

/** A repeating interval of at least 60 seconds, the sum of its parts. */
export type Interval = {
  readonly seconds?: number;
  readonly minutes?: number;
  readonly hours?: number;
  readonly days?: number;
};

/** Parts of a local date and time; the parts left out match any value. */
export type DateMatch = {
  readonly year?: number;
  /** 1 for January to 12. */
  readonly month?: number;
  readonly day?: number;
  /** 1 for Sunday to 7 for Saturday. */
  readonly weekday?: number;
  readonly hour?: number;
  readonly minute?: number;
  readonly second?: number;
};

/** A circle on the Earth's surface. */
export type Region = {
  readonly latitude: number;
  readonly longitude: number;
  /** In metres. */
  readonly radius: number;
  /** Whether entering the region shows the notification; true by default. */
  readonly onEntry?: boolean;
  /** Whether leaving the region shows the notification; false by default. */
  readonly onExit?: boolean;
};

export type ScheduledNotification = NotificationContent & Trigger;

/** A received push message. A data-only message has no title or body. */
export interface Message<Data extends object = Record<string, JsonValue>> {
  readonly id: string;
  readonly title?: string;
  readonly body?: string;
  readonly data: Data;
}

export interface DismissedNotification extends Message {
  readonly source: "local" | "push";
}

export interface OpenedNotification extends DismissedNotification {
  /** The action the user chose; none for the notification itself. */
  readonly action?: string;
  /** The text the user typed into the action. */
  readonly input?: string;
}

/** An action shown with each notification of a category. */
export type NotificationAction = {
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
};

/** The actions of the notifications that name the category, with Apple's options for them. */
export type Category = {
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
};

/** The parts of a notification Apple platforms show while the app is on screen. */
export type ForegroundPresentation = {
  readonly banner?: boolean;
  readonly list?: boolean;
  readonly sound?: boolean;
  readonly badge?: boolean;
};

/** An Android notification channel's importance. */
export type Importance = "none" | "min" | "low" | "default" | "high";

/** An Android notification channel's settings. */
export type ChannelSettings = {
  readonly name: string;
  readonly description?: string;
  /** A `channelGroups` id. */
  readonly group?: string;
  /** Defaults to `"default"`. */
  readonly importance?: Importance;
  /** `"default"`, `"none"`, or a file of the `notifications.sounds` build setting, such as `"chime.wav"`. */
  readonly sound?: string;
  /** Whether to vibrate, or a non-empty pattern of whole milliseconds alternating off and on. */
  readonly vibration?: boolean | readonly number[];
  /** The light's colour as `#rrggbb`, such as `"#00ff00"`, or false. */
  readonly lights?: string | false;
  readonly showBadge?: boolean;
  /** Takes effect only for apps the user gave Do Not Disturb access. */
  readonly bypassDnd?: boolean;
};

export type Channel = ChannelSettings & {
  readonly id: string;
};

export type ChannelGroup = {
  readonly id: string;
  readonly name: string;
  readonly description?: string;
};

/**
 * The notification configuration of every platform; each ignores the keys of others. `setup`
 * applies the whole of it, removing what an earlier configuration had and this one leaves out.
 */
export type SetupConfig = {
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
};

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

/** Apple only: what `requestPermission` asks for. `alert`, `sound` and `badge` are true by default. */
export interface PermissionOptions {
  readonly alert?: boolean;
  readonly sound?: boolean;
  readonly badge?: boolean;
  /** iOS only. */
  readonly carPlay?: boolean;
  /** Needs the critical alerts entitlement. */
  readonly criticalAlert?: boolean;
  /** Grants permission without a prompt; notifications then go quietly to Notification Center. */
  readonly provisional?: boolean;
  /** Shows a link to the app's notification settings in the system's, which `onOpenSettings` receives. */
  readonly providesAppNotificationSettings?: boolean;
}

/** Apple's state of a notification setting. */
export type Setting = "enabled" | "disabled" | "notSupported";

/** The user's notification settings: `permission`, and the platform's detail. */
export interface NotificationSettings {
  readonly permission: Permission;
  /** Apple only. */
  readonly authorizationStatus?: "notDetermined" | "denied" | "authorized" | "provisional" | "ephemeral";
  /** Apple only. */
  readonly alertStyle?: "none" | "banner" | "alert";
  /** Apple only: when notifications show their content. */
  readonly showPreviews?: "always" | "whenAuthenticated" | "never";
  /** Apple only: each setting's state. */
  readonly alert?: Setting;
  readonly sound?: Setting;
  readonly badge?: Setting;
  readonly lockScreen?: Setting;
  readonly notificationCenter?: Setting;
  readonly carPlay?: Setting;
  readonly criticalAlert?: Setting;
  readonly timeSensitive?: Setting;
  readonly scheduledDelivery?: Setting;
  readonly directMessages?: Setting;
  readonly announcement?: Setting;
  /** Apple only: whether the system shows a link to the app's own notification settings. */
  readonly providesAppNotificationSettings?: boolean;
  /** Android only: each channel's current importance, which the user may have changed. */
  readonly channels?: readonly { readonly id: string; readonly importance: Importance }[];
  /** Android only: the channel groups the user blocked. */
  readonly blockedChannelGroups?: readonly string[];
  /** Android only: whether the user gave the app Do Not Disturb access. */
  readonly doNotDisturbAccess?: boolean;
  /** Android only: whether the app may schedule exact alarms, which `exact` triggers use. */
  readonly exactAlarms?: boolean;
  /** Android only: whether the app may show full-screen notifications. */
  readonly fullScreenIntent?: boolean;
}

/** Which notification settings `openSettings` opens: the app's, by default. */
export interface OpenSettingsOptions {
  /** Android only: a channel's settings. */
  readonly channel?: string;
  /** Android only: the setting that allows exact alarms. */
  readonly exactAlarms?: boolean;
  /** Android only: the setting that allows full-screen notifications. */
  readonly fullScreenIntent?: boolean;
  /** Android only: the setting that gives the app Do Not Disturb access. */
  readonly doNotDisturbAccess?: boolean;
}

/** The user asked for the app's notification settings from the system's. */
export interface OpenSettingsEvent {
  /** Android only: the channel whose settings the user was in. */
  readonly channel?: string;
  /** The id of the notification the user came from. */
  readonly notification?: string;
}

export type Listener<T> = (value: T) => void;
export type ErrorListener = (error: DOMException) => void;
