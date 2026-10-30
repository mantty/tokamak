import CoreLocation
import Foundation
import TokamakPlugin

/// How long the Worker's listeners have for an event, within the few seconds
/// the system gives an app it launches for one.
private let eventDeadline: TimeInterval = 8

/// Delivers the plugin's events to the Worker's listeners and the page's. Use
/// it on the main thread.
final class TokamakLocationEvents {
  let updates = TokamakListeners<TokamakLocationPlugin.LocationUpdate>()
  let transitions = TokamakListeners<TokamakLocationPlugin.RegionTransition>()
  let visits = TokamakListeners<TokamakLocationPlugin.Visit>()
  let errors = TokamakListeners<TokamakLocationPlugin.LocationErrorEvent>()

  private let host: TokamakHost
  /// Positions that arrived while the Worker handled the previous update.
  private var positions: [TokamakLocationPlugin.Position] = []
  private var stationary: Bool?
  private var updating = false
  /// The time of the newest position delivered, as services that run together
  /// report the same positions.
  private var latest = Date.distantPast
  /// The messages of the problems last reported, by region id, or nil for
  /// updates.
  private var problems: [String?: Set<String>] = [:]

  init(host: TokamakHost) {
    self.host = host
  }

  /// Delivers `location` when it is newer than the positions delivered, with
  /// Apple's `stationary` flag when the system reports it. Positions that
  /// arrive while the Worker handles an update reach it together as the next.
  func update(_ location: CLLocation?, stationary: Bool? = nil) {
    if let location, location.timestamp > latest {
      latest = location.timestamp
      positions.append(TokamakLocationPlugin.Position(location))
    }
    if let stationary {
      self.stationary = stationary
    }
    if !updating {
      sendUpdate()
    }
  }

  private func sendUpdate() {
    guard !positions.isEmpty || stationary == true else {
      updating = false
      return
    }
    let update = TokamakLocationPlugin.LocationUpdate(positions: positions, stationary: stationary)
    positions = []
    stationary = nil
    updating = true
    send(update, to: updates, as: "update") { self.sendUpdate() }
  }

  func transition(_ id: String, entered: Bool, at date: Date) {
    send(
      TokamakLocationPlugin.RegionTransition(id: id, entered: entered, at: date), to: transitions,
      as: "region")
  }

  func visit(_ visit: CLVisit) {
    send(TokamakLocationPlugin.Visit(visit), to: visits, as: "visit")
  }

  /// Reports each of `problems` that updates, or the region `id`, did not
  /// report last time.
  func report(_ problems: [TokamakPluginError], region id: String? = nil) {
    let previous = self.problems[id] ?? []
    for problem in problems where !previous.contains(problem.message) {
      fail(problem, region: id)
    }
    self.problems[id] = Set(problems.map(\.message))
  }

  /// A main-actor task that runs `body`, reporting the failure that ends it
  /// unless the task was cancelled.
  @discardableResult
  func task(_ body: @escaping @MainActor () async throws -> Void) -> Task<Void, Never> {
    Task { @MainActor in
      do {
        try await body()
      } catch {
        guard !Task.isCancelled else { return }
        self.fail(.locationUnavailable(error.localizedDescription))
      }
    }
  }

  /// Reports a background service's failure, which concerns the region `id`
  /// when given.
  func fail(_ error: TokamakPluginError, region id: String? = nil) {
    send(
      TokamakLocationPlugin.LocationErrorEvent(name: error.name, message: error.message, id: id),
      to: errors, as: "error")
  }

  /// Delivers `event` to the page's `listeners` and the Worker's listeners of
  /// `name`, then calls `completion`.
  private func send<Event: Encodable>(
    _ event: Event, to listeners: TokamakListeners<Event>, as name: String,
    completion: @escaping () -> Void = {}
  ) {
    listeners.send(event)
    host.emit(name, event: event, timeout: eventDeadline) { result in
      if case .failure(let error) = result {
        print("location \(name) failed: \(error)")
      }
      completion()
    }
  }
}

extension TokamakPluginError {
  /// Location could not be found, as `message` says.
  static func locationUnavailable(_ message: String) -> Self {
    Self(name: "NotReadableError", message: message)
  }

  /// Core Location's `error`: `NotAllowedError` without permission, and
  /// `NotReadableError` otherwise.
  static func location(_ error: Error) -> Self {
    if (error as NSError).code == CLError.Code.denied.rawValue {
      return Self(name: "NotAllowedError", message: "Location permission was denied")
    }
    return .locationUnavailable(error.localizedDescription)
  }
}
