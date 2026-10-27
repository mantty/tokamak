package com.tokamak.plugins.location

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import com.tokamak.runtime.tokamakNotificationIcon
import com.tokamak.runtime.tokamakPlugin

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
            intent?.getStringArrayExtra(NOTIFICATION_EXTRA)?.let { (title, body, channel) -> ServiceNotification(title, body, channel) }
                ?: (tokamakPlugin("location") as? TokamakLocationPlugin)?.getUpdates()?.foregroundService
        if (content == null) {
            stopSelf()
            return START_NOT_STICKY
        }
        startForeground(NOTIFICATION_ID, notification(content), ServiceInfo.FOREGROUND_SERVICE_TYPE_LOCATION)
        return START_STICKY
    }

    /** The notification [content] describes, in the location channel it names, opening the app. */
    private fun notification(content: ServiceNotification): Notification {
        val channel = NotificationChannel(CHANNEL, content.channel, NotificationManager.IMPORTANCE_LOW)
        getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
        val open =
            packageManager.getLaunchIntentForPackage(packageName)?.let {
                PendingIntent.getActivity(this, 0, it, PendingIntent.FLAG_IMMUTABLE)
            }
        return Notification.Builder(this, CHANNEL)
            .setSmallIcon(tokamakNotificationIcon())
            .setContentTitle(content.title)
            .setContentText(content.body)
            .setContentIntent(open)
            .setOngoing(true)
            .build()
    }

    companion object {
        /** The start intent's notification: its title, body and channel. */
        private const val NOTIFICATION_EXTRA = "com.tokamak.plugins.location.NOTIFICATION"

        /** Starts the service showing [notification], or shows it in the running service. */
        fun start(context: Context, notification: ServiceNotification) {
            val content = arrayOf(notification.title, notification.body, notification.channel)
            context.startForegroundService(Intent(context, TokamakLocationService::class.java).putExtra(NOTIFICATION_EXTRA, content))
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, TokamakLocationService::class.java))
        }
    }
}
