package com.tokamak.plugins.location

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.os.Bundle
import android.os.CancellationSignal
import android.os.Looper
import com.tokamak.runtime.TokamakHost
import com.tokamak.runtime.TokamakPlugin
import com.tokamak.runtime.TokamakPluginError
import com.tokamak.runtime.TokamakPluginReply

class TokamakLocationPlugin(
    private val host: TokamakHost,
) : TokamakPlugin {
    override val id = "location"

    private val context = host.context
    private val manager =
        context.getSystemService(Context.LOCATION_SERVICE) as LocationManager

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

    /** Passes [action] whether the app may read fine or coarse location, asking when it may not. */
    private fun withPermission(action: (Boolean) -> Unit) {
        if (PERMISSIONS.any { context.checkSelfPermission(it) == PackageManager.PERMISSION_GRANTED }) {
            action(true)
            return
        }
        host.requestPermissions(PERMISSIONS) { granted -> action(granted.containsValue(true)) }
    }

    private fun currentPosition(reply: TokamakPluginReply) {
        val provider = availableProvider(reply) ?: return
        runCatching {
            manager.getCurrentLocation(
                provider,
                CancellationSignal(),
                context.mainExecutor,
            ) { location ->
                if (location == null) reply(unavailable("Location is unavailable"))
                else reply(Result.success(position(location)))
            }
        }.onFailure { reply(locationFailure(it)) }
    }

    private fun watchPosition(reply: TokamakPluginReply): () -> Unit {
        val provider = availableProvider(reply) ?: return {}
        val listener = object : LocationListener {
            override fun onLocationChanged(location: Location) {
                reply(Result.success(position(location)))
            }

            override fun onProviderDisabled(provider: String) {
                reply(unavailable("Location provider is unavailable"))
            }

            override fun onStatusChanged(provider: String?, status: Int, extras: Bundle?) = Unit
        }
        runCatching {
            manager.requestLocationUpdates(provider, 1000, 0F, listener, Looper.getMainLooper())
        }.onFailure {
            reply(locationFailure(it))
            return {}
        }
        return { manager.removeUpdates(listener) }
    }

    /** The provider to use, or null once [reply] has the reason there is none. */
    private fun availableProvider(reply: TokamakPluginReply): String? {
        val provider = runCatching(::provider).getOrElse {
            reply(locationFailure(it))
            return null
        }
        if (provider == null) reply(unavailable("No location provider is available"))
        return provider
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

    private fun locationFailure(error: Throwable): Result<Any?> =
        if (error is SecurityException) permissionDenied()
        else unavailable(error.message ?: "Location is unavailable")

    private companion object {
        val PERMISSIONS = setOf(Manifest.permission.ACCESS_FINE_LOCATION, Manifest.permission.ACCESS_COARSE_LOCATION)
    }
}
