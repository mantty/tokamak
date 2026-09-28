package com.tokamak.plugins.notifications

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.os.Bundle
import org.json.JSONObject

internal const val CHANNEL = "tokamak.notifications"
internal const val OPENED_EXTRA = "com.tokamak.notifications.opened"
private const val DATA_EXTRA = "com.tokamak.notifications.data"

/** The plugin and FCM post every notification with this ID, telling them apart by tag. */
private const val NOTIFICATION_ID = 0

/** Posts, lists and removes the app's notifications on the default channel. */
internal class Notifier(private val context: Context) {
    private val manager = context.getSystemService(NotificationManager::class.java)

    val enabled: Boolean
        get() = manager.areNotificationsEnabled()

    fun createChannel() {
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, "Notifications", NotificationManager.IMPORTANCE_DEFAULT),
        )
    }

    /** Shows [content], replacing a notification with the same ID; opening it launches the app. */
    fun post(content: Content, source: String) {
        createChannel()
        val opened = opened(content, source)
        val notification =
            Notification.Builder(context, CHANNEL)
                .setSmallIcon(icon())
                .setContentTitle(content.title)
                .setContentText(content.body)
                .setAutoCancel(true)
                .setContentIntent(launch(content.id, opened))
                .addExtras(Bundle().apply { putString(DATA_EXTRA, content.data.toString()) })
                .build()
        manager.notify(content.id, NOTIFICATION_ID, notification)
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
                }
        }

    fun remove(id: String) = manager.cancel(id, NOTIFICATION_ID)

    /** The app's monochrome `drawable/tokamak_notification`, or its launcher icon. */
    private fun icon(): Int =
        context.resources
            .getIdentifier("tokamak_notification", "drawable", context.packageName)
            .takeIf { it != 0 }
            ?: context.applicationInfo.icon.takeIf { it != 0 }
            ?: android.R.drawable.sym_def_app_icon

    private fun launch(id: String, opened: JSONObject): PendingIntent {
        val intent =
            requireNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName))
                .setIdentifier(id)
                .putExtra(OPENED_EXTRA, opened.toString())
        return PendingIntent.getActivity(
            context,
            0,
            intent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
    }

    private fun opened(content: Content, source: String): JSONObject =
        JSONObject()
            .put("id", content.id)
            .put("title", content.title)
            .put("body", content.body ?: JSONObject.NULL)
            .put("data", content.data)
            .put("source", source)
}
