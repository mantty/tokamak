package com.tokamak.plugins.location

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import org.json.JSONObject

private const val CHANNEL = "tokamak.location"
private const val NOTIFICATION_ID = 1

/**
 * Runs while updates have a `foregroundService`. Android neither throttles location for an app with
 * a location foreground service nor withholds it off screen. The service requests no location
 * itself, and Android restarts it after ending it for memory.
 */
class TokamakLocationService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    /** Shows the notification the start [intent] carries, or on a restart, the running updates'. */
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val content =
            intent?.getStringExtra(NOTIFICATION_EXTRA)?.let(::JSONObject)
                ?: LocationUpdates(this).options?.optJSONObject("foregroundService")
        if (content == null) {
            stopSelf()
            return START_NOT_STICKY
        }
        startForeground(NOTIFICATION_ID, notification(content), ServiceInfo.FOREGROUND_SERVICE_TYPE_LOCATION)
        return START_STICKY
    }

    /** The notification [content] describes, in the location channel it names, opening the app. */
    private fun notification(content: JSONObject): Notification {
        val channel = NotificationChannel(CHANNEL, content.getString("channel"), NotificationManager.IMPORTANCE_LOW)
        getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
        val open =
            packageManager.getLaunchIntentForPackage(packageName)?.let {
                PendingIntent.getActivity(this, 0, it, PendingIntent.FLAG_IMMUTABLE)
            }
        return Notification.Builder(this, CHANNEL)
            .setSmallIcon(icon())
            .setContentTitle(content.getString("title"))
            .setContentText(content.getString("body"))
            .setContentIntent(open)
            .setOngoing(true)
            .build()
    }

    /** The app's monochrome `drawable/tokamak_notification`, or its launcher icon. */
    private fun icon(): Int =
        resources.getIdentifier("tokamak_notification", "drawable", packageName).takeIf { it != 0 }
            ?: applicationInfo.icon.takeIf { it != 0 }
            ?: android.R.drawable.sym_def_app_icon

    companion object {
        /** The start intent's notification, as JSON. */
        const val NOTIFICATION_EXTRA = "com.tokamak.plugins.location.NOTIFICATION"
    }
}
