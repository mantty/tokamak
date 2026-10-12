package com.tokamak.plugins.notifications

import android.app.Notification
import android.graphics.Color
import com.tokamak.runtime.TokamakPluginError
import org.json.JSONObject

/**
 * A notification the page asked to show or schedule, kept as the JSON it arrived as, which
 * `getScheduled` and `getDelivered` return. Every field is read when it arrives, so a field of
 * the wrong type, as in a Worker's `onPush` reply, throws `TypeError` then.
 */
internal class Content private constructor(val json: JSONObject, scheduled: Boolean) {
    val id: String = requireString(json, "id").takeIf { it.isNotEmpty() }
        ?: throw TokamakPluginError.typeError("id must be a non-empty string")
    val title: String = requireString(json, "title")
    val body: String? = optionalString(json, "body")
    val data: JSONObject = optionalObject(json, "data") ?: JSONObject()

    /** When a scheduled notification shows; null to show now. */
    val trigger: Trigger? = if (scheduled) Trigger.parse(json) else null

    /** A `setup` category, whose actions the notification shows. */
    val category: String? = optionalString(json, "category")

    /** A `setup` channel; null for the default channel. */
    val channel: String? = optionalString(json, "channel")
    val group: String? = optionalString(json, "group")

    /** An `https` URL or a path the app serves. */
    val image: String? = optionalString(json, "image")
    val icon: String? = optionalString(json, "icon")
    val badge: Int? = optionalNumber(json, "badge")?.toInt()
    val silent: Boolean = optionalBoolean(json, "silent") ?: false
    val timestamp: Long? = optionalNumber(json, "timestamp")?.toLong()
    val subText: String? = optionalString(json, "subText")
    val color: Int? =
        optionalString(json, "color")?.let {
            runCatching { Color.parseColor(it) }.getOrElse {
                throw TokamakPluginError.typeError("color must be a colour such as #00ff00")
            }
        }
    val style: Style? = optionalObject(json, "style")?.let(Style::parse)
    val progress: Progress? = optionalObject(json, "progress")?.let(Progress::parse)
    val ongoing: Boolean = optionalBoolean(json, "ongoing") ?: false
    val autoCancel: Boolean = optionalBoolean(json, "autoCancel") ?: true
    val onlyAlertOnce: Boolean = optionalBoolean(json, "onlyAlertOnce") ?: false
    val timeoutAfter: Long? = optionalNumber(json, "timeoutAfter")?.toLong()

    /** Whether the notification shows a chronometer, which counts from `timestamp`. */
    val chronometer: Boolean
    val chronometerCountsDown: Boolean

    init {
        when (val chronometer = json.opt("chronometer")) {
            null, is Boolean -> {
                this.chronometer = chronometer == true
                chronometerCountsDown = false
            }
            is JSONObject -> {
                this.chronometer = true
                chronometerCountsDown = optionalBoolean(chronometer, "countDown") ?: false
            }
            else -> throw TokamakPluginError.typeError("chronometer must be a boolean or an object")
        }
    }

    val visibility: Int? =
        when (optionalString(json, "visibility")) {
            null -> null
            "public" -> Notification.VISIBILITY_PUBLIC
            "private" -> Notification.VISIBILITY_PRIVATE
            "secret" -> Notification.VISIBILITY_SECRET
            else -> throw TokamakPluginError.typeError("visibility must be public, private or secret")
        }

    /** The title and body the lock screen shows with `visibility` private. */
    val publicVersion: Pair<String, String?>? =
        optionalObject(json, "publicVersion")?.let { requireString(it, "title") to optionalString(it, "body") }
    val sortKey: String? = optionalString(json, "sortKey")

    /** The text of the long-press menu's link to the app's notification settings. */
    val settingsText: String? = optionalString(json, "settingsText")
    val androidCategory: String? =
        optionalString(json, "androidCategory")?.let {
            CATEGORIES[it] ?: throw TokamakPluginError.typeError("$it is not an androidCategory")
        }

    companion object {
        /** `Notification.CATEGORY_*` by the name the page gives each. */
        private val CATEGORIES =
            mapOf(
                "alarm" to Notification.CATEGORY_ALARM,
                "call" to Notification.CATEGORY_CALL,
                "email" to Notification.CATEGORY_EMAIL,
                "error" to Notification.CATEGORY_ERROR,
                "event" to Notification.CATEGORY_EVENT,
                "locationSharing" to Notification.CATEGORY_LOCATION_SHARING,
                "message" to Notification.CATEGORY_MESSAGE,
                "missedCall" to Notification.CATEGORY_MISSED_CALL,
                "navigation" to Notification.CATEGORY_NAVIGATION,
                "progress" to Notification.CATEGORY_PROGRESS,
                "promo" to Notification.CATEGORY_PROMO,
                "recommendation" to Notification.CATEGORY_RECOMMENDATION,
                "reminder" to Notification.CATEGORY_REMINDER,
                "service" to Notification.CATEGORY_SERVICE,
                "social" to Notification.CATEGORY_SOCIAL,
                "status" to Notification.CATEGORY_STATUS,
                "stopwatch" to Notification.CATEGORY_STOPWATCH,
                "system" to Notification.CATEGORY_SYSTEM,
                "transport" to Notification.CATEGORY_TRANSPORT,
                "workout" to Notification.CATEGORY_WORKOUT,
            )

        fun parse(arguments: Any?, scheduled: Boolean): Content {
            val json = arguments as? JSONObject ?: throw TokamakPluginError.typeError("a notification must be an object")
            return Content(json, scheduled)
        }

        /** A notification the schedule stored. */
        fun fromJson(json: JSONObject): Content = parse(json, scheduled = true)
    }
}

/** An Android notification style, which shows the expanded notification. */
internal sealed interface Style {
    class BigText(val text: String, val title: String?, val summary: String?) : Style

    /** Shows the notification's `image`. */
    class BigPicture(val title: String?, val summary: String?, val showWhenCollapsed: Boolean) : Style

    class Inbox(val lines: List<String>, val title: String?, val summary: String?) : Style

    /** A conversation; a message without a sender is the device's user's. */
    class Messaging(val user: String, val conversationTitle: String?, val messages: List<Message>) : Style

    class Message(val text: String, val at: Long, val sender: String?)

    companion object {
        fun parse(json: JSONObject): Style =
            when (val type = requireString(json, "type")) {
                "bigText" -> BigText(requireString(json, "text"), optionalString(json, "title"), optionalString(json, "summary"))
                "bigPicture" ->
                    BigPicture(optionalString(json, "title"), optionalString(json, "summary"), optionalBoolean(json, "showWhenCollapsed") ?: false)
                "inbox" -> Inbox(strings(json, "lines"), optionalString(json, "title"), optionalString(json, "summary"))
                "messaging" ->
                    Messaging(
                        requireString(json, "user"),
                        optionalString(json, "conversationTitle"),
                        objects(json, "messages").map {
                            Message(requireString(it, "text"), requireNumber(it, "at").toLong(), optionalString(it, "sender"))
                        },
                    )
                else -> throw TokamakPluginError.typeError("$type is not a style")
            }
    }
}

/** A progress bar: [current] of [max], or an animation without a value when [indeterminate]. */
internal class Progress(val current: Int, val max: Int, val indeterminate: Boolean) {
    companion object {
        fun parse(json: JSONObject): Progress =
            if (optionalBoolean(json, "indeterminate") == true) {
                Progress(0, 0, true)
            } else {
                Progress(requireNumber(json, "current").toInt(), requireNumber(json, "max").toInt(), false)
            }
    }
}
