import CoreLocation
import Foundation

/// Positions from a location manager of its own, for one `getCurrentPosition`
/// or `watchPosition`. Use it on the main thread.
final class TokamakLocationPositions: NSObject, CLLocationManagerDelegate {
  private let manager = CLLocationManager()
  private let reply: TokamakPluginReply
  private var watching = false

  /// Passes `reply` the new fix, or each position as the device moves, or a
  /// failure.
  init(reply: @escaping TokamakPluginReply) {
    self.reply = reply
    super.init()
    manager.delegate = self
    manager.desiredAccuracy = kCLLocationAccuracyBest
  }

  func locate() {
    manager.requestLocation()
  }

  func watch() {
    watching = true
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
