import CoreLocation
import Foundation

final class TokamakLocationPlugin: NSObject, TokamakPlugin,
  CLLocationManagerDelegate
{
  let id = "location"

  private static let permissionDenied = TokamakPluginError(
    name: "NotAllowedError",
    message: "Location permission was denied"
  )
  private static let timedOut = TokamakPluginError(
    name: "TimeoutError",
    message: "Location was not found in time"
  )

  /// A current position request, timed from when locating starts.
  private struct Request {
    let reply: TokamakPluginReply
    let maximumAge: TimeInterval
    let timeout: TimeInterval
    var timer: DispatchWorkItem?
  }

  /// Updates watchers.
  private let updates = CLLocationManager()
  /// Takes single fixes for current position requests, apart from watching.
  private let fixes = CLLocationManager()
  private var current: [UUID: Request] = [:]
  private var watchers: [UUID: TokamakPluginReply] = [:]

  init(host: TokamakHost) {
    super.init()
    for manager in [updates, fixes] {
      manager.delegate = self
      manager.desiredAccuracy = kCLLocationAccuracyBest
    }
  }

  func call(
    method: String,
    arguments: Any,
    reply: @escaping TokamakPluginReply
  ) {
    guard method == "getCurrentPosition" else {
      reply(.failure(.notSupported("\(id).\(method) is not supported")))
      return
    }
    let options = arguments as? [String: Any] ?? [:]
    current[UUID()] = Request(
      reply: reply,
      maximumAge: (options["maximumAge"] as? Double ?? 0) / 1000,
      timeout: (options["timeout"] as? Double ?? Double(UInt32.max)) / 1000
    )
    start()
  }

  func subscribe(
    method: String,
    arguments: Any,
    reply: @escaping TokamakPluginReply
  ) -> (() -> Void) {
    guard method == "watchPosition" else {
      reply(.failure(.notSupported("\(id).\(method) is not supported")))
      return {}
    }
    let subscription = UUID()
    watchers[subscription] = reply
    start()
    return { [weak self] in
      self?.watchers.removeValue(forKey: subscription)
      if self?.watchers.isEmpty == true {
        self?.updates.stopUpdatingLocation()
      }
    }
  }

  /// Also called when each manager is created, before anything asks for a
  /// position. Both managers report the app's one authorization.
  func locationManagerDidChangeAuthorization(
    _ manager: CLLocationManager
  ) {
    guard manager === updates, !current.isEmpty || !watchers.isEmpty else { return }
    serve(manager.authorizationStatus)
  }

  func locationManager(
    _ manager: CLLocationManager,
    didUpdateLocations locations: [CLLocation]
  ) {
    guard let location = locations.last else { return }
    deliver(.success(position(location)), from: manager)
  }

  /// A fix that is not yet available is taken again; updates continue.
  func locationManager(
    _ manager: CLLocationManager,
    didFailWithError error: Error
  ) {
    guard (error as NSError).code == CLError.Code.locationUnknown.rawValue else {
      deliver(.failure(locationError(error)), from: manager)
      return
    }
    if manager === fixes && !current.isEmpty {
      fixes.requestLocation()
    }
  }

  /// Checks location services off the main thread, which the check can block.
  private func start() {
    DispatchQueue.global(qos: .userInitiated).async {
      let enabled = CLLocationManager.locationServicesEnabled()
      DispatchQueue.main.async { self.start(servicesEnabled: enabled) }
    }
  }

  private func start(servicesEnabled: Bool) {
    guard servicesEnabled else {
      fail(unavailable("Location services are disabled"))
      return
    }
    let status = updates.authorizationStatus
    if status == .notDetermined {
      updates.requestWhenInUseAuthorization()
    } else {
      serve(status)
    }
  }

  /// Locates for the waiting requests and watchers when `status` allows it,
  /// and fails them when it forbids it.
  private func serve(_ status: CLAuthorizationStatus) {
    switch status {
    case .authorizedAlways, .authorizedWhenInUse:
      if !watchers.isEmpty {
        updates.startUpdatingLocation()
      }
      locate()
    case .denied, .restricted:
      fail(Self.permissionDenied)
    case .notDetermined:
      break
    @unknown default:
      fail(.notSupported("Location authorization is not supported"))
    }
  }

  /// Answers each new request with a fix no older than its `maximumAge`, and
  /// takes a fix for the rest, each failing after its `timeout`.
  private func locate() {
    for (key, request) in current where request.timer == nil {
      if let fix = fixes.location, -fix.timestamp.timeIntervalSinceNow <= request.maximumAge {
        finish(key, .success(position(fix)))
        continue
      }
      let timer = DispatchWorkItem { [weak self] in self?.finish(key, .failure(Self.timedOut)) }
      current[key]?.timer = timer
      DispatchQueue.main.asyncAfter(deadline: .now() + request.timeout, execute: timer)
    }
    if !current.isEmpty {
      fixes.requestLocation()
    }
  }

  /// Answers the waiting requests with a fix, or the watchers with an update.
  private func deliver(_ result: Result<Any?, TokamakPluginError>, from manager: CLLocationManager)
  {
    guard manager === fixes else {
      for reply in watchers.values {
        reply(result)
      }
      return
    }
    for key in current.keys {
      finish(key, result)
    }
  }

  private func fail(_ error: TokamakPluginError) {
    deliver(.failure(error), from: fixes)
    deliver(.failure(error), from: updates)
  }

  private func finish(_ key: UUID, _ result: Result<Any?, TokamakPluginError>) {
    guard let request = current.removeValue(forKey: key) else { return }
    request.timer?.cancel()
    request.reply(result)
    if current.isEmpty {
      fixes.stopUpdatingLocation()
    }
  }

  private func locationError(_ error: Error) -> TokamakPluginError {
    if (error as NSError).code == CLError.Code.denied.rawValue {
      return Self.permissionDenied
    }
    return unavailable(error.localizedDescription)
  }

  private func unavailable(_ message: String) -> TokamakPluginError {
    TokamakPluginError(name: "NotReadableError", message: message)
  }

  private func position(_ location: CLLocation) -> [String: Any] {
    let coordinates = location.coordinate
    return [
      "coords": [
        "latitude": coordinates.latitude,
        "longitude": coordinates.longitude,
        "accuracy": location.horizontalAccuracy,
        "altitude": nullable(
          location.verticalAccuracy >= 0,
          location.altitude
        ),
        "altitudeAccuracy": nullable(
          location.verticalAccuracy >= 0,
          location.verticalAccuracy
        ),
        "heading": nullable(location.course >= 0, location.course),
        "speed": nullable(location.speed >= 0, location.speed),
      ],
      "timestamp": location.timestamp.timeIntervalSince1970 * 1000,
    ]
  }

  private func nullable(_ available: Bool, _ value: Double) -> Any {
    available ? value : NSNull()
  }
}
