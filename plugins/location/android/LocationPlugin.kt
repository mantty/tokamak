package com.tokamak.plugins.location

import android.Manifest
import android.app.ForegroundServiceStartNotAllowedException
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.location.Location
import android.location.LocationManager
import android.net.Uri
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
    private val positions = LocationPositions(context, manager)

    /** Whether the provider was on when last reported, as each request reports its changes. */
    private var providerEnabled = true

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

    override fun call(method: String, arguments: Any?, reply: TokamakPluginReply): (() -> Unit)? {
        when (method) {
            "setup" -> setup(requireObject(arguments), reply)
            "getCurrentPosition" -> return currentPosition(requireObject(arguments), reply)
            "permission" -> reply(Result.success(permission()))
            "requestPermission" -> requestPermission(requireObject(arguments), reply)
            "servicesEnabled" -> reply(Result.success(manager.isLocationEnabled))
            "openSettings" -> openSettings(reply)
            "startUpdates" -> startUpdates(requireObject(arguments), reply)
            "stopUpdates" -> {
                stopUpdates()
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
        return null
    }

    override fun subscribe(
        method: String,
        arguments: Any?,
        reply: TokamakPluginReply,
    ): () -> Unit =
        if (method == "watchPosition") watchPosition(reply) else events.listen(method, reply) ?: super.subscribe(method, arguments, reply)

    /** Replies with a position, or null when Android finds none, unless the returned cancellation runs first. */
    private fun currentPosition(options: JSONObject, reply: TokamakPluginReply): () -> Unit =
        runCatching {
            positions.current(options.optDouble("maximumAge", 0.0)) { reply(Result.success(it?.let(::position))) }
        }.getOrElse { throw locationError(it) }

    /** Replies with each position as the device moves, until the provider turns off or the returned release. */
    private fun watchPosition(reply: TokamakPluginReply): () -> Unit =
        runCatching {
            positions.watch({ reply(Result.success(position(it))) }) {
                reply(Result.failure(TokamakPluginError("NotReadableError", "Location provider is unavailable")))
            }
        }.getOrElse { throw locationError(it) }

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
                stopUpdates()
                reply(Result.failure(locationError(it)))
            }
    }

    private fun stopUpdates() {
        updates.stop()
        TokamakLocationService.stop(context)
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
        runCatching {
            when {
                !config.optBoolean("significantChanges") -> significantChanges.stop()
                hasForegroundPermission() -> significantChanges.start(options)
                else -> significantChanges.keep(options)
            }
        }.onSuccess { reply(Result.success(null)) }
            .onFailure { reply(Result.failure(locationError(it))) }
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
            val enabled = intent.getBooleanExtra(LocationManager.KEY_PROVIDER_ENABLED, true)
            val turnedOff = providerEnabled && !enabled
            providerEnabled = enabled
            if (!turnedOff) return finish()
            return events.fail(TokamakPluginError("NotReadableError", "The location provider was turned off"), finish)
        }
        events.update(locations(intent).map(::position), finish)
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

    private fun locationError(error: Throwable): TokamakPluginError =
        when (error) {
            is SecurityException -> permissionDenied()
            // Off screen, Android starts a foreground service only in response to some events.
            is ForegroundServiceStartNotAllowedException -> TokamakPluginError.needsUI()
            else -> TokamakPluginError("NotReadableError", error.message ?: "Location is unavailable")
        }

    private companion object {
        const val FINE = Manifest.permission.ACCESS_FINE_LOCATION
        const val COARSE = Manifest.permission.ACCESS_COARSE_LOCATION
        const val BACKGROUND = Manifest.permission.ACCESS_BACKGROUND_LOCATION

        /** Apple's significant-change service's thresholds: 500 metres or more, at most every 5 minutes, without GPS. */
        const val SIGNIFICANT_CHANGE_OPTIONS = """{"accuracy":"low","interval":300000,"distance":500}"""
    }
}
