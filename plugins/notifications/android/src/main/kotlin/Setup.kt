package com.tokamak.plugins.notifications

import android.app.NotificationChannel
import android.app.NotificationChannelGroup
import android.content.ContentResolver
import android.content.Context
import android.graphics.Color
import android.media.AudioAttributes
import android.net.Uri
import android.provider.Settings
import com.tokamak.runtime.TokamakHost
import com.tokamak.runtime.TokamakPluginError
import org.json.JSONObject

private const val PREFERENCES = "tokamak.notifications.setup"
private const val CONFIG = "config"

/** The configuration `setup` applied, checked in full before anything applies it. */
internal class Setup(val config: SetupConfig) {
    val categories: Map<String, List<NotificationAction>> =
        config.categories.orEmpty().associate { it.id to it.actions }

    /** Whether notifications show while the app is on screen; an object means they do. */
    val showInForeground: Boolean = (config.showInForeground as? ShowInForeground.All)?.value ?: true

    val defaultChannel: ChannelSpec? = config.defaultChannel?.let { ChannelSpec(CHANNEL, it) }
    val channelGroups: List<NotificationChannelGroup> =
        config.channelGroups.orEmpty().map { group ->
            NotificationChannelGroup(group.id, group.name).apply { description = group.description }
        }
    val channels: List<ChannelSpec> =
        config.channels.orEmpty().mapIndexed { index, channel ->
            ChannelSpec(channel.id ?: throw TokamakPluginError.typeError("channels[$index].id is missing"), channel)
        }

    /** Throws `NotFoundError` unless the app bundles each channel's sound. */
    fun requireSounds(context: Context) {
        (listOfNotNull(defaultChannel) + channels).forEach { it.requireSound(context) }
    }

    fun store(host: TokamakHost) {
        preferences(host).edit().putString(CONFIG, host.json(config).toString()).apply()
    }

    companion object {
        /** The configuration `setup` last applied, or null before any. */
        fun stored(host: TokamakHost): Setup? =
            preferences(host)
                .getString(CONFIG, null)
                ?.let { runCatching { Setup(host.fromJson(JSONObject(it), SetupConfig::class)) }.getOrNull() }

        private fun preferences(host: TokamakHost) = host.context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
    }
}

/** An Android notification channel `setup` describes. */
internal class ChannelSpec(val id: String, private val settings: ChannelSettings) {
    private val importance = (settings.importance ?: Importance.Default).value

    /** A non-empty pattern, which turns vibration on. */
    private val vibrationPattern: LongArray? = (settings.vibration as? Vibration.Pattern)?.value?.toLongArray()
    private val vibrate: Boolean? = if (vibrationPattern != null) true else (settings.vibration as? Vibration.Enabled)?.value

    /** The light's colour; null when the light is off or unspecified. */
    private val lightColor: Int? = (settings.lights as? Lights.Colour)?.value?.let(Color::parseColor)
    private val showLights: Boolean? =
        when (val lights = settings.lights) {
            null -> null
            is Lights.Colour -> true
            is Lights.Shown -> lights.value
        }

    fun channel(context: Context): NotificationChannel =
        NotificationChannel(id, settings.name, importance).also { channel ->
            channel.description = settings.description
            channel.group = settings.group
            settings.sound?.let { channel.setSound(soundUri(context, it), AUDIO) }
            vibrate?.let(channel::enableVibration)
            vibrationPattern?.let { channel.vibrationPattern = it }
            showLights?.let(channel::enableLights)
            lightColor?.let { channel.lightColor = it }
            settings.showBadge?.let(channel::setShowBadge)
            settings.bypassDnd?.let(channel::setBypassDnd)
        }

    /**
     * Throws `NotFoundError` unless the app bundles the channel's sound: `"default"`, `"none"`, or a
     * file of the `sounds` build setting, which the app has as a raw resource.
     */
    fun requireSound(context: Context) {
        val file = settings.sound?.takeIf { it != "default" && it != "none" } ?: return
        if (context.resources.getIdentifier(resourceName(file), "raw", context.packageName) == 0) {
            throw TokamakPluginError("NotFoundError", "The app has no sound $file; add it to the notifications.sounds build setting")
        }
    }

    /** The settings this spec names that [existing] does not have, which Android kept. */
    fun unapplied(existing: NotificationChannel, context: Context): List<String> =
        buildList {
            if (existing.importance != importance) add("importance")
            if (settings.group != null && existing.group != settings.group) add("group")
            if (settings.sound != null && existing.sound != soundUri(context, settings.sound)) add("sound")
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
            if (settings.showBadge != null && existing.canShowBadge() != settings.showBadge) add("showBadge")
            if (settings.bypassDnd != null && existing.canBypassDnd() != settings.bypassDnd) add("bypassDnd")
        }

    private fun soundUri(context: Context, sound: String): Uri? =
        when (sound) {
            "default" -> Settings.System.DEFAULT_NOTIFICATION_URI
            "none" -> null
            else -> Uri.parse("${ContentResolver.SCHEME_ANDROID_RESOURCE}://${context.packageName}/raw/${resourceName(sound)}")
        }

    /** The raw resource the build makes of the sound file [file]: its name without the extension. */
    private fun resourceName(file: String): String = file.substringBeforeLast('.')

    private companion object {
        val AUDIO: AudioAttributes =
            AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_NOTIFICATION)
                .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                .build()
    }
}
