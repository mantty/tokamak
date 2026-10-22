package com.tokamak.plugins.location

import android.content.Context
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.location.LocationRequest
import android.os.CancellationSignal
import android.os.SystemClock
import com.tokamak.runtime.TokamakPluginError

/**
 * Positions for `getCurrentPosition` and `watchPosition`, at the provider's highest accuracy, from
 * the first enabled of the fused provider, which combines GPS with Wi-Fi and cell signals, GPS and
 * the network, or any other enabled provider. Each throws `SecurityException` without permission.
 */
internal class LocationPositions(private val context: Context, private val manager: LocationManager) {
    /** The provider's last known position when it is no older than [maximumAge] milliseconds. */
    fun recent(maximumAge: Double): Location? = manager.getLastKnownLocation(provider())?.takeIf { age(it) <= maximumAge }

    /** Passes [found] a new fix, or null when Android finds none, and returns the cancellation. */
    fun current(found: (Location?) -> Unit): () -> Unit {
        val signal = CancellationSignal()
        manager.getCurrentLocation(provider(), highAccuracy(0), signal, context.mainExecutor) { found(it) }
        return signal::cancel
    }

    /** Passes [next] each position, and [disabled] the provider turning off, until the returned release runs. */
    fun watch(next: (Location) -> Unit, disabled: () -> Unit): () -> Unit {
        val listener = object : LocationListener {
            override fun onLocationChanged(location: Location) = next(location)

            override fun onProviderDisabled(provider: String) = disabled()
        }
        manager.requestLocationUpdates(provider(), highAccuracy(1000), context.mainExecutor, listener)
        return { manager.removeUpdates(listener) }
    }

    private fun provider(): String =
        listOf(LocationManager.FUSED_PROVIDER, LocationManager.GPS_PROVIDER, LocationManager.NETWORK_PROVIDER)
            .firstOrNull(manager::isProviderEnabled)
            ?: manager.getProviders(true).firstOrNull()
            ?: throw TokamakPluginError("NotReadableError", "No location provider is available")

    /** A request for the most accurate fixes the provider gives, every [interval] milliseconds. */
    private fun highAccuracy(interval: Long): LocationRequest =
        LocationRequest.Builder(interval).setQuality(LocationRequest.QUALITY_HIGH_ACCURACY).build()

    /** The milliseconds since Android took [location]. */
    private fun age(location: Location): Double =
        (SystemClock.elapsedRealtimeNanos() - location.elapsedRealtimeNanos) / 1_000_000.0
}
