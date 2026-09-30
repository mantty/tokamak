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

  private let manager = CLLocationManager()
  private var current: [TokamakPluginReply] = []
  private var watchers: [UUID: TokamakPluginReply] = [:]

  init(host: TokamakHost) {
    super.init()
    manager.delegate = self
    manager.desiredAccuracy = kCLLocationAccuracyBest
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
    current.append(reply)
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
      self?.stopIfIdle()
    }
  }

  func locationManagerDidChangeAuthorization(
    _ manager: CLLocationManager
  ) {
    startIfAuthorized(manager.authorizationStatus)
  }

  func locationManager(
    _ manager: CLLocationManager,
    didUpdateLocations locations: [CLLocation]
  ) {
    guard let location = locations.last else { return }
    deliver(.success(position(location)))
  }

  func locationManager(
    _ manager: CLLocationManager,
    didFailWithError error: Error
  ) {
    if (error as NSError).code == CLError.Code.locationUnknown.rawValue {
      return
    }
    fail(locationError(error))
  }

  private func start() {
    guard CLLocationManager.locationServicesEnabled() else {
      fail(unavailable("Location services are disabled"))
      return
    }
    let status = manager.authorizationStatus
    if status == .notDetermined {
      manager.requestWhenInUseAuthorization()
    } else {
      startIfAuthorized(status)
    }
  }

  /// Starts updates when `status` allows them and fails every request when it
  /// forbids them.
  private func startIfAuthorized(_ status: CLAuthorizationStatus) {
    switch status {
    case .authorizedAlways, .authorizedWhenInUse:
      manager.startUpdatingLocation()
    case .denied, .restricted:
      fail(Self.permissionDenied)
    case .notDetermined:
      break
    @unknown default:
      fail(.notSupported("Location authorization is not supported"))
    }
  }

  private func fail(_ error: TokamakPluginError) {
    deliver(.failure(error))
  }

  /// Replies to the waiting requests and every watcher, then stops when none
  /// remain.
  private func deliver(_ result: Result<Any?, TokamakPluginError>) {
    let replies = current + Array(watchers.values)
    current.removeAll()
    for reply in replies {
      reply(result)
    }
    stopIfIdle()
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

  private func stopIfIdle() {
    if current.isEmpty && watchers.isEmpty {
      manager.stopUpdatingLocation()
    }
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
