package com.tokamak.plugins.location

import android.location.Location
import android.location.LocationRequest

data class Coordinates(
    val latitude: Double,
    val longitude: Double,
    val accuracy: Double,
    val altitude: Double?,
    val altitudeAccuracy: Double?,
    val heading: Double?,
    val speed: Double?,
)

data class Position(val coords: Coordinates, val timestamp: Long) {
    constructor(location: Location) : this(
        Coordinates(
            latitude = location.latitude,
            longitude = location.longitude,
            accuracy = location.accuracy.toDouble(),
            altitude = location.altitude.takeIf { location.hasAltitude() },
            altitudeAccuracy = location.verticalAccuracyMeters.toDouble().takeIf { location.hasVerticalAccuracy() },
            heading = location.bearing.toDouble().takeIf { location.hasBearing() },
            speed = location.speed.toDouble().takeIf { location.hasSpeed() },
        ),
        location.time,
    )
}

data class LocationUpdate(val positions: List<Position>)

enum class Transition { Enter, Exit }

data class RegionTransition(val id: String, val transition: Transition, val timestamp: Long)

data class LocationErrorEvent(val name: String, val message: String)

enum class Permission { Granted, Denied, Prompt }

enum class Precision { Precise, Approximate }

data class LocationPermission(val inUse: Permission, val backgroundAlways: Permission, val precision: Precision)

data class PermissionRequest(val backgroundAlways: Boolean?, val precision: Precision?)

data class SetupConfig(val significantChanges: Boolean?)

/** The request's quality, precision traded against battery use, as `LocationRequest.QUALITY_*` [quality] gives it. */
enum class Accuracy(val quality: Int) {
    High(LocationRequest.QUALITY_HIGH_ACCURACY),
    Balanced(LocationRequest.QUALITY_BALANCED_POWER_ACCURACY),
    Low(LocationRequest.QUALITY_LOW_POWER),
}

/** The foreground service's notification, set from JavaScript so apps can translate it. */
data class ServiceNotification(val title: String, val body: String, val channel: String)

data class UpdateOptions(
    val accuracy: Accuracy?,
    /** Metres the device moves before the next update. */
    val distance: Double?,
    /** Milliseconds between updates. */
    val interval: Long?,
    val foregroundService: ServiceNotification?,
)

data class Region(
    val id: String,
    val latitude: Double,
    val longitude: Double,
    /** In metres. */
    val radius: Double,
    val onEntry: Boolean?,
    val onExit: Boolean?,
)

data class MonitoredRegion(
    val id: String,
    val latitude: Double,
    val longitude: Double,
    val radius: Double,
    val onEntry: Boolean,
    val onExit: Boolean,
)
