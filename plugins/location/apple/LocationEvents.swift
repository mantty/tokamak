import CoreLocation
import Foundation

/// How long the Worker's listeners have for an event, within the few seconds
/// the system gives an app it launches for one.
private let eventDeadline: TimeInterval = 8

/// The listener methods, which receive the Worker's events.
private let listenerMethods: Set = [
  "onLocationUpdate", "onRegionTransition", "onVisit", "onLocationError",
]

/// Delivers the plugin's events to the Worker's listeners and the page's. Use
/// it on the main thread.
final class TokamakLocationEvents {
  private let host: TokamakHost
  private var listeners: [String: [UUID: TokamakPluginReply]] = [:]
  /// Positions that arrived while the Worker handled the previous update.
  private var positions: [[String: Any]] = []
  private var stationary: Bool?
  private var updating = false
  /// The messages of the problems last reported, by region id, or nil for
  /// updates.
  private var problems: [String?: Set<String>] = [:]

  init(host: TokamakHost) {
    self.host = host
  }

  /// Adds a page's or Worker's listener of `method`, returning its removal.
  func listen(_ method: String, _ reply: @escaping TokamakPluginReply) -> (() -> Void)? {
    guard listenerMethods.contains(method) else { return nil }
    let key = UUID()
    listeners[method, default: [:]][key] = reply
    return { [weak self] in
      self?.listeners[method]?.removeValue(forKey: key)
    }
  }

  /// Delivers `location`, with Apple's `stationary` flag when the system
  /// reports it. Positions that arrive while the Worker handles an update
  /// reach it together as the next.
  func update(_ location: CLLocation?, stationary: Bool? = nil) {
    positions += location.map { [Self.position($0)] } ?? []
    self.stationary = stationary
    if !updating {
      sendUpdate()
    }
  }

  private func sendUpdate() {
    guard !positions.isEmpty || stationary == true else {
      updating = false
      return
    }
    var event: [String: Any] = ["positions": positions]
    event["stationary"] = stationary
    positions = []
    stationary = nil
    updating = true
    send("onLocationUpdate", "update", event) { self.sendUpdate() }
  }

  func transition(_ id: String, entered: Bool, at date: Date) {
    let event: [String: Any] = [
      "id": id, "transition": entered ? "enter" : "exit", "timestamp": Self.milliseconds(date),
    ]
    send("onRegionTransition", "region", event)
  }

  func visit(_ visit: CLVisit) {
    let event: [String: Any] = [
      "latitude": visit.coordinate.latitude,
      "longitude": visit.coordinate.longitude,
      "accuracy": visit.horizontalAccuracy,
      "arrival": visit.arrivalDate == .distantPast
        ? NSNull() : Self.milliseconds(visit.arrivalDate),
      "departure": visit.departureDate == .distantFuture
        ? NSNull() : Self.milliseconds(visit.departureDate),
    ]
    send("onVisit", "visit", event)
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

  /// Reports a background service's failure, which concerns the region `id`
  /// when given.
  func fail(_ error: TokamakPluginError, region id: String? = nil) {
    var event: [String: Any] = ["name": error.name, "message": error.message]
    event["id"] = id
    send("onLocationError", "error", event)
  }

  private func send(
    _ method: String, _ name: String, _ event: [String: Any], completion: @escaping () -> Void = {}
  ) {
    for reply in (listeners[method] ?? [:]).values {
      reply(.success(event))
    }
    host.emit(name, event: event, timeout: eventDeadline) { result in
      if case .failure(let error) = result {
        print("location \(name) failed: \(error)")
      }
      completion()
    }
  }

  /// `location` as the page's `Position`.
  static func position(_ location: CLLocation) -> [String: Any] {
    let coordinates = location.coordinate
    return [
      "coords": [
        "latitude": coordinates.latitude,
        "longitude": coordinates.longitude,
        "accuracy": location.horizontalAccuracy,
        "altitude": nullable(location.verticalAccuracy >= 0, location.altitude),
        "altitudeAccuracy": nullable(location.verticalAccuracy >= 0, location.verticalAccuracy),
        "heading": nullable(location.course >= 0, location.course),
        "speed": nullable(location.speed >= 0, location.speed),
      ],
      "timestamp": milliseconds(location.timestamp),
    ]
  }

  private static func nullable(_ available: Bool, _ value: Double) -> Any {
    available ? value : NSNull()
  }

  private static func milliseconds(_ date: Date) -> Double {
    date.timeIntervalSince1970 * 1000
  }
}

extension TokamakPluginError {
  /// Location could not be found, as `message` says.
  static func locationUnavailable(_ message: String) -> Self {
    Self(name: "NotReadableError", message: message)
  }
}
