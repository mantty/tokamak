package com.tokamak.plugins.location

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.net.Uri
import android.os.Bundle
import android.os.CancellationSignal
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.provider.Settings
import android.util.Log
import com.tokamak.runtime.TokamakHost
import com.tokamak.runtime.TokamakPlugin
import com.tokamak.runtime.TokamakPluginError
import com.tokamak.runtime.TokamakPluginReply
import com.tokamak.runtime.optionalString
import com.tokamak.runtime.requireObject
import org.json.JSONArray
import org.json.JSONObject

/** How long the Worker's listeners have for an event, within the time Android gives a broadcast. */
private const val EVENT_DEADLINE_MILLIS = 8_000L

/** The listener methods, which receive the Worker's events. */
private val LISTENERS = setOf("onLocationUpdate", "onRegionTransition", "onVisit", "onLocationError")

class TokamakLocationPlugin(
    private val host: TokamakHost,
) : TokamakPlugin {
    override val id = "location"

    private val context = host.context
    private val manager =
        context.getSystemService(Context.LOCATION_SERVICE) as LocationManager
    private val handler = Handler(Looper.getMainLooper())

    /** The permissions the app's manifest requests. */
    private val declared: Set<String> by lazy {
        context.packageManager
            .getPackageInfo(context.packageName, PackageManager.GET_PERMISSIONS)
            .requestedPermissions
            .orEmpty()
            .toSet()
    }

    /** Foreground location permissions, which the `precision` build setting declares. */
    private val foreground: Set<String> by lazy { setOf(COARSE, FINE).filter(declared::contains).toSet() }

    private val updates = LocationUpdates(context)
    private val listeners = mutableMapOf<String, MutableMap<Int, TokamakPluginReply>>()
    private var nextListener = 0

    /** Positions that arrived while the Worker handled the previous update, and the broadcasts to finish once it has them. */
    private val positions = mutableListOf<JSONObject>()
    private val finishes = mutableListOf<() -> Unit>()
    private var updating = false

    init {
        restore(always = false)
    }

    override fun call(method: String, arguments: Any?, reply: TokamakPluginReply) {
        when (method) {
            "getCurrentPosition" ->
                withPermission { granted ->
                    if (granted) currentPosition(arguments as? JSONObject, reply) else reply(permissionDenied())
                }
            "permission" -> reply(Result.success(permission()))
            "requestPermission" -> requestPermission(requireObject(arguments), reply)
            "servicesEnabled" -> reply(Result.success(manager.isLocationEnabled))
            "openSettings" -> openSettings(reply)
            "startUpdates" -> startUpdates(requireObject(arguments), reply)
            "stopUpdates" -> {
                updates.stop()
                reply(Result.success(null))
            }
            "getUpdates" -> reply(Result.success(updates.options))
            else -> super.call(method, arguments, reply)
        }
    }

    override fun subscribe(
        method: String,
        arguments: Any?,
        reply: TokamakPluginReply,
    ): () -> Unit {
        if (method in LISTENERS) {
            val key = nextListener++
            listeners.getOrPut(method, ::mutableMapOf)[key] = reply
            return { listeners[method]?.remove(key) }
        }
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

    /**
     * Passes [action] whether the app may read fine or coarse location, asking when it may not.
     * Throws `NeedsUIError` when asking would show UI while the app is in the background.
     */
    private fun withPermission(action: (Boolean) -> Unit) {
        if (hasForegroundPermission()) {
            action(true)
            return
        }
        host.requestPermissions(foreground) { granted -> action(granted.containsValue(true)) }
    }

    private fun granted(permission: String): Boolean =
        context.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED

    private fun hasForegroundPermission(): Boolean = granted(FINE) || granted(COARSE)

    /** The app's in-use and background permission, and the precision it gets, or would get once allowed. */
    private fun permission(): JSONObject {
        val inUse = if (hasForegroundPermission()) "granted" else state(COARSE)
        val background = if (BACKGROUND in declared) state(BACKGROUND) else "denied"
        val precise = granted(FINE) || (!granted(COARSE) && FINE in declared)
        return JSONObject()
            .put("inUse", inUse)
            .put("background", background)
            .put("precision", if (precise) "precise" else "approximate")
    }

    private fun state(permission: String): String =
        when {
            granted(permission) -> "granted"
            host.wouldPrompt(permission) -> "prompt"
            else -> "denied"
        }

    /**
     * Asks for foreground permission, then for background permission when [options] ask for it, as
     * Android grants background permission only to an app with foreground permission.
     */
    private fun requestPermission(options: JSONObject, reply: TokamakPluginReply) {
        val background = options.optBoolean("background")
        if (background && BACKGROUND !in declared) {
            throw TokamakPluginError.notSupported("Background location needs the location plugin's background build setting")
        }
        val asked = if (options.optionalString("precision") == "approximate") setOf(COARSE) else foreground
        host.requestPermissions(asked) { granted ->
            if (!background || !granted.containsValue(true)) return@requestPermissions reply(Result.success(permission()))
            runCatching { host.requestPermissions(setOf(BACKGROUND)) { reply(Result.success(permission())) } }
                .onFailure { reply(Result.failure(it)) }
        }
    }

    private fun requirePermission() {
        if (!hasForegroundPermission()) throw TokamakPluginError("NotAllowedError", "Location permission has not been granted")
    }

    private fun startUpdates(options: JSONObject, reply: TokamakPluginReply) {
        requirePermission()
        runCatching { updates.start(options) }
            .onSuccess { reply(Result.success(null)) }
            .onFailure { reply(Result.failure(locationError(it))) }
    }

    /**
     * Requests updates again where Android removed them, always or only when their `PendingIntent`
     * no longer exists, reporting a failure such as revoked permission to `onLocationError`.
     */
    internal fun restore(always: Boolean) {
        runCatching { updates.restore(always) }.onFailure { fail(locationError(it)) }
    }

    /** Delivers the updates [intent] carries, then calls [finish]. Call it on the main thread. */
    internal fun receive(intent: Intent, finish: () -> Unit) {
        if (intent.hasExtra(LocationManager.KEY_PROVIDER_ENABLED)) {
            if (intent.getBooleanExtra(LocationManager.KEY_PROVIDER_ENABLED, true)) return finish()
            return fail(TokamakPluginError("NotReadableError", "The location provider was turned off"), done = finish)
        }
        val received = locations(intent).map(::position)
        if (received.isEmpty()) return finish()
        positions += received
        finishes += finish
        if (!updating) sendUpdate()
    }

    @Suppress("DEPRECATION")
    private fun locations(intent: Intent): List<Location> =
        intent.getParcelableArrayExtra(LocationManager.KEY_LOCATIONS)?.filterIsInstance<Location>()
            ?: listOfNotNull(intent.getParcelableExtra(LocationManager.KEY_LOCATION_CHANGED))

    /** Sends the waiting positions as one update, then the positions that arrive meanwhile as the next. */
    private fun sendUpdate() {
        updating = positions.isNotEmpty()
        if (!updating) return
        val event = JSONObject().put("positions", JSONArray(positions.toList()))
        val delivered = finishes.toList()
        positions.clear()
        finishes.clear()
        send("onLocationUpdate", "update", event) {
            delivered.forEach { it() }
            sendUpdate()
        }
    }

    /** Reports a background service's failure, which concerns the region [id] when given. */
    private fun fail(error: TokamakPluginError, id: String? = null, done: () -> Unit = {}) {
        val event = JSONObject().put("name", error.errorName).put("message", error.message).put("id", id)
        send("onLocationError", "error", event, done)
    }

    /** Delivers [event] to the page's [method] listeners and the Worker's [name] listeners, then calls [done]. */
    private fun send(method: String, name: String, event: JSONObject, done: () -> Unit = {}) {
        listeners[method].orEmpty().values.toList().forEach { it(Result.success(event)) }
        Thread {
            runCatching { host.emit(name, event.toString(), EVENT_DEADLINE_MILLIS) }
                .onFailure { Log.w("tokamak", "location $name failed: ${it.message}") }
            context.mainExecutor.execute(done)
        }.start()
    }

    private fun openSettings(reply: TokamakPluginReply) {
        val app = Uri.fromParts("package", context.packageName, null)
        host.requireForegroundActivity().startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, app))
        reply(Result.success(null))
    }

    /**
     * Replies with a fix no older than the `maximumAge` option, taking a new one when there is
     * none, or fails after the `timeout` option.
     */
    private fun currentPosition(options: JSONObject?, reply: TokamakPluginReply) {
        val provider = availableProvider(reply) ?: return
        val maximumAge = options?.optLong("maximumAge", 0) ?: 0
        val recent = runCatching { manager.getLastKnownLocation(provider) }.getOrNull()
        if (recent != null && ageMillis(recent) <= maximumAge) {
            reply(Result.success(position(recent)))
            return
        }
        val cancellation = CancellationSignal()
        val timeout = Runnable {
            cancellation.cancel()
            reply(Result.failure(TokamakPluginError("TimeoutError", "Location was not found in time")))
        }
        handler.postDelayed(timeout, options?.optLong("timeout", UNLIMITED) ?: UNLIMITED)
        runCatching {
            manager.getCurrentLocation(
                provider,
                cancellation,
                context.mainExecutor,
            ) { location ->
                handler.removeCallbacks(timeout)
                reply(if (location == null) missingFix() else Result.success(position(location)))
            }
        }.onFailure {
            handler.removeCallbacks(timeout)
            reply(locationFailure(it))
        }
    }

    private fun ageMillis(location: Location): Long =
        (SystemClock.elapsedRealtimeNanos() - location.elapsedRealtimeNanos) / 1_000_000

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

    private fun position(location: Location): JSONObject =
        JSONObject()
            .put(
                "coords",
                JSONObject()
                    .put("latitude", location.latitude)
                    .put("longitude", location.longitude)
                    .put("accuracy", location.accuracy.toDouble())
                    .put("altitude", nullable(location.hasAltitude(), location.altitude))
                    .put(
                        "altitudeAccuracy",
                        nullable(location.hasVerticalAccuracy(), location.verticalAccuracyMeters.toDouble()),
                    )
                    .put("heading", nullable(location.hasBearing(), location.bearing.toDouble()))
                    .put("speed", nullable(location.hasSpeed(), location.speed.toDouble())),
            )
            .put("timestamp", location.time)

    private fun nullable(available: Boolean, value: Double): Any = if (available) value else JSONObject.NULL

    private fun permissionDenied(): Result<Any?> = locationFailure(SecurityException())

    /** Why no fix came: Android gives none in the background without background location permission. */
    private fun missingFix(): Result<Any?> =
        if (host.isInForeground || granted(BACKGROUND)) {
            unavailable("Location is unavailable")
        } else {
            Result.failure(TokamakPluginError("NotAllowedError", "Location permission does not cover the background"))
        }

    private fun unavailable(message: String): Result<Any?> =
        Result.failure(TokamakPluginError("NotReadableError", message))

    private fun locationFailure(error: Throwable): Result<Any?> = Result.failure(locationError(error))

    private fun locationError(error: Throwable): TokamakPluginError =
        if (error is SecurityException) {
            TokamakPluginError("NotAllowedError", "Location permission was denied")
        } else {
            TokamakPluginError("NotReadableError", error.message ?: "Location is unavailable")
        }

    private companion object {
        /** The `timeout` that never expires, as the page sends it. */
        const val UNLIMITED = 0xffffffffL
        const val FINE = Manifest.permission.ACCESS_FINE_LOCATION
        const val COARSE = Manifest.permission.ACCESS_COARSE_LOCATION
        const val BACKGROUND = Manifest.permission.ACCESS_BACKGROUND_LOCATION
    }
}
