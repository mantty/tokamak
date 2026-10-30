import Foundation

/// A plugin whose `Exports.swift` lists the methods the Worker calls
/// and the events it receives. `tok plugin generate apple` reads it and writes
/// the plugin's dispatch and TypeScript API.
public protocol TokamakExports: TokamakPlugin {
  /// The plugin's host, whose resources hold the native references methods take.
  var host: TokamakHost { get }

  /// Lists the exported methods and events. The app never calls it.
  static func exports(_ e: inout TokamakExporter<Self>)

  /// `event` as the JSON data of the event `name`, which the registration lists.
  func eventPayload(_ name: String, _ event: Any) throws(TokamakPluginError) -> Data

  /// The listeners' JSON `reply` to the event `name`, which the registration
  /// lists with a reply, as its type, or nil for null.
  func eventReply(_ name: String, _ reply: Data) throws(TokamakPluginError) -> Any?
}

extension TokamakExports {
  public func eventPayload(_ name: String, _ event: Any) throws(TokamakPluginError) -> Data {
    throw .unlistedEvent(name)
  }

  public func eventReply(_ name: String, _ reply: Data) throws(TokamakPluginError) -> Any? {
    throw .unlistedReply(name)
  }
}

/// What a stream does with values a slow reader has not taken.
public enum TokamakOverflow {
  /// Ends the stream with `QuotaExceededError`.
  case error
  /// Drops the oldest values, for samples where only recent ones matter.
  case dropOldest
}

/// Lists a plugin's exported methods and events: method parameters and event
/// replies are `Decodable`, method results and events `Encodable`, and options
/// literal values.
public struct TokamakExporter<Plugin> {
  /// Lists the event `name`, whose listeners receive an `E`.
  public mutating func event<E: Encodable>(_ name: String, _ type: E.Type, doc: String = "") {}

  /// Lists the event `name`, whose listeners receive an `E` and may reply with an `R`.
  public mutating func event<E: Encodable, R: Decodable>(
    _ name: String, _ type: E.Type, reply: R.Type, doc: String = ""
  ) {}

  /// Exports `method`, which returns once.
  public mutating func call<each A: Decodable, R: Encodable>(
    _ method: (Plugin) -> (repeat each A) async throws -> R, doc: String = ""
  ) {}

  public mutating func call<each A: Decodable>(
    _ method: (Plugin) -> (repeat each A) async throws -> Void, doc: String = ""
  ) {}

  /// Exports `method`, whose last parameter reads the bytes the Worker streams.
  public mutating func call<each A: Decodable, R: Encodable>(
    _ method: (Plugin) -> (repeat each A, TokamakPluginBody) async throws -> R, doc: String = ""
  ) {}

  public mutating func call<each A: Decodable>(
    _ method: (Plugin) -> (repeat each A, TokamakPluginBody) async throws -> Void, doc: String = ""
  ) {}

  /// Exports `method`, which returns the values it sends until the Worker cancels.
  public mutating func stream<each A: Decodable, S: AsyncSequence>(
    _ method: (Plugin) -> (repeat each A) throws -> S, overflow: TokamakOverflow = .error,
    doc: String = ""
  ) where S.Element: Encodable {}

  /// Exports `method`, which returns a native object the Worker holds as a
  /// `NativeReference`.
  public mutating func resource<each A: Decodable, R>(
    _ method: (Plugin) -> (repeat each A) async throws -> R, doc: String = ""
  ) {}
}

/// A native object the Worker passes as a `NativeReference`, which another
/// plugin retained.
public struct TokamakReference<Object>: Decodable {
  public let object: Object

  public init(from decoder: any Decoder) throws {
    let id = try decoder.singleValueContainer().decode(String.self)
    guard let resources = decoder.userInfo[.tokamakResources] as? TokamakResources else {
      throw TokamakPluginError(name: "InvalidStateError", message: "No native references here")
    }
    object = try resources.get(id, as: Object.self)
  }
}

/// Any JSON value, for data a plugin passes through or reads itself.
public enum TokamakJSON: Codable, Equatable {
  case null
  case bool(Bool)
  case number(Double)
  case string(String)
  case array([TokamakJSON])
  case object([String: TokamakJSON])

  public init(from decoder: any Decoder) throws {
    // Each failed attempt throws, so the commonest kinds of value come first.
    let container = try decoder.singleValueContainer()
    if container.decodeNil() {
      self = .null
    } else if let string = try? container.decode(String.self) {
      self = .string(string)
    } else if let number = try? container.decode(Double.self) {
      self = .number(number)
    } else if let bool = try? container.decode(Bool.self) {
      self = .bool(bool)
    } else if let object = try? container.decode([String: TokamakJSON].self) {
      self = .object(object)
    } else {
      self = .array(try container.decode([TokamakJSON].self))
    }
  }

  public func encode(to encoder: any Encoder) throws {
    var container = encoder.singleValueContainer()
    switch self {
    case .null: try container.encodeNil()
    case .bool(let bool): try container.encode(bool)
    case .number(let number): try container.encode(number)
    case .string(let string): try container.encode(string)
    case .array(let array): try container.encode(array)
    case .object(let object): try container.encode(object)
    }
  }

  /// `value` as JSON, which `JSONSerialization` writes it as, or nil when it is not JSON.
  public init?(_ value: Any) {
    switch value {
    case is NSNull: self = .null
    case let number as NSNumber where CFGetTypeID(number) == CFBooleanGetTypeID():
      self = .bool(number.boolValue)
    case let number as NSNumber: self = .number(number.doubleValue)
    case let string as String: self = .string(string)
    case let array as [Any]:
      let elements = array.compactMap(TokamakJSON.init)
      guard elements.count == array.count else { return nil }
      self = .array(elements)
    case let object as [String: Any]:
      let entries = object.compactMapValues(TokamakJSON.init)
      guard entries.count == object.count else { return nil }
      self = .object(entries)
    default: return nil
    }
  }

  /// The value as `JSONSerialization` reads it: `NSNull`, an `NSNumber` for a boolean or a
  /// number, which `as? Int` reads when whole, `String`, arrays and dictionaries.
  public var value: Any {
    switch self {
    case .null: NSNull()
    case .bool(let bool): NSNumber(value: bool)
    case .number(let number): NSNumber(value: number)
    case .string(let string): string
    case .array(let array): array.map(\.value)
    case .object(let object): object.mapValues(\.value)
    }
  }
}

/// An enum whose cases are the forms its values take: a case without an associated value is
/// the string of its name, and a case with one unlabeled associated value is that value.
/// `tok plugin generate apple` writes its coding in place of these defaults, which throw.
public protocol TokamakUnion: Codable {}

/// A `TokamakUnion` whose values are objects that name their case in the field `tag`, with a
/// struct case's fields.
public protocol TokamakTaggedUnion: TokamakUnion {
  /// The tag field's name, as a string literal, which `tok plugin generate apple` reads.
  static var tag: String { get }
}

extension TokamakUnion {
  public init(from decoder: any Decoder) throws {
    throw TokamakPluginError.ungenerated(Self.self)
  }

  public func encode(to encoder: any Encoder) throws {
    throw TokamakPluginError.ungenerated(Self.self)
  }
}

/// The JSON value whose kind a union's generated decoder chooses its case by.
public struct TokamakUnionValue {
  public enum Kind {
    case null, boolean, number, array
    case string(String)
    /// An object, with its keys.
    case object(Set<String>)
  }

  public let kind: Kind
  private let decoder: any Decoder

  public init(_ decoder: any Decoder) throws {
    self.decoder = decoder
    let value = try decoder.singleValueContainer()
    if value.decodeNil() {
      kind = .null
    } else if let string = try? value.decode(String.self) {
      kind = .string(string)
    } else if (try? value.decode(Bool.self)) != nil {
      kind = .boolean
    } else if (try? value.decode(Double.self)) != nil {
      kind = .number
    } else if let object = try? decoder.container(keyedBy: TokamakUnionKey.self) {
      kind = .object(Set(object.allKeys.map(\.stringValue)))
    } else {
      kind = .array
    }
  }

  /// The value as a `T`.
  public func decode<T: Decodable>(_ type: T.Type) throws -> T {
    try decoder.singleValueContainer().decode(type)
  }

  /// The string in the object's field `tag`.
  public func tag(_ tag: String) throws -> String {
    try decoder.container(keyedBy: TokamakUnionKey.self).decode(String.self, forKey: .init(tag))
  }

  /// The error of a value that is none of the union `name`'s forms.
  public func mismatch(_ name: String) -> DecodingError {
    .dataCorrupted(
      .init(codingPath: decoder.codingPath, debugDescription: "must be one of \(name)'s forms"))
  }
}

/// Encodes the case `name` of a tagged union as an object with `value`'s fields, if any, and the
/// field `tag` naming the case.
public func tokamakEncode(
  tag: String, _ name: String, _ value: (any Encodable)? = nil, to encoder: any Encoder
) throws {
  try value?.encode(to: encoder)
  var container = encoder.container(keyedBy: TokamakUnionKey.self)
  try container.encode(name, forKey: .init(tag))
}

/// A JSON object's key, as a union's coding reads and writes it.
public struct TokamakUnionKey: CodingKey {
  public let stringValue: String
  public var intValue: Int? { nil }

  public init(_ stringValue: String) {
    self.stringValue = stringValue
  }

  public init?(stringValue: String) {
    self.stringValue = stringValue
  }

  public init?(intValue: Int) {
    nil
  }
}

extension CodingUserInfoKey {
  /// The resources a `TokamakReference` finds its object in.
  fileprivate static let tokamakResources = CodingUserInfoKey(rawValue: "tokamak.resources")!
}

extension TokamakPluginError {
  /// A `TypeError` naming where the `root` value, such as a call's
  /// arguments, fails to decode.
  public init(decoding error: DecodingError, in root: String = "arguments") {
    let (path, problem): ([any CodingKey], String) =
      switch error {
      case .keyNotFound(let key, let context): (context.codingPath + [key], "is missing")
      case .typeMismatch(_, let context), .valueNotFound(_, let context),
        .dataCorrupted(let context):
        (context.codingPath, context.debugDescription)
      @unknown default: ([], "\(error)")
      }
    let location = path.reduce("") { location, key in
      if let index = key.intValue { return "\(location)[\(index)]" }
      return location.isEmpty ? key.stringValue : "\(location).\(key.stringValue)"
    }
    self.init(name: "TypeError", message: "\(location.isEmpty ? root : location) \(problem)")
  }

  /// A `TokamakUnion` without the coding `tok plugin generate apple` writes.
  public static func ungenerated(_ union: Any.Type) -> Self {
    Self(name: "TypeError", message: "run tok plugin generate apple to code \(union)")
  }

  /// The plugin emitted an event its registration does not list.
  public static func unlistedEvent(_ name: String) -> Self {
    Self(name: "TypeError", message: "Exports.swift lists no event \(name)")
  }

  /// The plugin asked for the reply to an event its registration lists without one.
  public static func unlistedReply(_ name: String) -> Self {
    Self(name: "TypeError", message: "Exports.swift lists no reply to event \(name)")
  }
}

/// Decodes a call's JSON arguments, or another `root` value, finding the
/// native references they name in `resources`.
public func tokamakDecode<T: Decodable>(
  _ type: T.Type, from payload: Data, resources: TokamakResources? = nil,
  in root: String = "arguments"
) throws(TokamakPluginError) -> T {
  let decoder = JSONDecoder()
  decoder.userInfo[.tokamakResources] = resources
  do {
    return try decoder.decode(type, from: payload)
  } catch let error as DecodingError {
    throw TokamakPluginError(decoding: error, in: root)
  } catch {
    throw TokamakPluginError(error)
  }
}

/// `value` as a reply's payload: `Data` as it is, and any other value as the
/// JSON `encoder` writes.
public func tokamakEncode<T: Encodable>(
  _ value: T, with encoder: JSONEncoder = JSONEncoder()
) throws(TokamakPluginError) -> Data {
  if let data = value as? Data { return data }
  do {
    return try encoder.encode(value)
  } catch {
    throw TokamakPluginError(name: "DataCloneError", message: "\(error)")
  }
}

/// `event` as the JSON data of the event `name`, whose listeners receive a `T`.
public func tokamakEvent<T: Encodable>(
  _ name: String, _ event: Any, as type: T.Type
) throws(TokamakPluginError) -> Data {
  guard let event = event as? T else {
    throw TokamakPluginError(
      name: "TypeError", message: "event \(name) takes \(T.self), not \(Swift.type(of: event))")
  }
  return try tokamakEncode(event)
}

/// Replies with what `method` returns. A synchronous method runs on the
/// call's executor and has nothing to cancel.
public func tokamakCall<T: Encodable>(
  _ reply: TokamakPluginBytesReply, _ method: () throws -> T
) -> (() -> Void)? {
  reply(result { try tokamakEncode(try method()) })
  return nil
}

public func tokamakCall(_ reply: TokamakPluginBytesReply, _ method: () throws -> Void) -> (() -> Void)? {
  reply(
    result {
      try method()
      return null
    })
  return nil
}

/// Runs `method` in a task, replying with what it returns. Cancelling the
/// call cancels the task.
public func tokamakCall<T: Encodable>(
  _ reply: @escaping TokamakPluginBytesReply, _ method: @escaping () async throws -> T
) -> (() -> Void)? {
  task { reply(await awaitResult { try tokamakEncode(try await method()) }) }
}

public func tokamakCall(
  _ reply: @escaping TokamakPluginBytesReply, _ method: @escaping () async throws -> Void
) -> (() -> Void)? {
  task {
    reply(
      await awaitResult {
        try await method()
        return null
      })
  }
}

/// Replies with each value `method`'s sequence sends, until it ends or the
/// subscription is cancelled. A synchronous method runs on the
/// subscription's executor.
public func tokamakStream<S: AsyncSequence>(
  _ reply: @escaping TokamakPluginBytesReply, _ method: () throws -> S
) throws(TokamakPluginError) -> () -> Void where S.Element: Encodable {
  let values = try catching(method)
  return task { await send(values, reply) }
}

public func tokamakStream<S: AsyncSequence>(
  _ reply: @escaping TokamakPluginBytesReply, _ method: @escaping () async throws -> S
) -> () -> Void where S.Element: Encodable {
  task {
    do {
      await send(try await method(), reply)
    } catch {
      reply(TokamakPluginResult(.failure(TokamakPluginError(error)), done: true))
    }
  }
}

/// Retains the object `method` returns in `resources`, replying with its
/// ID, until the subscription is cancelled. A synchronous method runs on the
/// subscription's executor.
public func tokamakResource(
  _ resources: TokamakResources, _ reply: @escaping TokamakPluginBytesReply,
  _ method: () throws -> Any
) throws(TokamakPluginError) -> () -> Void {
  resources.retain(try catching(method), reply: reply)
}

public func tokamakResource(
  _ resources: TokamakResources, _ reply: @escaping TokamakPluginBytesReply,
  _ method: @escaping () async throws -> Any
) -> () -> Void {
  let retained = Retained()
  let cancel = task {
    do {
      let object = try await method()
      retained.hold(resources.retain(object, reply: reply))
    } catch {
      reply(TokamakPluginResult(.failure(TokamakPluginError(error)), done: true))
    }
  }
  return {
    cancel()
    retained.end()
  }
}

/// How many values wait for the first listener.
private let heldCapacity = 64

/// The streams of an event's listeners, such as a page's, each of which
/// receives every value `send` delivers until it ends. A stream keeps every
/// value its reader has not taken, so a Worker's subscription applies its own
/// overflow policy. When `holding`, the newest 64 values sent while none
/// listen wait for the first listener.
public final class TokamakListeners<Value>: @unchecked Sendable {
  private let lock = NSLock()
  private let holding: Bool
  private var continuations: [UUID: AsyncStream<Value>.Continuation] = [:]
  private var held: [Value] = []

  public init(holding: Bool = false) {
    self.holding = holding
  }

  /// A new listener's stream.
  // Values are yielded under the lock, so one sent while a listener starts
  // follows the held ones. Yielding only buffers.
  public func stream() -> AsyncStream<Value> {
    let (stream, continuation) = AsyncStream.makeStream(of: Value.self)
    let key = UUID()
    continuation.onTermination = { [weak self] _ in
      self?.lock.withLock { _ = self?.continuations.removeValue(forKey: key) }
    }
    lock.withLock {
      continuations[key] = continuation
      for value in held {
        continuation.yield(value)
      }
      held = []
    }
    return stream
  }

  public func send(_ value: Value) {
    lock.withLock {
      if holding && continuations.isEmpty {
        held.append(value)
        if held.count > heldCapacity { held.removeFirst() }
      }
      for continuation in continuations.values {
        continuation.yield(value)
      }
    }
  }
}

/// The JSON payload of a method that returns nothing.
private let null = Data("null".utf8)

/// A call's last reply: `payload`'s bytes, or its failure.
private func result(_ payload: () throws -> Data) -> TokamakPluginResult {
  TokamakPluginResult(Result { () throws(TokamakPluginError) in try catching(payload) }, done: true)
}

private func awaitResult(_ payload: () async throws -> Data) async -> TokamakPluginResult {
  do {
    return TokamakPluginResult(.success(try await payload()), done: true)
  } catch {
    return TokamakPluginResult(.failure(TokamakPluginError(error)), done: true)
  }
}

/// What `body` returns, with what it throws as a `TokamakPluginError`.
private func catching<T>(_ body: () throws -> T) throws(TokamakPluginError) -> T {
  do {
    return try body()
  } catch {
    throw TokamakPluginError(error)
  }
}

/// Runs `body` in a task, returning its cancellation.
private func task(_ body: @escaping () async -> Void) -> () -> Void {
  Task { await body() }.cancel
}

/// Replies with each of `values`, then with the end of the subscription.
private func send<S: AsyncSequence>(
  _ values: S, _ reply: TokamakPluginBytesReply
) async where S.Element: Encodable {
  // One encoder serves the stream, whose values it encodes one at a time.
  let encoder = JSONEncoder()
  do {
    for try await value in values {
      reply(TokamakPluginResult(.success(try tokamakEncode(value, with: encoder)), done: false))
    }
    reply(TokamakPluginResult(payload: Data(), failed: false, done: true))
  } catch {
    reply(TokamakPluginResult(.failure(TokamakPluginError(error)), done: true))
  }
}

/// The release of an object a subscription retains, which runs once the
/// subscription ends, whether the object is retained before or after.
private final class Retained {
  private let lock = NSLock()
  private var release: (() -> Void)?
  private var ended = false

  func hold(_ release: @escaping () -> Void) {
    let ended = lock.withLock {
      if !self.ended { self.release = release }
      return self.ended
    }
    if ended { release() }
  }

  func end() {
    let release = lock.withLock {
      ended = true
      defer { self.release = nil }
      return self.release
    }
    release?()
  }
}
