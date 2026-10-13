package com.tokamak.plugins.location

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.location.LocationManager
import org.json.JSONObject

/** The JSON of each monitored region, as `addRegion` received it, by id. */
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

    fun all(): List<JSONObject> = preferences.all.values.map { JSONObject(it as String) }

    fun get(id: String): JSONObject? = preferences.getString(id, null)?.let(::JSONObject)

    /**
     * Monitors [region], replacing one with the same id. It starts in an unknown state, so a device
     * already inside reports an entry. Throws `SecurityException` without precise location.
     */
    fun add(region: JSONObject) {
        val id = region.getString("id")
        val alert = pendingIntent(id, PendingIntent.FLAG_UPDATE_CURRENT) ?: return
        manager.removeProximityAlert(alert)
        manager.addProximityAlert(
            region.getDouble("latitude"),
            region.getDouble("longitude"),
            region.getDouble("radius").toFloat(),
            -1,
            alert,
        )
        preferences.edit().putString(id, region.toString()).apply()
    }

    fun remove(id: String) {
        preferences.edit().remove(id).apply()
        pendingIntent(id, PendingIntent.FLAG_NO_CREATE)?.let {
            manager.removeProximityAlert(it)
            it.cancel()
        }
    }

    /**
     * Adds the regions again: all when Android [removed] them, and otherwise those whose
     * `PendingIntent` no longer exists. Throws `SecurityException` without precise location.
     */
    fun restore(removed: Boolean) {
        all().filter { removed || pendingIntent(it.getString("id"), PendingIntent.FLAG_NO_CREATE) == null }
            .forEach(::add)
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
