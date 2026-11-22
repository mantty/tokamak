import TokamakPlugin

extension TokamakLocationPlugin: TokamakExports {
  public static func exports(_ e: inout TokamakExporter<TokamakLocationPlugin>) {
    e.call(
      startSignificantChanges,
      doc: "Starts significant changes of location, until `stopSignificantChanges`.")
    e.call(stopSignificantChanges)
    e.call(startVisits, doc: "Starts arrivals at and departures from places, until `stopVisits`.")
    e.call(stopVisits)
    e.call(
      recentPosition, doc: "Core Location's latest position, when it is no older than `maximumAge`."
    )
    e.call(currentPosition, doc: "A new fix. Cancelling the call stops locating.")
    e.stream(watchPosition, overflow: .dropOldest, doc: "Positions as the device moves.")
    e.call(
      permission, doc: "The app's location permission, and the precision it gets.")
    e.call(
      requestPermission,
      doc: "Asks for the permission `request` names where the user has not answered.")
    e.call(servicesEnabled, doc: "Whether location services are on for the device.")
    e.call(openSettings, doc: "Opens the app's page in the system settings.")
    e.call(
      startUpdates, doc: "Starts continuous updates, replacing any running, until `stopUpdates`.")
    e.call(stopUpdates)
    e.call(getUpdates, doc: "The options of the running updates, or null when none are running.")
    e.call(addRegion, doc: "Monitors a circular region, replacing one with the same `id`.")
    e.call(getRegions, doc: "The monitored regions.")
    e.call(removeRegion)
    e.stream(
      onLocationUpdate, overflow: .dropOldest,
      doc: "The updates the Worker's `_onLocationUpdate` receives.")
    e.stream(
      onRegionTransition, doc: "The transitions the Worker's `_onRegionTransition` receives.")
    e.stream(onVisit, doc: "The visits the Worker's `_onVisit` receives.")
    e.stream(onLocationError, doc: "The failures the Worker's `_onLocationError` receives.")
    e.event(
      "_onLocationUpdate", LocationUpdate.self,
      doc:
        "Receives continuous updates and significant changes, including those that launch the app."
    )
    e.event(
      "_onRegionTransition", RegionTransition.self,
      doc: "Receives each region entry or exit, including one that launches the app.")
    e.event(
      "_onVisit", Visit.self,
      doc:
        "Receives each arrival at or departure from a place, including one that launches the app."
    )
    e.event(
      "_onLocationError", LocationErrorEvent.self,
      doc:
        "Receives failures of background services, such as a region Core Location cannot monitor."
    )
  }
}
