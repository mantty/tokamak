package com.tokamak.plugins.notifications

import com.tokamak.runtime.TokamakTag
import org.json.JSONObject

/** Whether the app may show notifications. */
enum class Permission { Granted, Denied, Prompt }

/** A channel's importance. */
enum class Importance { None, Min, Low, Default, High }

data class ChannelImportance(val id: String, val importance: Importance)

data class NotificationSettings(
    val permission: Permission,
    /** Each channel's current importance, which the user may have changed. */
    val channels: List<ChannelImportance>,
    /** The channel groups the user blocked. */
    val blockedChannelGroups: List<String>,
    val doNotDisturbAccess: Boolean,
    /** Whether the app may schedule exact alarms, which `exact` triggers use. */
    val exactAlarms: Boolean,
    val fullScreenIntent: Boolean,
)

/** Settings of an existing channel `setup` could not change: Android keeps most as first created. */
data class Unapplied(val channel: String, val fields: List<String>)

data class SetupResult(val unapplied: List<Unapplied>)

/** Which settings `openSettings` opens: the app's notification settings when none is named. */
data class OpenSettingsOptions(
    val channel: String?,
    val exactAlarms: Boolean?,
    val fullScreenIntent: Boolean?,
    val doNotDisturbAccess: Boolean?,
)

enum class PushService { Fcm }

/** Where the app's server sends push messages for this installation: its Firebase Installation ID. */
data class Subscription(val service: PushService, val fid: String)

/** A received push message. A data-only message has no title or body. */
data class Message(val id: String, val title: String?, val body: String?, val data: JSONObject)

enum class Source { Local, Push }

data class DismissedNotification(val id: String, val title: String?, val body: String?, val data: JSONObject, val source: Source)

data class OpenedNotification(
    val id: String,
    val title: String?,
    val body: String?,
    val data: JSONObject,
    val source: Source,
    /** The action the user chose, or null for the notification itself. */
    val action: String? = null,
    /** The text the user typed into the action. */
    val input: String? = null,
)

/** The user asked for the app's notification settings from the system's. */
data class OpenSettingsEvent(
    /** The channel whose settings the user was in. */
    val channel: String?,
    /** The id of the notification the user came from. */
    val notification: String?,
)

/** A notification's fields that Android shows. */
data class NotificationContent(
    /** Showing or scheduling another notification with the same id replaces this one. */
    val id: String,
    val title: String,
    val body: String? = null,
    val data: JSONObject? = null,
    /** A `setup` category, whose actions the notification shows. */
    val category: String? = null,
    /** A `setup` channel; the default channel when null. */
    val channel: String? = null,
    /** Shows the notifications that name the same group together. */
    val group: String? = null,
    /** An image, from an `https` URL or a path the app serves, shown in the big picture style. */
    val image: String? = null,
    /** A large icon, from an `https` URL or a path the app serves. */
    val icon: String? = null,
    /** The app's badge count once the notification shows. */
    val badge: Int? = null,
    /** Shows the notification without sound or vibration. */
    val silent: Boolean? = null,
    /** The time the notification shows, in milliseconds since the epoch. */
    val timestamp: Long? = null,
    /** A line shown with the app's name. */
    val subText: String? = null,
    /** The accent colour, as `#rrggbb`. */
    val color: String? = null,
    val style: Style? = null,
    val progress: Progress? = null,
    /** Keeps the notification from being dismissed. */
    val ongoing: Boolean? = null,
    /** Whether opening the notification removes it; true by default. */
    val autoCancel: Boolean? = null,
    /** A replacement with the same id does not alert again. */
    val onlyAlertOnce: Boolean? = null,
    /** Removes the notification after this many milliseconds. */
    val timeoutAfter: Long? = null,
    val chronometer: Chronometer? = null,
    /** How much of the notification shows on the lock screen. */
    val visibility: Visibility? = null,
    /** What the lock screen shows instead, with [visibility] private. */
    val publicVersion: PublicVersion? = null,
    /** Orders the notification among others of its group. */
    val sortKey: String? = null,
    /** What kind of notification this is, which ranking and Do Not Disturb use. */
    val androidCategory: AndroidCategory? = null,
    /** The text of the long-press menu's link to the app's notification settings. */
    val settingsText: String? = null,
)

/** An Android notification style, which shows the expanded notification. */
@TokamakTag("type")
sealed interface Style {
    data class BigText(val text: String, val title: String?, val summary: String?) : Style

    /** Shows the notification's `image`; the collapsed notification shows it too with [showWhenCollapsed]. */
    data class BigPicture(val title: String?, val summary: String?, val showWhenCollapsed: Boolean?) : Style

    data class Inbox(val lines: List<String>, val title: String?, val summary: String?) : Style

    /** A conversation; a message without a sender is the device's user's. */
    data class Messaging(val user: String, val conversationTitle: String?, val messages: List<StyleMessage>) : Style
}

data class StyleMessage(
    val text: String,
    /** Milliseconds since the epoch. */
    val at: Long,
    val sender: String?,
)

/** A progress bar: [ProgressValue.current] of [ProgressValue.max], or an animation without a value. */
sealed interface Progress {
    data class ProgressValue(val current: Int, val max: Int) : Progress

    data class IndeterminateProgress(val indeterminate: Boolean) : Progress
}

/** A chronometer, which counts from `timestamp`, or down to it with [Countdown.countDown]. */
sealed interface Chronometer {
    @JvmInline value class Shown(val value: Boolean) : Chronometer

    data class Countdown(val countDown: Boolean) : Chronometer
}

enum class Visibility { Public, Private, Secret }

data class PublicVersion(val title: String, val body: String?)

/** The kinds of notification, as `Notification.CATEGORY_*` names them. */
enum class AndroidCategory {
    Alarm, Call, Email, Error, Event, LocationSharing, Message, MissedCall, Navigation, Progress,
    Promo, Recommendation, Reminder, Service, Social, Status, Stopwatch, System, Transport, Workout,
}

/** When a scheduled notification shows; [exact] alarms need the exact alarm permission. */
sealed interface Trigger {
    val exact: Boolean?

    /** At [at], in milliseconds since the epoch, or now once it has passed. */
    data class AtTrigger(val at: Long, override val exact: Boolean?) : Trigger

    /** Once each interval, the first time one interval from when it was scheduled. */
    data class EveryTrigger(val every: Interval, override val exact: Boolean?) : Trigger

    /** At the next local time [on] matches, or at every match with [repeats]. */
    data class DateTrigger(val on: DateMatch, val repeats: Boolean?, override val exact: Boolean?) : Trigger
}

/** A repeating interval of at least 60 seconds, the sum of its parts. */
data class Interval(val seconds: Double?, val minutes: Double?, val hours: Double?, val days: Double?)

/** Parts of a local date and time; the parts left out match any value. */
data class DateMatch(
    val year: Int?,
    /** 1 for January to 12. */
    val month: Int?,
    val day: Int?,
    /** 1 for Sunday to 7 for Saturday. */
    val weekday: Int?,
    val hour: Int?,
    val minute: Int?,
    val second: Int?,
)

/** A notification not yet shown and its trigger. */
data class ScheduledNotification(val content: NotificationContent, val trigger: Trigger)

/** The configuration Android applies. */
data class SetupConfig(
    val categories: List<Category>?,
    /** Whether notifications show while the app is on screen; true by default, as with an object. */
    val showInForeground: ShowInForeground?,
    /** The channel of notifications that name none, named after the app by default. */
    val defaultChannel: ChannelSettings?,
    val channelGroups: List<ChannelGroup>?,
    val channels: List<ChannelSettings>?,
)

sealed interface ShowInForeground {
    @JvmInline value class All(val value: Boolean) : ShowInForeground

    /** The parts Apple platforms show, which Android shows in full. */
    data class ForegroundPresentation(val banner: Boolean?, val list: Boolean?, val sound: Boolean?, val badge: Boolean?) : ShowInForeground
}

/** The actions of the notifications that name the category. */
data class Category(val id: String, val actions: List<NotificationAction>)

data class NotificationAction(
    val id: String,
    val title: String,
    /** A text field, whose text arrives as the opened notification's `input`. */
    val input: ActionInput?,
    /** Opens the app; otherwise the action reaches listeners without opening it. */
    val foreground: Boolean?,
    /** An image for wearables, from an `https` URL or a path the app serves. */
    val icon: String?,
)

data class ActionInput(val placeholder: String?)

/** A notification channel's settings. */
data class ChannelSettings(
    /** A `channels` entry's id; the default channel has none. */
    val id: String?,
    val name: String,
    val description: String?,
    /** A `channelGroups` id. */
    val group: String?,
    /** Defaults to `default`. */
    val importance: Importance?,
    /** `"default"`, `"none"`, or a file of the `notifications.sounds` build setting. */
    val sound: String?,
    val vibration: Vibration?,
    /** The light's colour as `#rrggbb`, or false for none. */
    val lights: Lights?,
    val showBadge: Boolean?,
    /** Takes effect only for apps the user gave Do Not Disturb access. */
    val bypassDnd: Boolean?,
)

/** Whether a channel vibrates, or a non-empty pattern of milliseconds alternating off and on. */
sealed interface Vibration {
    @JvmInline value class Enabled(val value: Boolean) : Vibration

    @JvmInline value class Pattern(val value: List<Long>) : Vibration
}

sealed interface Lights {
    @JvmInline value class Colour(val value: String) : Lights

    @JvmInline value class Shown(val value: Boolean) : Lights
}

data class ChannelGroup(val id: String, val name: String, val description: String?)
