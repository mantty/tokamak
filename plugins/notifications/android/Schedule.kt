package com.tokamak.plugins.notifications

import android.app.AlarmManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import org.json.JSONObject

private const val PREFERENCES = "tokamak.notifications.schedule"
private const val ID_EXTRA = "com.tokamak.notifications.scheduled"

/**
 * Scheduled notifications, stored so they survive restarts, each with an inexact alarm
 * allowed while idle. Use it on the main thread.
 */
internal class Schedule(private val context: Context) {
    private val preferences = context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
    private val alarms = context.getSystemService(AlarmManager::class.java)

    fun all(): List<Content> =
        preferences.all.values.map { Content.fromJson(JSONObject(it as String)) }

    fun add(content: Content) {
        preferences.edit().putString(content.id, content.toJson().toString()).apply()
        arm(content)
    }

    fun remove(id: String) {
        preferences.edit().remove(id).apply()
        alarms.cancel(alarm(id))
    }

    /** Removes and returns the notification scheduled as [id]. */
    fun take(id: String): Content? {
        val stored = preferences.getString(id, null) ?: return null
        preferences.edit().remove(id).apply()
        return Content.fromJson(JSONObject(stored))
    }

    /** Sets an alarm for every stored notification, as a restart clears them. */
    fun armAll() = all().forEach(::arm)

    private fun arm(content: Content) {
        alarms.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, content.at ?: 0, alarm(content.id))
    }

    private fun alarm(id: String): PendingIntent =
        PendingIntent.getBroadcast(
            context,
            0,
            Intent(context, TokamakAlarmReceiver::class.java).setIdentifier(id).putExtra(ID_EXTRA, id),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )

    companion object {
        fun scheduledId(intent: Intent): String? = intent.getStringExtra(ID_EXTRA)
    }
}

/** Shows a scheduled notification when its alarm fires. */
class TokamakAlarmReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val id = Schedule.scheduledId(intent) ?: return
        val content = Schedule(context).take(id) ?: return
        Notifier(context).post(content, "local")
    }
}

/** Restores the schedule's alarms after the device restarts or the app updates. */
class TokamakBootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action == Intent.ACTION_BOOT_COMPLETED || intent.action == Intent.ACTION_MY_PACKAGE_REPLACED) {
            Schedule(context).armAll()
        }
    }
}
