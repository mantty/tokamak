import CoreLocation
import Foundation

extension TokamakLocationPlugin {
  struct Coordinates: Codable {
    let latitude: Double
    let longitude: Double
    let accuracy: Double
    let altitude: Double?
    let altitudeAccuracy: Double?
    let heading: Double?
    let speed: Double?
  }

  struct Position: Codable {
    let coords: Coordinates
    /// Milliseconds since the epoch.
    let timestamp: Double

    init(_ location: CLLocation) {
      let coordinate = location.coordinate
      let altitude = location.verticalAccuracy >= 0
      coords = Coordinates(
        latitude: coordinate.latitude,
        longitude: coordinate.longitude,
        accuracy: location.horizontalAccuracy,
        altitude: altitude ? location.altitude : nil,
        altitudeAccuracy: altitude ? location.verticalAccuracy : nil,
        heading: location.course >= 0 ? location.course : nil,
        speed: location.speed >= 0 ? location.speed : nil
      )
      timestamp = location.timestamp.milliseconds
    }
  }

  struct LocationUpdate: Codable {
    let positions: [Position]
    /// True on the last update before updates pause while the device is still.
    let stationary: Bool?
  }

  enum Transition: String, Codable {
    case enter
    case exit
  }

  struct RegionTransition: Codable {
    let id: String
    let transition: Transition
    /// Milliseconds since the epoch.
    let timestamp: Double

    init(id: String, entered: Bool, at date: Date) {
      self.id = id
      transition = entered ? .enter : .exit
      timestamp = date.milliseconds
    }
  }

  struct Visit: Codable {
    let latitude: Double
    let longitude: Double
    let accuracy: Double
    /// Milliseconds since the epoch, or nil when unknown.
    let arrival: Double?
    /// Milliseconds since the epoch, or nil before the user leaves.
    let departure: Double?

    init(_ visit: CLVisit) {
      latitude = visit.coordinate.latitude
      longitude = visit.coordinate.longitude
      accuracy = visit.horizontalAccuracy
      arrival = visit.arrivalDate == .distantPast ? nil : visit.arrivalDate.milliseconds
      departure = visit.departureDate == .distantFuture ? nil : visit.departureDate.milliseconds
    }
  }

  struct LocationErrorEvent: Codable {
    let name: String
    let message: String
    /// The region's id, when the failure concerns one.
    let id: String?
  }

  enum Permission: String, Codable {
    case granted
    case denied
    case prompt
  }

  enum Precision: String, Codable {
    case precise
    case approximate
  }

  struct LocationPermission: Codable {
    let inUse: Permission
    let background: Permission
    let precision: Precision
  }

  struct PermissionRequest: Codable {
    let background: Bool?
    let precision: Precision?
    /// The key in the app's `NSLocationTemporaryUsageDescriptionDictionary` of the reason to show.
    let purpose: String?
  }

  struct SetupConfig: Codable {
    let significantChanges: Bool?
    let visits: Bool?
  }

  /// The live update configuration, by Apple's name.
  enum Activity: String, Codable {
    case `default`
    case automotiveNavigation
    case otherNavigation
    case fitness
    case airborne
    case maritime
  }

  struct UpdateOptions: Codable {
    let activity: Activity?
  }

  struct Region: Codable {
    let id: String
    let latitude: Double
    let longitude: Double
    /// In metres.
    let radius: Double
    let onEntry: Bool?
    let onExit: Bool?
  }

  enum RegionState: String, Codable {
    case inside
    case outside
    case unknown
  }

  struct MonitoredRegion: Codable {
    let id: String
    let latitude: Double
    let longitude: Double
    let radius: Double
    let onEntry: Bool
    let onExit: Bool
    /// Whether the device is inside the region, as the system last determined.
    let state: RegionState
  }
}

extension Date {
  fileprivate var milliseconds: Double { timeIntervalSince1970 * 1000 }
}
