import Foundation
import TokamakPlugin

enum Precision: String, Codable {
  case precise
  case approximate
}

struct Region: Codable, Equatable {
  let id: String
  let radius: Double
  let onEntry: Bool?
  let tags: [String?]
  let counts: [String: Int]
  let data: [String: TokamakJSON]?
  let kind = "region"
}

#if os(iOS)
  struct Visit: Codable {
    let place: String
  }
#endif

/// How a region alerts: a name, a value, or an object with a field only its form has.
enum Alert: TokamakUnion, Equatable {
  case silent
  case muted
  case volume(Double)
  case pattern([Int])
  case enabled(Bool)
  case tone(Tone)
  case flash(Flash)
}

struct Tone: Codable, Equatable {
  let frequency: Double
  let volume: Double?
}

struct Flash: Codable, Equatable {
  let colour: String
  let volume: Double?
}

/// A region's mark: none, a precision, or labels, an object of any keys.
enum Mark: TokamakUnion, Equatable {
  case none
  case accuracy(Precision)
  case labels([String: String])
}

extension TokamakExamplePlugin {
  /// Where a region alerts, tagged by its kind. Its generated coding names `Circle` by its path.
  enum Zone: TokamakTaggedUnion, Equatable {
    static let tag = "kind"
    case everywhere
    case circle(Circle)
  }

  struct Circle: Codable, Equatable {
    let radius: Double
  }
}

/// Every kind of method the Worker calls, for the describer's tests.
public final class TokamakExamplePlugin: TokamakPlugin {
  public let id = "example"
  public let host: TokamakHost
  private var added: [Region] = []

  public init(host: TokamakHost) {
    self.host = host
  }

  @MainActor
  func addRegion(_ region: Region, replace: Bool?) async throws {
    added.append(region)
  }

  func regions() -> [Region] {
    added
  }

  #if os(iOS)
    func permission(_ precision: Precision) -> [String: String?] {
      ["precision": precision.rawValue]
    }
  #endif

  func watchPosition(every: Double) -> AsyncStream<Region> {
    AsyncStream { continuation in
      continuation.yield(
        Region(id: "\(every)", radius: 1, onEntry: nil, tags: [], counts: [:], data: nil))
      continuation.finish()
    }
  }

  func onVisit() -> AsyncThrowingStream<String, any Error> {
    AsyncThrowingStream {
      $0.finish(throwing: TokamakPluginError(name: "NotAllowedError", message: "No visits"))
    }
  }

  func capture(quality: Int) async throws -> Data {
    Data(repeating: UInt8(quality), count: 2)
  }

  func encode(_ frame: TokamakReference<Data>) -> Int {
    frame.object.count
  }

  func upload(_ name: String, file: TokamakPluginBody) async throws -> Int {
    try await withCheckedThrowingContinuation { continuation in
      file.read { continuation.resume(with: $0.map { $0?.count ?? 0 }) }
    }
  }

  func digest(_ data: Data) -> String {
    data.map { String(format: "%02x", $0) }.joined()
  }

  func read(path: String) throws(TokamakPluginError) -> Data {
    guard path == "a.txt" else { throw TokamakPluginError(name: "NotFoundError", message: path) }
    return Data([9])
  }

  func copy(_ data: Data) -> Data {
    data
  }

  func mirror(_ value: TokamakJSON) -> TokamakJSON {
    value
  }

  func alert(_ alert: Alert, zone: Zone) -> [Alert] {
    zone == .everywhere ? [alert, .silent] : [alert]
  }

  func zone(_ zone: Zone) -> Zone {
    zone
  }

  func mark(_ mark: Mark) -> Mark {
    mark
  }
}
