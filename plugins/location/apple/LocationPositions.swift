import CoreLocation
import Foundation

/// Positions from a location manager of its own, for one `getCurrentPosition`
/// or `watchPosition`. Use it on the main thread.
final class TokamakLocationPositions: NSObject, CLLocationManagerDelegate {
  private let manager = CLLocationManager()
  private let watching: Bool
  private let reply: (Result<Any?, TokamakPluginError>) -> Void

  /// Passes `reply` one position, or each as the device moves when
  /// `watching`, or a failure.
  init(watching: Bool, reply: @escaping (Result<Any?, TokamakPluginError>) -> Void) {
    self.watching = watching
    self.reply = reply
    super.init()
    manager.delegate = self
    manager.desiredAccuracy = kCLLocationAccuracyBest
  }

  /// Takes a fix, unless Core Location's latest position is no older than
  /// `maximumAge` seconds.
  func locate(maximumAge: TimeInterval) {
    if let recent = manager.location, -recent.timestamp.timeIntervalSinceNow <= maximumAge {
      reply(.success(TokamakLocationEvents.position(recent)))
    } else {
      manager.requestLocation()
    }
  }

  func watch() {
    manager.startUpdatingLocation()
  }

  func stop() {
    manager.stopUpdatingLocation()
  }

  func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
    for location in watching ? locations : locations.suffix(1) {
      reply(.success(TokamakLocationEvents.position(location)))
    }
  }

  /// Asks again, or keeps watching, while Core Location does not know the
  /// position yet.
  func locationManager(_ manager: CLLocationManager, didFailWithError error: Error) {
    guard (error as NSError).code == CLError.Code.locationUnknown.rawValue else {
      return reply(.failure(.location(error)))
    }
    if !watching {
      manager.requestLocation()
    }
  }
}
