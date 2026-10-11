package com.tokamak.plugins.notifications

import android.app.NotificationChannel
import android.app.NotificationChannelGroup
import android.app.NotificationManager
import android.content.ContentResolver
import android.content.Context
import android.graphics.Color
import android.media.AudioAttributes
import android.net.Uri
import android.provider.Settings
import com.tokamak.runtime.TokamakPluginError
import org.json.JSONArray
import org.json.JSONObject

private const val PREFERENCES = "tokamak.notifications.setup"
private const val CONFIG = "config"

/** An action of a `setup` category; [inputLabel] labels its text field, if it has one. */
internal class Action(val id: String, val title: String, val inputLabel: String?, val foreground: Boolean)

/** The configuration `setup` applied, parsed in full before anything applies it. */
internal class Setup private constructor(val json: JSONObject) {
    val categories: Map<String, List<Action>> =
        objects(json, "categories").associate { category ->
            requireString(category, "id") to objects(category, "actions").map(::action)
        }

    /** Whether notifications show while the app is on screen; an object means they do. */
    val showInForeground: Boolean =
        when (val shown = json.opt("showInForeground")) {
            null, is JSONObject -> true
            is Boolean -> shown
            else -> throw TokamakPluginError.typeError("showInForeground must be a boolean or an object")
        }

    val defaultChannel: ChannelSpec? = json.optJSONObject("defaultChannel")?.let { ChannelSpec(CHANNEL, it) }
    val channelGroups: List<NotificationChannelGroup> = objects(json, "channelGroups").map(::channelGroup)
    val channels: List<ChannelSpec> = objects(json, "channels").map { ChannelSpec(requireString(it, "id"), it) }

    fun store(context: Context) {
        context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE).edit().putString(CONFIG, json.toString()).apply()
    }

    companion object {
        /** Throws `TypeError` unless [arguments] is a valid configuration. */
        fun parse(arguments: Any?): Setup =
            Setup(arguments as? JSONObject ?: throw TokamakPluginError.typeError("setup takes a configuration object"))

        /** The configuration `setup` last applied, or null before any. */
        fun stored(context: Context): Setup? =
            context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
                .getString(CONFIG, null)
                ?.let { runCatching { Setup(JSONObject(it)) }.getOrNull() }

        private fun action(json: JSONObject): Action {
            val title = requireString(json, "title")
            return Action(
                requireString(json, "id"),
                title,
                json.optJSONObject("input")?.let { optionalString(it, "placeholder") ?: title },
                optionalBoolean(json, "foreground") ?: false,
            )
        }

        private fun channelGroup(json: JSONObject): NotificationChannelGroup =
            NotificationChannelGroup(requireString(json, "id"), requireString(json, "name")).apply {
                description = optionalString(json, "description")
            }
    }
}

/** An Android notification channel `setup` describes. */
internal class ChannelSpec(val id: String, json: JSONObject) {
    private val name = requireString(json, "name")
    private val description = optionalString(json, "description")
    private val group = optionalString(json, "group")
    private val importance =
        when (optionalString(json, "importance") ?: "default") {
            "none" -> NotificationManager.IMPORTANCE_NONE
            "min" -> NotificationManager.IMPORTANCE_MIN
            "low" -> NotificationManager.IMPORTANCE_LOW
            "default" -> NotificationManager.IMPORTANCE_DEFAULT
            "high" -> NotificationManager.IMPORTANCE_HIGH
            else -> throw TokamakPluginError.typeError("importance must be none, min, low, default or high")
        }

    /** `"default"`, `"none"`, or a raw resource the app ships. */
    private val sound = optionalString(json, "sound")
    /** A non-empty pattern, which turns vibration on. */
    private val vibrationPattern: LongArray? =
        (json.opt("vibration") as? JSONArray)?.let { steps -> LongArray(steps.length()) { steps.getLong(it) } }
    private val vibrate: Boolean? = if (vibrationPattern != null) true else optionalBoolean(json, "vibration")
    /** The light's colour; null when the light is off or unspecified. */
    private val lightColor: Int? =
        when (val lights = json.opt("lights")) {
            null, false -> null
            is String -> runCatching { Color.parseColor(lights) }.getOrElse { throw invalidLights() }
            else -> throw invalidLights()
        }
    private val showLights: Boolean? = if (json.has("lights")) lightColor != null else null
    private val showBadge = optionalBoolean(json, "showBadge")
    private val bypassDnd = optionalBoolean(json, "bypassDnd")

    fun channel(context: Context): NotificationChannel =
        NotificationChannel(id, name, importance).also { channel ->
            channel.description = description
            channel.group = group
            sound?.let { channel.setSound(soundUri(context, it), AUDIO) }
            vibrate?.let(channel::enableVibration)
            vibrationPattern?.let { channel.vibrationPattern = it }
            showLights?.let(channel::enableLights)
            lightColor?.let { channel.lightColor = it }
            showBadge?.let(channel::setShowBadge)
            bypassDnd?.let(channel::setBypassDnd)
        }

    /** The settings this spec names that [existing] does not have, which Android kept. */
    fun unapplied(existing: NotificationChannel, context: Context): List<String> =
        buildList {
            if (existing.importance != importance) add("importance")
            if (group != null && existing.group != group) add("group")
            if (sound != null && existing.sound != soundUri(context, sound)) add("sound")
            if ((vibrate != null && existing.shouldVibrate() != vibrate) ||
                (vibrationPattern != null && !existing.vibrationPattern.contentEquals(vibrationPattern))
            ) {
                add("vibration")
            }
            if ((showLights != null && existing.shouldShowLights() != showLights) ||
                (lightColor != null && existing.lightColor != lightColor)
            ) {
                add("lights")
            }
            if (showBadge != null && existing.canShowBadge() != showBadge) add("showBadge")
            if (bypassDnd != null && existing.canBypassDnd() != bypassDnd) add("bypassDnd")
        }

    private fun soundUri(context: Context, sound: String): Uri? =
        when (sound) {
            "default" -> Settings.System.DEFAULT_NOTIFICATION_URI
            "none" -> null
            else -> Uri.parse("${ContentResolver.SCHEME_ANDROID_RESOURCE}://${context.packageName}/raw/$sound")
        }

    private companion object {
        fun invalidLights() = TokamakPluginError.typeError("lights must be false or a colour such as #00ff00")

        val AUDIO: AudioAttributes =
            AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_NOTIFICATION)
                .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                .build()
    }
}

internal fun objects(json: JSONObject, key: String): List<JSONObject> {
    if (!json.has(key)) return emptyList()
    val array = json.opt(key) as? JSONArray ?: throw TokamakPluginError.typeError("$key must be an array")
    return List(array.length()) {
        array.opt(it) as? JSONObject ?: throw TokamakPluginError.typeError("each of $key must be an object")
    }
}

internal fun requireString(json: JSONObject, key: String): String =
    json.opt(key) as? String ?: throw TokamakPluginError.typeError("$key must be a string")

internal fun optionalString(json: JSONObject, key: String): String? =
    if (json.has(key)) requireString(json, key) else null

internal fun optionalBoolean(json: JSONObject, key: String): Boolean? {
    if (!json.has(key)) return null
    return json.opt(key) as? Boolean ?: throw TokamakPluginError.typeError("$key must be a boolean")
}
