package com.tokamak.plugins.location

import com.tokamak.runtime.TokamakExporter
import com.tokamak.runtime.TokamakExports
import com.tokamak.runtime.TokamakOverflow

object LocationExports : TokamakExports<TokamakLocationPlugin> {
    override fun exports(e: TokamakExporter<TokamakLocationPlugin>) {
        e.call(TokamakLocationPlugin::startSignificantChanges, doc = "Starts significant changes of location, until `stopSignificantChanges`.")
        e.call(TokamakLocationPlugin::stopSignificantChanges)
        e.call(TokamakLocationPlugin::recentPosition, doc = "The provider's last known position, when it is no older than `maximumAge`.")
        e.call(TokamakLocationPlugin::currentPosition, doc = "A new fix, or null when Android finds none. Cancelling the call stops locating.")
        e.stream(TokamakLocationPlugin::watchPosition, overflow = TokamakOverflow.DROP_OLDEST, doc = "Positions as the device moves.")
        e.call(TokamakLocationPlugin::permission, doc = "The app's location permission, and the precision it gets.")
        e.call(TokamakLocationPlugin::requestPermission, doc = "Asks for the permission `request` names where the user has not answered.")
        e.call(TokamakLocationPlugin::servicesEnabled, doc = "Whether location services are on for the device.")
        e.call(TokamakLocationPlugin::openSettings, doc = "Opens the app's page in the system settings.")
        e.call(TokamakLocationPlugin::startUpdates, doc = "Starts continuous updates, replacing any running, until `stopUpdates`.")
        e.call(TokamakLocationPlugin::stopUpdates)
        e.call(TokamakLocationPlugin::getUpdates, doc = "The options of the running updates, or null when none are running.")
        e.call(TokamakLocationPlugin::addRegion, doc = "Monitors a circular region, replacing one with the same `id`.")
        e.call(TokamakLocationPlugin::getRegions, doc = "The monitored regions.")
        e.call(TokamakLocationPlugin::removeRegion)
        e.stream(TokamakLocationPlugin::onLocationUpdate, overflow = TokamakOverflow.DROP_OLDEST, doc = "The updates the Worker's `_location.onUpdate` receives.")
        e.stream(TokamakLocationPlugin::onRegionTransition, doc = "The transitions the Worker's `_location.onRegionTransition` receives.")
        e.stream(TokamakLocationPlugin::onLocationError, doc = "The failures the Worker's `_location.onError` receives.")
        e.event("onUpdate", LocationUpdate::class, doc = "Receives continuous updates and significant changes, including those that start the app.")
        e.event("onRegionTransition", RegionTransition::class, doc = "Receives each region entry or exit, including one that starts the app.")
        e.event("onError", LocationErrorEvent::class, doc = "Receives failures of background services, such as a region Android cannot monitor.")
    }
}
