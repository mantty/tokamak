package com.tokamak.plugins.notifications

import android.app.AlarmManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import com.tokamak.runtime.tokamakPlugin
import org.json.JSONObject

private const val PREFERENCES = "tokamak.notifications.schedule"

/** When each notification was scheduled, which its trigger's intervals count from. */
private const val ANCHORS = "tokamak.notifications.anchors"
private const val ID_EXTRA = "com.tokamak.notifications.scheduled"

/**
 * Scheduled notifications, stored so they survive restarts, each with an alarm allowed while
 * idle for its trigger's next time: exact when the trigger asks and the app may, inexact
 * otherwise. Use it on the main thread.
 */
internal class Schedule(private val context: Context) {
    private val preferences = context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
    private val anchors = context.getSharedPreferences(ANCHORS, Context.MODE_PRIVATE)
    private val alarms = context.getSystemService(AlarmManager::class.java)

    /** Whether the app may set exact alarms, which the user can allow and revoke. */
    val canScheduleExact: Boolean get() = alarms.canScheduleExactAlarms()

    fun all(): List<Content> =
        preferences.all.values.map { Content.fromJson(JSONObject(it as String)) }

    fun add(content: Content) {
        preferences.edit().putString(content.id, content.json.toString()).apply()
        anchors.edit().putLong(content.id, System.currentTimeMillis()).apply()
        arm(content)
    }

    fun remove(id: String) {
        preferences.edit().remove(id).apply()
        anchors.edit().remove(id).apply()
        alarms.cancel(alarm(id))
    }

    /**
     * The notification scheduled as [id], whose alarm fired. Sets the alarm for its next time
     * when its trigger repeats, and removes it otherwise.
     */
    fun fire(id: String): Content? {
        val content = preferences.getString(id, null)?.let { Content.fromJson(JSONObject(it)) } ?: return null
        if (content.trigger?.repeats == true && arm(content)) return content
        preferences.edit().remove(id).apply()
        anchors.edit().remove(id).apply()
        return content
    }

    /** Sets an alarm for every stored notification, as a restart clears them. */
    fun armAll() = all().forEach(::arm)

    /** Sets the alarm for [content]'s next time, returning false when its trigger never fires again. */
    private fun arm(content: Content): Boolean {
        val trigger = content.trigger ?: return false
        val now = System.currentTimeMillis()
        val next = trigger.next(now, anchors.getLong(content.id, now)) ?: return false
        if (trigger.exact && canScheduleExact) {
            alarms.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, next, alarm(content.id))
        } else {
            alarms.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, next, alarm(content.id))
        }
        return true
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
        val content = Schedule(context).fire(id) ?: return
        val plugin = context.tokamakPlugin("notifications") as? TokamakNotificationsPlugin ?: return
        val pending = goAsync()
        plugin.present(content, "local") { pending.finish() }
    }
}

/**
 * Sets the schedule's alarms again when Android cleared them, after a restart or an update, or
 * when their times change: the clock or time zone changed, or the user allowed exact alarms.
 */
class TokamakBootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action in ACTIONS) Schedule(context).armAll()
    }

    private companion object {
        val ACTIONS =
            setOf(
                Intent.ACTION_BOOT_COMPLETED,
                Intent.ACTION_MY_PACKAGE_REPLACED,
                Intent.ACTION_TIME_CHANGED,
                Intent.ACTION_TIMEZONE_CHANGED,
                AlarmManager.ACTION_SCHEDULE_EXACT_ALARM_PERMISSION_STATE_CHANGED,
            )
    }
}
