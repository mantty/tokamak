import CoreLocation
import Foundation

/// Positions from a location manager of its own, for one `currentPosition`
/// or `watchPosition`. Use it on the main thread, where the end of its stream
/// stops it.
final class TokamakLocationPositions: NSObject, CLLocationManagerDelegate, @unchecked Sendable {
  private let manager = CLLocationManager()
  private let continuation: AsyncThrowingStream<TokamakLocationPlugin.Position, Error>.Continuation
  private let watching: Bool

  /// A new fix, or each position as the device moves when `watching`, until
  /// the stream ends.
  static func stream(watching: Bool) -> AsyncThrowingStream<TokamakLocationPlugin.Position, Error> {
    AsyncThrowingStream { continuation in
      let positions = TokamakLocationPositions(continuation, watching: watching)
      // The stream owns the location manager until it ends.
      continuation.onTermination = { _ in
        DispatchQueue.main.async { positions.manager.stopUpdatingLocation() }
      }
      if watching {
        positions.manager.startUpdatingLocation()
      } else {
        positions.manager.requestLocation()
      }
    }
  }

  private init(
    _ continuation: AsyncThrowingStream<TokamakLocationPlugin.Position, Error>.Continuation,
    watching: Bool
  ) {
    self.continuation = continuation
    self.watching = watching
    super.init()
    manager.delegate = self
    manager.desiredAccuracy = kCLLocationAccuracyBest
  }

  func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
    for location in watching ? locations : locations.suffix(1) {
      continuation.yield(TokamakLocationPlugin.Position(location))
    }
    if !watching {
      continuation.finish()
    }
  }

  /// Asks again, or keeps watching, while Core Location does not know the
  /// position yet.
  func locationManager(_ manager: CLLocationManager, didFailWithError error: Error) {
    guard (error as NSError).code == CLError.Code.locationUnknown.rawValue else {
      return continuation.finish(throwing: TokamakPluginError.location(error))
    }
    if !watching {
      manager.requestLocation()
    }
  }
}
