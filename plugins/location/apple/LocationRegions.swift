import CoreLocation
import Foundation

#if os(iOS)
  import UIKit
#endif

/// The JSON of each monitored region, as `addRegion` received it, by id.
private let regionsKey = "tokamak.location.regions"
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
    Task { @MainActor [weak self] in
      do {
        for try await event in await monitor.value.events {
          self?.receive(event)
        }
      } catch {
        self?.events.fail(
          .locationUnavailable(error.localizedDescription))
      }
    }
  }

  /// The regions, by id.
  private var regions: [String: [String: Any]] {
    get {
      defaults.data(forKey: regionsKey)
        .flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: [String: Any]] } ?? [:]
    }
    set {
      defaults.set(try? JSONSerialization.data(withJSONObject: newValue), forKey: regionsKey)
    }
  }

  /// Monitors the region `region` describes, replacing one with the same id.
  /// It starts outside, so a device already inside reports an entry, as on
  /// Android.
  func add(_ region: [String: Any]) async throws(TokamakPluginError) {
    guard let id = region["id"] as? String, let radius = region["radius"] as? Double,
      let latitude = region["latitude"] as? Double, let longitude = region["longitude"] as? Double
    else {
      throw TokamakPluginError(
        name: "TypeError", message: "region must have an id, latitude, longitude and radius")
    }
    let monitor = await monitor.value
    let monitored = await monitor.identifiers
    guard monitored.count < regionLimit || monitored.contains(id) else {
      throw TokamakPluginError(
        name: "QuotaExceededError",
        message: "The app monitors \(regionLimit) regions, the most the system allows")
    }
    let center = CLLocationCoordinate2D(latitude: latitude, longitude: longitude)
    let condition = CLMonitor.CircularGeographicCondition(center: center, radius: radius)
    regions[id] = region
    await monitor.add(condition, identifier: id, assuming: .unsatisfied)
  }

  func remove(_ id: String) async {
    regions[id] = nil
    events.report([], region: id)
    await monitor.value.remove(id)
  }

  /// The monitored regions, each with whether the device is inside it.
  func all() async -> [[String: Any]] {
    let monitor = await monitor.value
    let regions = regions
    var all: [[String: Any]] = []
    for id in await monitor.identifiers {
      guard var region = regions[id] else { continue }
      region["state"] =
        switch await monitor.record(for: id)?.lastEvent.state {
        case .satisfied: "inside"
        case .unsatisfied: "outside"
        default: "unknown"
        }
      all.append(region)
    }
    return all
  }

  private func receive(_ event: CLMonitor.Event) {
    let id = event.identifier
    if #available(iOS 18, macOS 15, *) {
      events.report(Self.problems(event), region: id)
    }
    guard let region = regions[id] else { return }
    switch event.state {
    case .satisfied where region["onEntry"] as? Bool != false:
      events.transition(id, entered: true, at: event.date)
    case .unsatisfied where region["onExit"] as? Bool != false:
      events.transition(id, entered: false, at: event.date)
    default:
      break
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

  /// Returns once the device has been unlocked since it started, as `CLMonitor` keeps its
  /// conditions in a file that data protection makes readable from then. The system launches the
  /// app for their events while the device is locked.
  @MainActor
  private static func firstUnlock() async {
    #if os(iOS)
      let unlocked = NotificationCenter.default.notifications(
        named: UIApplication.protectedDataDidBecomeAvailableNotification)
      guard !unlockedSinceStart() else { return }
      for await _ in unlocked {
        return
      }
    #endif
  }

  #if os(iOS)
    /// Whether the device has been unlocked since it started, as only then can a file protected
    /// until the first unlock be written.
    private static func unlockedSinceStart() -> Bool {
      let probe = FileManager.default.temporaryDirectory.appending(path: "TokamakLocation.unlocked")
      let protection = Data.WritingOptions.completeFileProtectionUntilFirstUserAuthentication
      return (try? Data().write(to: probe, options: protection)) != nil
    }
  #endif
}
