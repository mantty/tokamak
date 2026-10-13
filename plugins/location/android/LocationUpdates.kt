package com.tokamak.plugins.location

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.location.LocationManager
import android.location.LocationRequest
import org.json.JSONObject

private const val PREFERENCES = "tokamak.location"

/** The JSON of the running updates' options, as `startUpdates` received them. */
private const val UPDATES = "updates"
private const val UPDATES_ACTION = "com.tokamak.plugins.location.UPDATES"
private const val DEFAULT_INTERVAL_MILLIS = 10_000L

private val QUALITIES =
    mapOf(
        "high" to LocationRequest.QUALITY_HIGH_ACCURACY,
        "balanced" to LocationRequest.QUALITY_BALANCED_POWER_ACCURACY,
        "low" to LocationRequest.QUALITY_LOW_POWER,
    )

/**
 * Continuous updates, which `LocationManager` sends to [TokamakLocationReceiver] through a
 * `PendingIntent`, starting the app's process when it has ended, with [TokamakLocationService] when
 * their options name a `foregroundService`. The options persist until [stop], and [restore] requests
 * them again where Android removed the request. Use it on the main thread.
 */
internal class LocationUpdates(private val context: Context) {
    private val manager = context.getSystemService(LocationManager::class.java)
    private val preferences = context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)

    /** The options of the running updates, or null. */
    val options: JSONObject?
        get() = preferences.getString(UPDATES, null)?.let(::JSONObject)

    /**
     * Starts updates with [options], replacing any running, in the foreground service when they name
     * one, which stops otherwise. Throws `SecurityException` without permission.
     */
    fun start(options: JSONObject) {
        val notification = options.optJSONObject("foregroundService")
        if (notification == null) stopService() else startService(notification)
        request(options)
        preferences.edit().putString(UPDATES, options.toString()).apply()
    }

    fun stop() {
        preferences.edit().remove(UPDATES).apply()
        stopService()
        pendingIntent(PendingIntent.FLAG_NO_CREATE)?.let {
            manager.removeUpdates(it)
            it.cancel()
        }
    }

    /** Starts the running updates' foreground service, if they have one. */
    fun startService() {
        options?.optJSONObject("foregroundService")?.let(::startService)
    }

    private fun startService(notification: JSONObject) {
        val start =
            Intent(context, TokamakLocationService::class.java)
                .putExtra(TokamakLocationService.NOTIFICATION_EXTRA, notification.toString())
        context.startForegroundService(start)
    }

    private fun stopService() {
        context.stopService(Intent(context, TokamakLocationService::class.java))
    }

    /**
     * Requests the running updates again when Android [removed] the request, or their `PendingIntent`
     * no longer exists. Throws `SecurityException` when the app no longer has permission.
     */
    fun restore(removed: Boolean) {
        val options = options ?: return
        if (removed || pendingIntent(PendingIntent.FLAG_NO_CREATE) == null) request(options)
    }

    private fun request(options: JSONObject) {
        val accuracy = options.optString("accuracy", "high")
        val request =
            LocationRequest.Builder(options.optLong("interval", DEFAULT_INTERVAL_MILLIS))
                .setQuality(QUALITIES[accuracy] ?: LocationRequest.QUALITY_HIGH_ACCURACY)
                .setMinUpdateDistanceMeters(options.optDouble("distance", 0.0).toFloat())
                .build()
        val updates = pendingIntent(PendingIntent.FLAG_UPDATE_CURRENT) ?: return
        manager.requestLocationUpdates(provider(accuracy), request, updates)
    }

    /** The fused provider when the device has one, and otherwise GPS for high accuracy and the network for less. */
    private fun provider(accuracy: String): String =
        when {
            manager.hasProvider(LocationManager.FUSED_PROVIDER) -> LocationManager.FUSED_PROVIDER
            accuracy == "high" -> LocationManager.GPS_PROVIDER
            else -> LocationManager.NETWORK_PROVIDER
        }

    /** The updates' `PendingIntent`, mutable so `LocationManager` adds the locations. */
    private fun pendingIntent(flags: Int): PendingIntent? =
        PendingIntent.getBroadcast(
            context,
            0,
            Intent(context, TokamakLocationReceiver::class.java).setAction(UPDATES_ACTION),
            flags or PendingIntent.FLAG_MUTABLE,
        )
}
