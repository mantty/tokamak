package com.tokamak.plugins.location

import android.location.Location
import com.tokamak.runtime.TokamakValue
import org.json.JSONObject

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

enum class Transition { enter, exit }

data class RegionTransition(val id: String, val transition: Transition, val timestamp: Long)

data class LocationErrorEvent(val name: String, val message: String)

enum class Permission { granted, denied, prompt }

enum class Precision { precise, approximate }

data class LocationPermission(val inUse: Permission, val background: Permission, val precision: Precision)

data class PermissionRequest(val background: Boolean?, val precision: Precision?)

data class SetupConfig(val significantChanges: Boolean?)

/** The request's quality, precision traded against battery use. */
enum class Accuracy { high, balanced, low }

/** The foreground service's notification, set from JavaScript so apps can translate it. */
data class ServiceNotification(val title: String, val body: String, val channel: String)

data class UpdateOptions(
    val accuracy: Accuracy?,
    /** Metres the device moves before the next update. */
    val distance: Double?,
    /** Milliseconds between updates. */
    val interval: Long?,
    val foregroundService: ServiceNotification?,
) {
    fun json(): JSONObject {
        val notification =
            foregroundService?.let { JSONObject().put("title", it.title).put("body", it.body).put("channel", it.channel) }
        return JSONObject()
            .putOpt("accuracy", accuracy?.name)
            .putOpt("distance", distance)
            .putOpt("interval", interval)
            .putOpt("foregroundService", notification)
    }

    companion object {
        /** The options [json] holds, as [json] wrote them. */
        fun from(json: String): UpdateOptions {
            val options = TokamakValue(JSONObject(json), "")
            return UpdateOptions(
                accuracy = options["accuracy"].nullable { it.enum(Accuracy.entries) },
                distance = options["distance"].nullable { it.double() },
                interval = options["interval"].nullable { it.long() },
                foregroundService =
                    options["foregroundService"].nullable {
                        ServiceNotification(it["title"].string(), it["body"].string(), it["channel"].string())
                    },
            )
        }
    }
}

data class Region(
    val id: String,
    val latitude: Double,
    val longitude: Double,
    /** In metres. */
    val radius: Double,
    val onEntry: Boolean?,
    val onExit: Boolean?,
) {
    fun json(): JSONObject =
        JSONObject()
            .put("id", id)
            .put("latitude", latitude)
            .put("longitude", longitude)
            .put("radius", radius)
            .putOpt("onEntry", onEntry)
            .putOpt("onExit", onExit)

    companion object {
        /** The region [json] holds, as [json] wrote it. */
        fun from(json: String): Region {
            val region = TokamakValue(JSONObject(json), "")
            return Region(
                id = region["id"].string(),
                latitude = region["latitude"].double(),
                longitude = region["longitude"].double(),
                radius = region["radius"].double(),
                onEntry = region["onEntry"].nullable { it.boolean() },
                onExit = region["onExit"].nullable { it.boolean() },
            )
        }
    }
}

data class MonitoredRegion(
    val id: String,
    val latitude: Double,
    val longitude: Double,
    val radius: Double,
    val onEntry: Boolean,
    val onExit: Boolean,
)
