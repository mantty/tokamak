package com.tokamak.plugins.location

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.location.LocationManager

/** The JSON of each monitored region, by id. */
private const val PREFERENCES = "tokamak.location.regions"

/**
 * Circular regions, as proximity alerts that `LocationManager` sends to [TokamakLocationReceiver]
 * through a `PendingIntent` per region, starting the app's process when it has ended. The regions
 * persist until [remove], and [restore] adds them again where Android removed them. Use it on the
 * main thread.
 */
internal class LocationRegions(private val context: Context) {
    private val manager = context.getSystemService(LocationManager::class.java)
    private val preferences = context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)

    fun all(): List<Region> = preferences.all.values.map { Region.from(it as String) }

    fun get(id: String): Region? = preferences.getString(id, null)?.let(Region::from)

    /**
     * Monitors [region], replacing one with the same id. It starts in an unknown state, so a device
     * already inside reports an entry. Throws `SecurityException` without precise location.
     */
    fun add(region: Region) {
        val alert = checkNotNull(pendingIntent(region.id, PendingIntent.FLAG_UPDATE_CURRENT))
        manager.removeProximityAlert(alert)
        manager.addProximityAlert(region.latitude, region.longitude, region.radius.toFloat(), -1, alert)
        preferences.edit().putString(region.id, region.json().toString()).apply()
    }

    fun remove(id: String) {
        preferences.edit().remove(id).apply()
        pendingIntent(id, PendingIntent.FLAG_NO_CREATE)?.let {
            manager.removeProximityAlert(it)
            it.cancel()
        }
    }

    /**
     * Adds the regions whose `PendingIntent` no longer exists again, as after a restart or Android
     * 15's force stop. Adding one resets its state. Throws `SecurityException` without precise
     * location.
     */
    fun restore() {
        all().filter { pendingIntent(it.id, PendingIntent.FLAG_NO_CREATE) == null }.forEach(::add)
    }

    /** The region [id]'s `PendingIntent`, mutable so `LocationManager` adds whether the device entered. */
    private fun pendingIntent(id: String, flags: Int): PendingIntent? =
        PendingIntent.getBroadcast(
            context,
            0,
            Intent(context, TokamakLocationReceiver::class.java).setAction(REGION_ACTION).setIdentifier(id),
            flags or PendingIntent.FLAG_MUTABLE,
        )

    companion object {
        const val REGION_ACTION = "com.tokamak.plugins.location.REGION"
    }
}
