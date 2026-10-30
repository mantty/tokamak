package com.tokamak.plugins.notifications

import android.app.AlarmManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import com.tokamak.runtime.TokamakHost
import com.tokamak.runtime.tokamakPlugin
import org.json.JSONObject

private const val PREFERENCES = "tokamak.notifications.schedule"

/** When each notification was scheduled, which its trigger's intervals count from. */
private const val ANCHORS = "tokamak.notifications.anchors"
private const val ID_EXTRA = "com.tokamak.notifications.scheduled"

/**
 * Scheduled notifications, stored so they survive restarts as the page gives them, the content's
 * fields and the trigger's in one object, each with an alarm allowed while idle for its trigger's
 * next time: exact when the trigger asks and the app may, inexact otherwise. Use it on the main
 * thread.
 */
internal class Schedule(private val host: TokamakHost) {
    private val context = host.context
    private val preferences = context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
    private val anchors = context.getSharedPreferences(ANCHORS, Context.MODE_PRIVATE)
    private val alarms = context.getSystemService(AlarmManager::class.java)

    /** Whether the app may set exact alarms, which the user can allow and revoke. */
    val canScheduleExact: Boolean get() = alarms.canScheduleExactAlarms()

    fun all(): List<ScheduledNotification> = preferences.all.values.mapNotNull { stored(it as String) }

    fun add(scheduled: ScheduledNotification) {
        val json = host.json(scheduled.content) as JSONObject
        val trigger = host.json(scheduled.trigger) as JSONObject
        trigger.keys().forEach { json.put(it, trigger.get(it)) }
        preferences.edit().putString(scheduled.content.id, json.toString()).apply()
        anchors.edit().putLong(scheduled.content.id, System.currentTimeMillis()).apply()
        arm(scheduled)
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
    fun fire(id: String): ScheduledNotification? {
        val scheduled = preferences.getString(id, null)?.let(::stored) ?: return null
        if (scheduled.trigger.repeating && arm(scheduled)) return scheduled
        preferences.edit().remove(id).apply()
        anchors.edit().remove(id).apply()
        return scheduled
    }

    /** Sets an alarm for every stored notification, as a restart clears them. */
    fun armAll() = all().forEach(::arm)

    /** The scheduled notification stored as [json], or null when it no longer reads as one. */
    private fun stored(json: String): ScheduledNotification? =
        runCatching {
            val fields = JSONObject(json)
            ScheduledNotification(host.fromJson(fields, NotificationContent::class), host.fromJson(fields, Trigger::class))
        }.getOrNull()

    /** Sets the alarm for [scheduled]'s next time, returning false when its trigger never fires again. */
    private fun arm(scheduled: ScheduledNotification): Boolean {
        val id = scheduled.content.id
        val trigger = scheduled.trigger
        val now = System.currentTimeMillis()
        val next = trigger.next(now, anchors.getLong(id, now)) ?: return false
        if (trigger.exact == true && canScheduleExact) {
            alarms.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, next, alarm(id))
        } else {
            alarms.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, next, alarm(id))
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
        val plugin = context.tokamakPlugin("notifications") as? TokamakNotificationsPlugin ?: return
        val pending = goAsync()
        plugin.fire(id) { pending.finish() }
    }
}

/**
 * Sets the schedule's alarms again when Android cleared them, after a restart or an update, or
 * when their times change: the clock or time zone changed, or the user allowed exact alarms.
 */
class TokamakBootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action in ACTIONS) (context.tokamakPlugin("notifications") as? TokamakNotificationsPlugin)?.armAll()
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
