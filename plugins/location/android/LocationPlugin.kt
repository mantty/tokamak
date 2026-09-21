package com.tokamak.plugins.location

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.pm.PackageManager
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.os.Bundle
import android.os.CancellationSignal
import android.os.Looper
import com.tokamak.runtime.TokamakPlugin
import com.tokamak.runtime.TokamakPluginError
import com.tokamak.runtime.TokamakPluginReply

internal class TokamakLocationPlugin(
    private val activity: Activity,
) : TokamakPlugin {
    override val id = "location"

    private val manager =
        activity.getSystemService(Context.LOCATION_SERVICE) as LocationManager
    private val permissionRequests = mutableListOf<(Boolean) -> Unit>()

    override fun call(method: String, arguments: Any?, reply: TokamakPluginReply) {
        if (method != "getCurrentPosition") {
            super.call(method, arguments, reply)
            return
        }
        withPermission { granted ->
            if (granted) currentPosition(reply) else reply(permissionDenied())
        }
    }

    override fun subscribe(
        method: String,
        arguments: Any?,
        reply: TokamakPluginReply,
    ): () -> Unit {
        if (method != "watchPosition") return super.subscribe(method, arguments, reply)
        var cancelled = false
        var stop = {}
        withPermission { granted ->
            if (!granted) {
                reply(permissionDenied())
            } else if (!cancelled) {
                stop = watchPosition(reply)
            }
        }
        return {
            cancelled = true
            stop()
        }
    }

    override fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray,
    ) {
        if (requestCode != LOCATION_PERMISSION_REQUEST) return
        val granted = grantResults.any { it == PackageManager.PERMISSION_GRANTED }
        val requests = permissionRequests.toList()
        permissionRequests.clear()
        requests.forEach { it(granted) }
    }

    private fun withPermission(action: (Boolean) -> Unit) {
        if (hasPermission()) {
            action(true)
            return
        }
        permissionRequests.add(action)
        if (permissionRequests.size > 1) return
        activity.requestPermissions(
            arrayOf(
                Manifest.permission.ACCESS_FINE_LOCATION,
                Manifest.permission.ACCESS_COARSE_LOCATION,
            ),
            LOCATION_PERMISSION_REQUEST,
        )
    }

    private fun hasPermission(): Boolean =
        activity.checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) ==
            PackageManager.PERMISSION_GRANTED ||
            activity.checkSelfPermission(Manifest.permission.ACCESS_COARSE_LOCATION) ==
            PackageManager.PERMISSION_GRANTED

    private fun currentPosition(reply: TokamakPluginReply) {
        val provider = runCatching(::provider).getOrElse {
            reply(locationFailure(it, "Location is unavailable"))
            return
        }
        if (provider == null) {
            reply(unavailable("No location provider is available"))
            return
        }
        runCatching {
            manager.getCurrentLocation(
                provider,
                CancellationSignal(),
                activity.mainExecutor,
            ) { location ->
                if (location == null) reply(unavailable("Location is unavailable"))
                else reply(Result.success(position(location)))
            }
        }.onFailure { reply(locationFailure(it, "Location is unavailable")) }
    }

    private fun watchPosition(reply: TokamakPluginReply): () -> Unit {
        val provider = runCatching(::provider).getOrElse {
            reply(locationFailure(it, "Location is unavailable"))
            return {}
        }
        if (provider == null) {
            reply(unavailable("No location provider is available"))
            return {}
        }
        val listener = object : LocationListener {
            override fun onLocationChanged(location: Location) {
                reply(Result.success(position(location)))
            }

            override fun onProviderDisabled(provider: String) {
                reply(unavailable("Location provider is unavailable"))
            }

            override fun onStatusChanged(provider: String?, status: Int, extras: Bundle?) = Unit
        }
        val started = runCatching {
            manager.requestLocationUpdates(provider, 1000, 0F, listener, Looper.getMainLooper())
        }
        if (started.isFailure) {
            reply(locationFailure(started.exceptionOrNull(), "Location is unavailable"))
            return {}
        }
        return { manager.removeUpdates(listener) }
    }

    private fun provider(): String? =
        listOf(
            LocationManager.GPS_PROVIDER,
            LocationManager.NETWORK_PROVIDER,
            LocationManager.FUSED_PROVIDER,
        ).firstOrNull(manager::isProviderEnabled)
            ?: manager.getProviders(true).firstOrNull()

    private fun position(location: Location): Map<String, Any?> =
        mapOf(
            "coords" to
                mapOf(
                    "latitude" to location.latitude,
                    "longitude" to location.longitude,
                    "accuracy" to location.accuracy.toDouble(),
                    "altitude" to location.altitude.takeIf { location.hasAltitude() },
                    "altitudeAccuracy" to
                        location.verticalAccuracyMeters.toDouble()
                            .takeIf { location.hasVerticalAccuracy() },
                    "heading" to location.bearing.toDouble().takeIf { location.hasBearing() },
                    "speed" to location.speed.toDouble().takeIf { location.hasSpeed() },
                ),
            "timestamp" to location.time,
        )

    private fun permissionDenied(): Result<Any?> =
        Result.failure(TokamakPluginError("NotAllowedError", "Location permission was denied"))

    private fun unavailable(message: String): Result<Any?> =
        Result.failure(TokamakPluginError("NotReadableError", message))

    private fun locationFailure(error: Throwable?, fallback: String): Result<Any?> =
        if (error is SecurityException) permissionDenied()
        else unavailable(error?.message ?: fallback)

    private companion object {
        const val LOCATION_PERMISSION_REQUEST = 0xA771
    }
}
