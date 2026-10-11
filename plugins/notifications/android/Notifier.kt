package com.tokamak.plugins.notifications

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.RemoteInput
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.graphics.drawable.Icon
import android.os.Bundle
import com.tokamak.runtime.TokamakPluginError
import com.tokamak.runtime.tokamakPlugin
import org.json.JSONArray
import org.json.JSONObject

internal const val CHANNEL = "tokamak.notifications"
internal const val OPENED_EXTRA = "com.tokamak.notifications.opened"
internal const val DISMISSED_EXTRA = "com.tokamak.notifications.dismissed"
private const val DATA_EXTRA = "com.tokamak.notifications.data"
private const val CATEGORY_EXTRA = "com.tokamak.notifications.category"
private const val INPUT_KEY = "com.tokamak.notifications.input"

/** The plugin and FCM post every notification with this ID, telling them apart by tag. */
private const val NOTIFICATION_ID = 0

/** Posts, lists and removes the app's notifications, and keeps its channels as `setup` describes. */
internal class Notifier(private val context: Context) {
    private val manager = context.getSystemService(NotificationManager::class.java)

    val enabled: Boolean
        get() = manager.areNotificationsEnabled()

    /** Creates the default channel, named after the app, unless it exists. */
    fun ensureDefaultChannel() {
        if (manager.getNotificationChannel(CHANNEL) == null) manager.createNotificationChannel(appChannel())
    }

    /**
     * Makes the channel groups and channels [setup] describes, removing those of [previous] it
     * leaves out, and returns the settings of existing channels Android kept.
     */
    fun configure(setup: Setup, previous: Setup?): JSONArray {
        val groups = setup.channelGroups.map { it.id }.toSet()
        previous?.channelGroups?.map { it.id }?.filterNot(groups::contains)?.forEach(manager::deleteNotificationChannelGroup)
        manager.createNotificationChannelGroups(setup.channelGroups)
        val channels = setup.channels.map { it.id }.toSet()
        previous?.channels?.map { it.id }?.filterNot(channels::contains)?.forEach(manager::deleteNotificationChannel)
        if (setup.defaultChannel == null) manager.createNotificationChannel(appChannel())
        val unapplied = JSONArray()
        for (spec in listOfNotNull(setup.defaultChannel) + setup.channels) {
            manager.createNotificationChannel(spec.channel(context))
            val kept = spec.unapplied(manager.getNotificationChannel(spec.id), context)
            if (kept.isNotEmpty()) unapplied.put(JSONObject().put("channel", spec.id).put("fields", JSONArray(kept)))
        }
        return unapplied
    }

    /** Throws `TypeError` unless [content] names a channel that exists. */
    fun requireChannel(content: Content) {
        val channel = content.channel ?: return
        if (manager.getNotificationChannel(channel) == null) {
            throw TokamakPluginError.typeError("channel $channel is not a setup channel")
        }
    }

    /**
     * Shows [content], replacing a notification with the same ID, with its category's actions.
     * Opening it launches the app. A channel `setup` has since removed falls back to the default.
     */
    fun post(content: Content, source: String) {
        ensureDefaultChannel()
        val channel = content.channel?.takeIf { manager.getNotificationChannel(it) != null } ?: CHANNEL
        val builder =
            Notification.Builder(context, channel)
                .setSmallIcon(icon())
                .setContentTitle(content.title)
                .setContentText(content.body)
                .setAutoCancel(true)
                .setContentIntent(
                    launch("opened\n${content.id}", described(content, source).put("action", JSONObject.NULL), mutable = false),
                )
                .setDeleteIntent(
                    respond("dismissed\n${content.id}", Intent().putExtra(DISMISSED_EXTRA, described(content, source).toString())),
                )
                .addExtras(
                    Bundle().apply {
                        putString(DATA_EXTRA, content.data.toString())
                        putString(CATEGORY_EXTRA, content.category)
                    },
                )
        val actions = content.category?.let { Setup.stored(context)?.categories?.get(it) }.orEmpty()
        for (action in actions) builder.addAction(action(content, source, action))
        manager.notify(content.id, NOTIFICATION_ID, builder.build())
    }

    /** The app's shown notifications, identified by tag, as the page sees them. */
    fun delivered(): List<JSONObject> =
        manager.activeNotifications.mapNotNull { shown ->
            val id = shown.tag?.takeIf { shown.id == NOTIFICATION_ID } ?: return@mapNotNull null
            val extras = shown.notification.extras
            JSONObject()
                .put("id", id)
                .put("title", extras.getCharSequence(Notification.EXTRA_TITLE)?.toString() ?: "")
                .put("data", JSONObject(extras.getString(DATA_EXTRA) ?: "{}"))
                .apply {
                    extras.getCharSequence(Notification.EXTRA_TEXT)?.let { put("body", it.toString()) }
                    extras.getString(CATEGORY_EXTRA)?.let { put("category", it) }
                    shown.notification.channelId.takeIf { it != CHANNEL }?.let { put("channel", it) }
                }
        }

    fun remove(id: String) = manager.cancel(id, NOTIFICATION_ID)

    /**
     * An action that launches the app when it has `foreground`, or otherwise reaches the plugin
     * without opening it. An action with a text field passes the text it receives.
     */
    private fun action(content: Content, source: String, action: Action): Notification.Action {
        val opened = described(content, source).put("action", action.id)
        val identifier = "action\n${content.id}\n${action.id}"
        val mutable = action.inputLabel != null
        val intent =
            if (action.foreground) {
                launch(identifier, opened, mutable)
            } else {
                respond(identifier, Intent().putExtra(OPENED_EXTRA, opened.toString()), mutable)
            }
        val builder = Notification.Action.Builder(null as Icon?, action.title, intent)
        action.inputLabel?.let { builder.addRemoteInput(RemoteInput.Builder(INPUT_KEY).setLabel(it).build()) }
        return builder.build()
    }

    /** The app's monochrome `drawable/tokamak_notification`, or its launcher icon. */
    private fun icon(): Int =
        context.resources
            .getIdentifier("tokamak_notification", "drawable", context.packageName)
            .takeIf { it != 0 }
            ?: context.applicationInfo.icon.takeIf { it != 0 }
            ?: android.R.drawable.sym_def_app_icon

    /** Launches the app with [opened]. */
    private fun launch(identifier: String, opened: JSONObject, mutable: Boolean): PendingIntent {
        val intent =
            requireNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName))
                .setIdentifier(identifier)
                .putExtra(OPENED_EXTRA, opened.toString())
        return PendingIntent.getActivity(context, 0, intent, flags(mutable))
    }

    /** Delivers [response] to [TokamakResponseReceiver] without opening the app. */
    private fun respond(identifier: String, response: Intent, mutable: Boolean = false): PendingIntent {
        val intent =
            Intent(context, TokamakResponseReceiver::class.java)
                .setIdentifier(identifier)
                .putExtras(response)
        return PendingIntent.getBroadcast(context, 0, intent, flags(mutable))
    }

    /** Text fields fill in their intent, which therefore must be mutable. */
    private fun flags(mutable: Boolean): Int =
        PendingIntent.FLAG_UPDATE_CURRENT or if (mutable) PendingIntent.FLAG_MUTABLE else PendingIntent.FLAG_IMMUTABLE

    /** The default channel, named after the app. */
    private fun appChannel(): NotificationChannel =
        NotificationChannel(
            CHANNEL,
            context.applicationInfo.loadLabel(context.packageManager),
            NotificationManager.IMPORTANCE_DEFAULT,
        )

    /** [content] as listeners receive it, before the plugin adds the chosen action and typed text. */
    private fun described(content: Content, source: String): JSONObject =
        JSONObject()
            .put("id", content.id)
            .put("title", content.title)
            .put("body", content.body ?: JSONObject.NULL)
            .put("data", content.data)
            .put("source", source)

    companion object {
        /** The text typed into the action that sent [intent], if any. */
        fun input(intent: Intent): String? =
            RemoteInput.getResultsFromIntent(intent)?.getCharSequence(INPUT_KEY)?.toString()
    }
}

/** Receives actions that do not open the app, and dismissed notifications. */
class TokamakResponseReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val plugin = context.tokamakPlugin("notifications") as? TokamakNotificationsPlugin ?: return
        val pending = goAsync()
        plugin.respond(intent, pending::finish)
    }
}
