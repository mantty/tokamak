package com.tokamak.plugins.location

import android.content.Context
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.location.LocationRequest
import android.os.CancellationSignal
import android.os.SystemClock
import com.tokamak.runtime.TokamakPluginError
import kotlin.coroutines.resume
import kotlinx.coroutines.channels.SendChannel
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.suspendCancellableCoroutine

/**
 * Positions for `getCurrentPosition` and `watchPosition`, at the provider's highest accuracy, from
 * the first enabled of the fused provider, which combines GPS with Wi-Fi and cell signals, GPS and
 * the network, or any other enabled provider. Each throws `SecurityException` without permission.
 */
internal class LocationPositions(private val context: Context, private val manager: LocationManager) {
    /** The provider's last known position when it is no older than [maximumAge] milliseconds. */
    fun recent(maximumAge: Double): Location? = manager.getLastKnownLocation(provider())?.takeIf { age(it) <= maximumAge }

    /** A new fix, or null when Android finds none. */
    suspend fun current(): Location? =
        suspendCancellableCoroutine { continuation ->
            val signal = CancellationSignal()
            continuation.invokeOnCancellation { signal.cancel() }
            manager.getCurrentLocation(provider(), highAccuracy(0), signal, context.mainExecutor) { continuation.resume(it) }
        }

    /** Each position as the device moves, until the provider turns off. */
    fun watch(): Flow<Location> =
        callbackFlow {
            val listener = Forwarder(this)
            manager.requestLocationUpdates(provider(), highAccuracy(1000), context.mainExecutor, listener)
            awaitClose { manager.removeUpdates(listener) }
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

/** Sends each location to [positions], which a disabled provider closes. */
private class Forwarder(private val positions: SendChannel<Location>) : LocationListener {
    override fun onLocationChanged(location: Location) {
        positions.trySend(location)
    }

    override fun onProviderDisabled(provider: String) {
        positions.close(TokamakPluginError("NotReadableError", "Location provider is unavailable"))
    }
}
