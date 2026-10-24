package com.tokamak.plugins.location

import android.app.ActivityManager
import android.app.ApplicationExitInfo
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.location.LocationManager
import android.location.LocationRequest
import com.tokamak.runtime.TokamakHost
import org.json.JSONObject

/** The JSON of each kind of updates' options, by name. */
private const val PREFERENCES = "tokamak.location"
private const val DEFAULT_INTERVAL_MILLIS = 10_000L

private val QUALITIES =
    mapOf(
        Accuracy.High to LocationRequest.QUALITY_HIGH_ACCURACY,
        Accuracy.Balanced to LocationRequest.QUALITY_BALANCED_POWER_ACCURACY,
        Accuracy.Low to LocationRequest.QUALITY_LOW_POWER,
    )

/**
 * Location updates of one kind, [name]: `startUpdates`' or significant changes'. `LocationManager`
 * sends them to [TokamakLocationReceiver] through a `PendingIntent`, starting the app's process when
 * it has ended. Their options persist until [stop], and [restore] requests them where they are not
 * running. Use it on the main thread.
 */
internal class LocationUpdates(private val host: TokamakHost, private val name: String) {
    private val context = host.context
    private val manager = context.getSystemService(LocationManager::class.java)
    private val preferences = context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)

    /** The options of the updates, or null. */
    val options: UpdateOptions?
        get() = preferences.getString(name, null)?.let { host.fromJson(JSONObject(it), UpdateOptions::class) }

    /** Keeps [options], which [restore] requests. */
    fun keep(options: UpdateOptions) {
        preferences.edit().putString(name, host.json(options).toString()).apply()
    }

    /**
     * Keeps [options] and requests updates with them, replacing others; the same options running
     * change nothing. Throws `SecurityException` without permission.
     */
    fun start(options: UpdateOptions) {
        if (options == this.options && isRequested()) return
        keep(options)
        request(options)
    }

    fun stop() {
        preferences.edit().remove(name).apply()
        pendingIntent(PendingIntent.FLAG_NO_CREATE)?.let {
            manager.removeUpdates(it)
            it.cancel()
        }
    }

    /**
     * Requests the running updates again when Android [removed] the request, when their
     * `PendingIntent` no longer exists, or after a force stop, which removes the request on Android
     * 14. Throws `SecurityException` when the app no longer has permission.
     */
    fun restore(removed: Boolean) {
        val options = options ?: return
        if (removed || forceStopped() || !isRequested()) request(options)
    }

    private fun isRequested(): Boolean = pendingIntent(PendingIntent.FLAG_NO_CREATE) != null

    /** Whether the app's process last ended by the user's force stop. */
    private fun forceStopped(): Boolean =
        context.getSystemService(ActivityManager::class.java)
            .getHistoricalProcessExitReasons(null, 0, 1)
            .firstOrNull()
            ?.reason == ApplicationExitInfo.REASON_USER_REQUESTED

    private fun request(options: UpdateOptions) {
        val accuracy = options.accuracy ?: Accuracy.High
        val request =
            LocationRequest.Builder(options.interval ?: DEFAULT_INTERVAL_MILLIS)
                .setQuality(QUALITIES.getValue(accuracy))
                .setMinUpdateDistanceMeters((options.distance ?: 0.0).toFloat())
                .build()
        val updates = checkNotNull(pendingIntent(PendingIntent.FLAG_UPDATE_CURRENT))
        try {
            manager.requestLocationUpdates(provider(accuracy), request, updates)
        } catch (error: Exception) {
            // Without the `PendingIntent`, [restore] requests them again.
            updates.cancel()
            throw error
        }
    }

    /** The fused provider when the device has one, and otherwise GPS for high accuracy and the network for less. */
    private fun provider(accuracy: Accuracy): String =
        when {
            manager.hasProvider(LocationManager.FUSED_PROVIDER) -> LocationManager.FUSED_PROVIDER
            accuracy == Accuracy.High -> LocationManager.GPS_PROVIDER
            else -> LocationManager.NETWORK_PROVIDER
        }

    /** The updates' `PendingIntent`, mutable so `LocationManager` adds the locations. */
    private fun pendingIntent(flags: Int): PendingIntent? =
        PendingIntent.getBroadcast(
            context,
            0,
            Intent(context, TokamakLocationReceiver::class.java).setAction("com.tokamak.plugins.location.${name.uppercase()}"),
            flags or PendingIntent.FLAG_MUTABLE,
        )

    companion object {
        /** `startUpdates`' updates. */
        const val UPDATES = "updates"

        /** Significant changes, as `setup` starts them. */
        const val SIGNIFICANT_CHANGES = "significantChanges"
    }
}
