import CoreLocation
import Foundation
import TokamakPlugin

/// The JSON of the running updates' options.
private let updatesKey = "tokamak.location.updates"

/// Continuous updates from `CLLocationUpdate.liveUpdates`. iOS launches an app
/// it ended while they ran for the next update, so the options persist until
/// `stop`, and `resume` starts them again at launch. Use it on the main thread.
final class TokamakLocationUpdates {
  private let events: TokamakLocationEvents
  private let defaults = UserDefaults.standard
  private var task: Task<Void, Never>?
  #if os(iOS)
    /// Keeps the app in use while updates run off screen, with the location
    /// indicator.
    private var session: CLBackgroundActivitySession?
  #endif

  init(events: TokamakLocationEvents) {
    self.events = events
  }

  /// The options of the running updates, or nil.
  var options: TokamakLocationPlugin.UpdateOptions? {
    defaults.data(forKey: updatesKey)
      .flatMap { try? JSONDecoder().decode(TokamakLocationPlugin.UpdateOptions.self, from: $0) }
  }

  /// Starts updates with `options`, replacing any running.
  func start(_ options: TokamakLocationPlugin.UpdateOptions) throws(TokamakPluginError) {
    let configuration = try Self.configuration(options.activity ?? .default)
    stop()
    defaults.set(try? JSONEncoder().encode(options), forKey: updatesKey)
    run(configuration)
  }

  /// Starts the updates that ran when the app last ended. The system keeps
  /// their session for only a few seconds after it launches the app, so call
  /// it before launch finishes.
  func resume() {
    guard let options, let configuration = try? Self.configuration(options.activity ?? .default)
    else { return }
    run(configuration)
  }

  func stop() {
    defaults.removeObject(forKey: updatesKey)
    task?.cancel()
    task = nil
    #if os(iOS)
      session?.invalidate()
      session = nil
    #endif
    events.report([])
  }

  private func run(_ configuration: CLLocationUpdate.LiveConfiguration) {
    #if os(iOS)
      if Self.declaresBackgroundMode {
        session = CLBackgroundActivitySession()
      }
    #endif
    task = events.task { [weak self] in
      for try await update in CLLocationUpdate.liveUpdates(configuration) {
        self?.receive(update)
      }
    }
  }

  private func receive(_ update: CLLocationUpdate) {
    guard #available(iOS 18, macOS 15, *) else {
      if let location = update.location {
        events.update(location)
      }
      return
    }
    events.report(Self.problems(update))
    if update.location != nil || update.stationary {
      events.update(update.location, stationary: update.stationary)
    }
  }

  /// The problems that suspend updates, which `update` names.
  @available(iOS 18, macOS 15, *)
  private static func problems(_ update: CLLocationUpdate) -> [TokamakPluginError] {
    let flags = [
      (update.authorizationDenied, "NotAllowedError", "Location permission was denied"),
      (update.authorizationDeniedGlobally, "NotAllowedError", "Location services are off"),
      (update.authorizationRestricted, "NotAllowedError", "Location is restricted on this device"),
      (
        update.insufficientlyInUse, "NotAllowedError",
        "Updates need the app in use, or Always permission"
      ),
      (update.accuracyLimited, "NotAllowedError", "Approximate location limits updates"),
      (update.locationUnavailable, "NotReadableError", "Location is unavailable"),
    ]
    return flags.filter(\.0).map { TokamakPluginError(name: $0.1, message: $0.2) }
  }

  /// The live update configuration of `activity`.
  private static func configuration(
    _ activity: TokamakLocationPlugin.Activity
  ) throws(TokamakPluginError) -> CLLocationUpdate.LiveConfiguration {
    switch activity {
    case .default: return .default
    case .automotiveNavigation: return .automotiveNavigation
    case .otherNavigation: return .otherNavigation
    case .fitness: return .fitness
    case .airborne: return .airborne
    case .maritime:
      // The SDKs that have `maritime` ship with Swift 6.4.
      #if compiler(>=6.4)
        if #available(iOS 27, macOS 27, *) {
          return .maritime
        }
      #endif
      throw .notSupported("The maritime activity needs iOS 27 or macOS 27")
    }
  }

  #if os(iOS)
    /// Whether the app declares the `location` background mode, as the
    /// `background-updates` build setting does.
    private static let declaresBackgroundMode =
      (Bundle.main.object(forInfoDictionaryKey: "UIBackgroundModes") as? [String])?
      .contains("location") == true
  #endif
}
