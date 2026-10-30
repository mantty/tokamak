import CoreLocation
import Foundation
import TokamakPlugin

#if os(iOS)
  import UIKit
#endif

/// The JSON of each monitored region, by id.
private let regionsKey = "tokamak.location.regions"
/// The ids of the regions whose last transition was an entry.
private let insideKey = "tokamak.location.inside"
/// `CLMonitor` monitors at most 20 conditions for an app, and reports no
/// error beyond them.
private let regionLimit = 20
/// The monitor's name, which must be alphanumeric.
private let monitorName = "TokamakLocation"

/// Circular regions in one `CLMonitor`, which the plugin creates at launch
/// and reads events from for the life of the process. The system launches the
/// app for their events on iOS. Use it on the main thread.
final class TokamakLocationRegions {
  private let events: TokamakLocationEvents
  private let defaults = UserDefaults.standard
  private let monitor: Task<CLMonitor, Never>

  init(events: TokamakLocationEvents) {
    self.events = events
    let monitor = Task { @MainActor in
      await Self.firstUnlock()
      return await CLMonitor(monitorName)
    }
    self.monitor = monitor
    events.task { [weak self] in
      for try await event in await monitor.value.events {
        self?.receive(event)
      }
    }
  }

  /// The regions, by id.
  private var regions: [String: TokamakLocationPlugin.Region] {
    get {
      defaults.data(forKey: regionsKey)
        .flatMap {
          try? JSONDecoder().decode([String: TokamakLocationPlugin.Region].self, from: $0)
        } ?? [:]
    }
    set {
      defaults.set(try? JSONEncoder().encode(newValue), forKey: regionsKey)
    }
  }

  private var inside: Set<String> {
    get { Set(defaults.stringArray(forKey: insideKey) ?? []) }
    set { defaults.set(Array(newValue), forKey: insideKey) }
  }

  /// Monitors `region`, replacing one with the same id. It starts outside, so
  /// a device already inside reports an entry, as on Android. `CLMonitor`
  /// keeps the state of a condition added again under its identifier, so the
  /// old condition is removed first.
  func add(_ region: TokamakLocationPlugin.Region) async throws(TokamakPluginError) {
    let id = region.id
    let monitor = await monitor.value
    let monitored = await monitor.identifiers
    guard monitored.count < regionLimit || monitored.contains(id) else {
      throw TokamakPluginError(
        name: "QuotaExceededError",
        message: "The app monitors \(regionLimit) regions, the most the system allows")
    }
    let center = CLLocationCoordinate2D(latitude: region.latitude, longitude: region.longitude)
    let condition = CLMonitor.CircularGeographicCondition(center: center, radius: region.radius)
    regions[id] = region
    inside.remove(id)
    await monitor.remove(id)
    await monitor.add(condition, identifier: id, assuming: .unsatisfied)
  }

  func remove(_ id: String) async {
    regions[id] = nil
    inside.remove(id)
    events.report([], region: id)
    await monitor.value.remove(id)
  }

  /// The monitored regions, each with whether the device is inside it.
  func all() async -> [TokamakLocationPlugin.MonitoredRegion] {
    let monitor = await monitor.value
    let regions = regions
    var all: [TokamakLocationPlugin.MonitoredRegion] = []
    for id in await monitor.identifiers {
      guard let region = regions[id] else { continue }
      let state: TokamakLocationPlugin.RegionState =
        switch await monitor.record(for: id)?.lastEvent.state {
        case .satisfied: .inside
        case .unsatisfied: .outside
        default: .unknown
        }
      all.append(
        TokamakLocationPlugin.MonitoredRegion(
          id: id, latitude: region.latitude, longitude: region.longitude, radius: region.radius,
          onEntry: region.onEntry ?? true, onExit: region.onExit ?? true, state: state))
    }
    return all
  }

  /// Reports the event's problems, and its transition when it changes whether the device is
  /// inside the region, as events can repeat the state.
  private func receive(_ event: CLMonitor.Event) {
    let id = event.identifier
    if #available(iOS 18, macOS 15, *) {
      events.report(Self.problems(event), region: id)
    }
    guard let region = regions[id], event.state == .satisfied || event.state == .unsatisfied
    else { return }
    let entered = event.state == .satisfied
    guard entered != inside.contains(id) else { return }
    inside = entered ? inside.union([id]) : inside.subtracting([id])
    if (entered ? region.onEntry : region.onExit) != false {
      events.transition(id, entered: entered, at: event.date)
    }
  }

  /// The problems that stop monitoring the region, which `event` names.
  @available(iOS 18, macOS 15, *)
  private static func problems(_ event: CLMonitor.Event) -> [TokamakPluginError] {
    let flags = [
      (event.authorizationDenied, "NotAllowedError", "Location permission was denied"),
      (event.authorizationDeniedGlobally, "NotAllowedError", "Location services are off"),
      (event.authorizationRestricted, "NotAllowedError", "Location is restricted on this device"),
      (
        event.insufficientlyInUse, "NotAllowedError",
        "Regions need the app in use, or Always permission"
      ),
      (event.accuracyLimited, "NotAllowedError", "Regions need precise location"),
      (event.conditionUnsupported, "NotSupportedError", "The device cannot monitor the region"),
      (
        event.conditionLimitExceeded, "QuotaExceededError",
        "The app monitors the most regions the system allows"
      ),
      (
        event.persistenceUnavailable, "NotReadableError",
        "The system cannot store the app's regions"
      ),
    ]
    return flags.filter(\.0).map { TokamakPluginError(name: $0.1, message: $0.2) }
  }

  /// Returns once the device has been unlocked since it booted, from when `CLMonitor` can read the
  /// file that keeps its conditions, even while the device is locked.
  @MainActor
  private static func firstUnlock() async {
    #if os(iOS)
      let unlocked = NotificationCenter.default.notifications(
        named: UIApplication.protectedDataDidBecomeAvailableNotification)
      guard !unlockedSinceBoot() else { return }
      for await _ in unlocked {
        return
      }
    #endif
  }

  #if os(iOS)
    /// Whether the device has been unlocked since it booted, as only then can a file protected
    /// until the first unlock be written.
    private static func unlockedSinceBoot() -> Bool {
      let probe = FileManager.default.temporaryDirectory.appending(path: "TokamakLocation.unlocked")
      let protection = Data.WritingOptions.completeFileProtectionUntilFirstUserAuthentication
      return (try? Data().write(to: probe, options: protection)) != nil
    }
  #endif
}
