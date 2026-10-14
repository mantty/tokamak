package com.tokamak.plugins.location

import android.Manifest
import android.app.ForegroundServiceStartNotAllowedException
import android.content.ComponentName
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
import com.tokamak.runtime.TokamakHost
import com.tokamak.runtime.TokamakPlugin
import com.tokamak.runtime.TokamakPluginError
import com.tokamak.runtime.TokamakPluginReply
import com.tokamak.runtime.optionalString
import com.tokamak.runtime.requireObject
import com.tokamak.runtime.requireString
import org.json.JSONArray
import org.json.JSONObject

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

    private val updates = LocationUpdates(context, LocationUpdates.UPDATES)
    private val significantChanges = LocationUpdates(context, LocationUpdates.SIGNIFICANT_CHANGES)
    private val regions = LocationRegions(context)
    private val events = LocationEvents(host)

    /** Whether the app's manifest declares the foreground service, as the `background-updates` build setting does. */
    private val declaresService: Boolean by lazy {
        runCatching { context.packageManager.getServiceInfo(ComponentName(context, TokamakLocationService::class.java), 0) }
            .isSuccess
    }

    init {
        restore(updatesRemoved = false)
    }

    /** Starts the updates' foreground service again once the app is on screen. */
    override fun onIntent(intent: Intent) {
        if (host.isInForeground) startService()
    }

    override fun call(method: String, arguments: Any?, reply: TokamakPluginReply) {
        when (method) {
            "setup" -> setup(requireObject(arguments), reply)
            "getCurrentPosition" ->
                withPermission { granted ->
                    if (granted) currentPosition(arguments as? JSONObject, reply) else reply(Result.failure(permissionDenied()))
                }
            "permission" -> reply(Result.success(permission()))
            "requestPermission" -> requestPermission(requireObject(arguments), reply)
            "servicesEnabled" -> reply(Result.success(manager.isLocationEnabled))
            "openSettings" -> openSettings(reply)
            "startUpdates" -> startUpdates(requireObject(arguments), reply)
            "stopUpdates" -> {
                updates.stop()
                TokamakLocationService.stop(context)
                reply(Result.success(null))
            }
            "getUpdates" -> reply(Result.success(updates.options))
            "addRegion" -> addRegion(requireObject(arguments), reply)
            "getRegions" -> reply(Result.success(JSONArray(regions.all())))
            "removeRegion" -> {
                regions.remove(requireObject(arguments).requireString("id"))
                reply(Result.success(null))
            }
            else -> super.call(method, arguments, reply)
        }
    }

    override fun subscribe(
        method: String,
        arguments: Any?,
        reply: TokamakPluginReply,
    ): () -> Unit {
        events.listen(method, reply)?.let { return it }
        if (method != "watchPosition") return super.subscribe(method, arguments, reply)
        var cancelled = false
        var stop = {}
        withPermission { granted ->
            if (!granted) {
                reply(Result.failure(permissionDenied()))
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
        askForeground(foreground, action)
    }

    /**
     * Asks for the foreground [permissions], then passes [action] whether the app has one, first
     * starting what waited for permission.
     */
    private fun askForeground(permissions: Set<String>, action: (Boolean) -> Unit) {
        host.requestPermissions(permissions) { granted ->
            val allowed = granted.containsValue(true)
            if (allowed) restore(updatesRemoved = false)
            action(allowed)
        }
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
        askForeground(asked) { allowed ->
            if (!background || !allowed) return@askForeground reply(Result.success(permission()))
            runCatching { host.requestPermissions(setOf(BACKGROUND)) { reply(Result.success(permission())) } }
                .onFailure { reply(Result.failure(it)) }
        }
    }

    private fun requirePermission() {
        if (!hasForegroundPermission()) throw TokamakPluginError("NotAllowedError", "Location permission has not been granted")
    }

    /** Starts updates in the foreground service when [options] name one, which stops otherwise. On failure, none run. */
    private fun startUpdates(options: JSONObject, reply: TokamakPluginReply) {
        requirePermission()
        val notification = options.optJSONObject("foregroundService")
        if (notification != null) requireService()
        runCatching {
            if (notification == null) TokamakLocationService.stop(context) else TokamakLocationService.start(context, notification)
            updates.start(options)
        }.onSuccess { reply(Result.success(null)) }
            .onFailure {
                updates.stop()
                TokamakLocationService.stop(context)
                reply(Result.failure(locationError(it)))
            }
    }

    private fun startService() {
        updates.options?.optJSONObject("foregroundService")?.let { TokamakLocationService.start(context, it) }
    }

    /**
     * Starts significant changes when [config] names them, and stops them otherwise. Without
     * permission they start once the app has it.
     */
    private fun setup(config: JSONObject, reply: TokamakPluginReply) {
        val options = JSONObject(SIGNIFICANT_CHANGE_OPTIONS)
        when {
            !config.optBoolean("significantChanges") -> significantChanges.stop()
            hasForegroundPermission() -> significantChanges.start(options)
            else -> significantChanges.keep(options)
        }
        reply(Result.success(null))
    }

    /**
     * Throws unless the app declares the foreground service and may start it now: Android 14 and
     * later allow a location foreground service off screen only with background permission.
     */
    private fun requireService() {
        if (!declaresService) {
            throw TokamakPluginError.notSupported("The foreground service needs the location plugin's background-updates build setting")
        }
        if (!host.isInForeground && !granted(BACKGROUND)) throw TokamakPluginError.needsUI()
    }

    /** Monitors a region, which needs precise location. */
    private fun addRegion(region: JSONObject, reply: TokamakPluginReply) {
        if (!granted(FINE)) throw TokamakPluginError("NotAllowedError", "Regions need precise location")
        runCatching { regions.add(region) }
            .onSuccess { reply(Result.success(null)) }
            .onFailure { reply(Result.failure(locationError(it))) }
    }

    /**
     * Restores updates, significant changes and regions where the app has permission, as Android
     * pauses them without it, and starts the updates' foreground service on screen, or after a restart
     * or an update with background permission. Reports a failure to `onLocationError`.
     */
    internal fun restore(updatesRemoved: Boolean) {
        if (!hasForegroundPermission()) return
        runCatching {
            updates.restore(updatesRemoved)
            significantChanges.restore(updatesRemoved)
            if (host.isInForeground || (updatesRemoved && granted(BACKGROUND))) startService()
        }.onFailure { events.fail(locationError(it)) }
        runCatching { regions.restore() }.onFailure { events.fail(locationError(it)) }
    }

    /** Delivers the updates or region transition [intent] carries, then calls [finish]. Call it on the main thread. */
    internal fun receive(intent: Intent, finish: () -> Unit) {
        if (intent.action == LocationRegions.REGION_ACTION) return transition(intent, finish)
        if (intent.hasExtra(LocationManager.KEY_PROVIDER_ENABLED)) {
            if (intent.getBooleanExtra(LocationManager.KEY_PROVIDER_ENABLED, true)) return finish()
            return events.fail(TokamakPluginError("NotReadableError", "The location provider was turned off"), finish)
        }
        val received = locations(intent).map(::position)
        if (received.isEmpty()) return finish()
        events.update(received, finish)
    }

    /** Delivers a region's entry or exit, when the region reports it. */
    private fun transition(intent: Intent, finish: () -> Unit) {
        val id = intent.identifier ?: return finish()
        val entered = intent.getBooleanExtra(LocationManager.KEY_PROXIMITY_ENTERING, false)
        val region = regions.get(id) ?: return finish()
        if (!region.optBoolean(if (entered) "onEntry" else "onExit", true)) return finish()
        events.transition(id, entered, finish)
    }

    @Suppress("DEPRECATION")
    private fun locations(intent: Intent): List<Location> =
        intent.getParcelableArrayExtra(LocationManager.KEY_LOCATIONS)?.filterIsInstance<Location>()
            ?: listOfNotNull(intent.getParcelableExtra(LocationManager.KEY_LOCATION_CHANGED))

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

    private fun permissionDenied() = TokamakPluginError("NotAllowedError", "Location permission was denied")

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
        when (error) {
            is SecurityException -> permissionDenied()
            // Off screen, Android starts a foreground service only in response to some events.
            is ForegroundServiceStartNotAllowedException -> TokamakPluginError.needsUI()
            else -> TokamakPluginError("NotReadableError", error.message ?: "Location is unavailable")
        }

    private companion object {
        /** The `timeout` that never expires, as the page sends it. */
        const val UNLIMITED = 0xffffffffL
        const val FINE = Manifest.permission.ACCESS_FINE_LOCATION
        const val COARSE = Manifest.permission.ACCESS_COARSE_LOCATION
        const val BACKGROUND = Manifest.permission.ACCESS_BACKGROUND_LOCATION

        /** Apple's significant-change service's thresholds: 500 metres or more, at most every 5 minutes, without GPS. */
        const val SIGNIFICANT_CHANGE_OPTIONS = """{"accuracy":"low","interval":300000,"distance":500}"""
    }
}
