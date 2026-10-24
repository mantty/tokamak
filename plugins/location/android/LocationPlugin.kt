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
import kotlin.coroutines.cancellation.CancellationException
import kotlin.coroutines.resume
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.suspendCancellableCoroutine

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

    private val updates = LocationUpdates(host, LocationUpdates.UPDATES)
    private val significantChanges = LocationUpdates(host, LocationUpdates.SIGNIFICANT_CHANGES)
    private val regions = LocationRegions(host)
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

    /**
     * Starts significant changes when [config] names them, and stops them otherwise. Without
     * permission they start once the app has it.
     */
    fun setup(config: SetupConfig) =
        located {
            when {
                config.significantChanges != true -> significantChanges.stop()
                hasForegroundPermission() -> significantChanges.start(SIGNIFICANT_CHANGES)
                else -> significantChanges.keep(SIGNIFICANT_CHANGES)
            }
        }

    /** The provider's last known position when it is no older than [maximumAge] milliseconds, or null. */
    fun recentPosition(maximumAge: Double): Position? = located { positions.recent(maximumAge)?.let(::Position) }

    /** A new fix, or null when Android finds none. */
    suspend fun currentPosition(): Position? = located { positions.current()?.let(::Position) }

    /** Each position as the device moves, until the provider turns off. */
    fun watchPosition(): Flow<Position> = positions.watch().map(::Position).catch { throw locationError(it) }

    /** The app's in-use and background permission, and the precision it gets, or would get once allowed. */
    fun permission(): LocationPermission {
        val inUse = if (hasForegroundPermission()) Permission.Granted else state(COARSE)
        val background = if (BACKGROUND in declared) state(BACKGROUND) else Permission.Denied
        val precise = granted(FINE) || (!granted(COARSE) && FINE in declared)
        return LocationPermission(inUse, background, if (precise) Precision.Precise else Precision.Approximate)
    }

    /**
     * Asks for foreground permission, then for background permission when [request] asks for it, as
     * Android grants background permission only to an app with foreground permission.
     */
    suspend fun requestPermission(request: PermissionRequest?): LocationPermission {
        val background = request?.background == true
        if (background && BACKGROUND !in declared) {
            throw TokamakPluginError.notSupported("Background location needs the location plugin's background build setting")
        }
        val asked = if (request?.precision == Precision.Approximate) setOf(COARSE) else foreground
        val allowed = ask(asked).containsValue(true)
        if (allowed) restore(updatesRemoved = false)
        if (background && allowed) ask(setOf(BACKGROUND))
        return permission()
    }

    fun servicesEnabled(): Boolean = manager.isLocationEnabled

    fun openSettings() {
        val app = Uri.fromParts("package", context.packageName, null)
        host.requireForegroundActivity().startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, app))
    }

    /** Starts updates in the foreground service when [options] name one, which stops otherwise. On failure, none run. */
    fun startUpdates(options: UpdateOptions?) {
        val running = options ?: UpdateOptions(null, null, null, null)
        val notification = running.foregroundService
        requirePermission()
        if (notification != null) requireService()
        try {
            if (notification == null) TokamakLocationService.stop(context) else TokamakLocationService.start(context, notification)
            updates.start(running)
        } catch (error: Exception) {
            stopUpdates()
            throw locationError(error)
        }
    }

    fun stopUpdates() {
        updates.stop()
        TokamakLocationService.stop(context)
    }

    fun getUpdates(): UpdateOptions? = updates.options

    /** Monitors a region, which needs precise location. */
    fun addRegion(region: Region) {
        if (!granted(FINE)) throw TokamakPluginError("NotAllowedError", "Regions need precise location")
        located { regions.add(region) }
    }

    fun getRegions(): List<MonitoredRegion> =
        regions.all().map { MonitoredRegion(it.id, it.latitude, it.longitude, it.radius, it.onEntry ?: true, it.onExit ?: true) }

    fun removeRegion(id: String) = regions.remove(id)

    fun onLocationUpdate(): Flow<LocationUpdate> = events.updates.stream()

    fun onRegionTransition(): Flow<RegionTransition> = events.transitions.stream()

    fun onLocationError(): Flow<LocationErrorEvent> = events.errors.stream()

    /** Asks the user for [permissions], returning whether each is granted. */
    private suspend fun ask(permissions: Set<String>): Map<String, Boolean> =
        suspendCancellableCoroutine { continuation -> host.requestPermissions(permissions) { continuation.resume(it) } }

    private fun granted(permission: String): Boolean =
        context.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED

    private fun hasForegroundPermission(): Boolean = granted(FINE) || granted(COARSE)

    private fun state(permission: String): Permission =
        when {
            granted(permission) -> Permission.Granted
            host.wouldPrompt(permission) -> Permission.Prompt
            else -> Permission.Denied
        }

    private fun requirePermission() {
        if (!hasForegroundPermission()) throw TokamakPluginError("NotAllowedError", "Location permission has not been granted")
    }

    private fun startService() {
        updates.options?.foregroundService?.let { TokamakLocationService.start(context, it) }
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
        events.update(locations(intent).map(::Position), finish)
    }

    /** Delivers a region's entry or exit, when the region reports it. */
    private fun transition(intent: Intent, finish: () -> Unit) {
        val id = intent.identifier ?: return finish()
        val entered = intent.getBooleanExtra(LocationManager.KEY_PROXIMITY_ENTERING, false)
        val region = regions.get(id) ?: return finish()
        if ((if (entered) region.onEntry else region.onExit) == false) return finish()
        events.transition(id, entered, finish)
    }

    @Suppress("DEPRECATION")
    private fun locations(intent: Intent): List<Location> =
        intent.getParcelableArrayExtra(LocationManager.KEY_LOCATIONS)?.filterIsInstance<Location>()
            ?: listOfNotNull(intent.getParcelableExtra(LocationManager.KEY_LOCATION_CHANGED))

    /** Runs [operation], translating its failure into the error the Worker receives. */
    private inline fun <T> located(operation: () -> T): T =
        try {
            operation()
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            throw locationError(error)
        }

    private fun permissionDenied() = TokamakPluginError("NotAllowedError", "Location permission was denied")

    private fun locationError(error: Throwable): TokamakPluginError =
        when (error) {
            is TokamakPluginError -> error
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
        val SIGNIFICANT_CHANGES = UpdateOptions(Accuracy.Low, distance = 500.0, interval = 300_000, foregroundService = null)
    }
}
