import Foundation

/// One call or subscription of a generated platform binding. It decodes the
/// Worker's JSON into native values and encodes native values back, keeping
/// the native objects it returns in the receiving invocation's scope.
///
/// JSON objects tagged `#` carry what JSON lacks: native objects, dates,
/// bytes, errors, non-finite numbers, and records whose keys include `#`.
struct TokamakBinding {
  private let resources: TokamakResources
  private let request: Result<[String: Any], TokamakPluginError>
  private let reply: TokamakPluginBytesReply

  init(
    _ payload: Data, resources: TokamakResources, reply: @escaping TokamakPluginBytesReply
  ) {
    self.resources = resources
    self.reply = reply
    request = Result { () throws(TokamakPluginError) in
      guard let request = try decodePluginValue(payload) as? [String: Any] else {
        throw typeError("A binding request is an object")
      }
      return request
    }
  }

  /// The rejection of a member the running OS version lacks.
  static let unavailable = TokamakPluginError.notSupported(
    "The API is unavailable on this OS version")

  /// Argument `index` as JSON.
  subscript(index: Int) -> Any {
    get throws {
      let arguments = try request.get()["arguments"] as? [Any] ?? []
      return arguments.indices.contains(index) ? arguments[index] : NSNull()
    }
  }

  /// The object the call is made on.
  func target<T>(_ type: T.Type) throws -> T {
    try object(request.get()["target"] as Any, as: type)
  }

  /// The handlers an implementation calls.
  var methods: Set<String> {
    Set((try? request.get()["methods"] as? [String]) ?? [])
  }

  private var scope: String {
    (try? request.get()["scope"] as? String) ?? ""
  }

  // MARK: Decoding

  func decode(_ value: Any, as type: Bool.Type) throws -> Bool {
    guard let value = value as? Bool else { throw mismatch(value, type) }
    return value
  }

  func decode<T: BinaryInteger>(_ value: Any, as type: T.Type) throws -> T {
    guard let number = value as? NSNumber, let value = T(exactly: number.doubleValue) else {
      throw mismatch(value, type)
    }
    return value
  }

  func decode<T: BinaryFloatingPoint>(_ value: Any, as type: T.Type) throws -> T {
    if let number = value as? NSNumber { return T(number.doubleValue) }
    guard let number = try Double(decode(tagged(value, "number"), as: String.self)) else {
      throw mismatch(value, type)
    }
    return T(number)
  }

  func decode(_ value: Any, as type: String.Type) throws -> String {
    guard let value = value as? String else { throw mismatch(value, type) }
    return value
  }

  func decode(_ value: Any, as type: NSNumber.Type) throws -> NSNumber {
    guard let value = value as? NSNumber else { throw mismatch(value, type) }
    return value
  }

  func decode(_ value: Any, as type: Locale.Type) throws -> Locale {
    Locale(identifier: try decode(value, as: String.self))
  }

  func decode(_ value: Any, as type: TimeZone.Type) throws -> TimeZone {
    guard let zone = TimeZone(identifier: try decode(value, as: String.self)) else {
      throw mismatch(value, type)
    }
    return zone
  }

  func decode(_ value: Any, as type: Date.Type) throws -> Date {
    Date(timeIntervalSince1970: try decode(tagged(value, "date"), as: Double.self) / 1000)
  }

  func decode(_ value: Any, as type: Data.Type) throws -> Data {
    guard let data = Data(base64Encoded: try decode(tagged(value, "bytes"), as: String.self)) else {
      throw mismatch(value, type)
    }
    return data
  }

  func decode(_ value: Any, as type: URL.Type) throws -> URL {
    guard let url = URL(string: try decode(value, as: String.self)) else {
      throw mismatch(value, type)
    }
    return url
  }

  func decode(_ value: Any, as type: UUID.Type) throws -> UUID {
    guard let uuid = UUID(uuidString: try decode(value, as: String.self)) else {
      throw mismatch(value, type)
    }
    return uuid
  }

  /// The field `name` of the struct `value`.
  func field(_ value: Any, _ name: String) throws -> Any {
    guard let fields = value as? [String: Any] else { throw mismatch(value, [String: Any].self) }
    return fields[name] ?? NSNull()
  }

  /// The native object `value` refers to.
  func object<T>(_ value: Any, as type: T.Type) throws -> T {
    try resources.get(tagged(value, "object", key: "id"), as: type)
  }

  func optional<T>(_ value: Any, _ decode: (Any) throws -> T) rethrows -> T? {
    value is NSNull ? nil : try decode(value)
  }

  func array<T>(_ value: Any, _ decode: (Any) throws -> T) throws -> [T] {
    guard let values = value as? [Any] else { throw mismatch(value, [T].self) }
    return try values.map(decode)
  }

  func record<T>(_ value: Any, _ decode: (Any) throws -> T) throws -> [String: T] {
    let fields = (try? tagged(value, "record")) ?? value
    guard let fields = fields as? [String: Any] else { throw mismatch(value, [String: T].self) }
    return try fields.mapValues(decode)
  }

  // MARK: Encoding

  func encode(_ value: Bool) -> Any { value }

  func encode<T: BinaryInteger>(_ value: T) -> Any {
    Int64(exactly: value).map { NSNumber(value: $0) } ?? NSNumber(value: Double(value))
  }

  func encode<T: BinaryFloatingPoint>(_ value: T) -> Any {
    let number = Double(value)
    if number.isFinite { return number }
    return ["#": "number", "value": number.isNaN ? "NaN" : number > 0 ? "Infinity" : "-Infinity"]
  }

  func encode(_ value: String) -> Any { value }

  func encode(_ value: NSNumber) -> Any { encode(value.doubleValue) }

  func encode(_ value: Locale) -> Any { value.identifier }

  func encode(_ value: TimeZone) -> Any { value.identifier }

  func encode(_ value: Date) -> Any { ["#": "date", "value": value.timeIntervalSince1970 * 1000] }

  func encode(_ value: Data) -> Any { ["#": "bytes", "value": value.base64EncodedString()] }

  func encode(_ value: URL) -> Any { value.absoluteString }

  func encode(_ value: UUID) -> Any { value.uuidString }

  func encode(_ value: any Error) -> Any {
    let error = Self.failure(value)
    var encoded: [String: Any] = ["#": "error", "name": error.name, "message": error.message]
    encoded["cause"] = error.cause
    return encoded
  }

  /// Keeps `value` in the invocation's scope and refers to it, with the
  /// names of its class and superclasses.
  func encode(object value: Any) -> Any {
    ["#": "object", "id": resources.retain(value, in: scope), "types": types(of: value)]
  }

  func encode<T>(optional value: T?, _ encode: (T) -> Any) -> Any {
    value.map(encode) ?? NSNull()
  }

  func encode<S: Sequence>(array values: S, _ encode: (S.Element) -> Any) -> Any {
    values.map(encode)
  }

  func encode<T>(record fields: [String: T], _ encode: (T) -> Any) -> Any {
    let encoded = fields.mapValues(encode)
    return encoded["#"] == nil ? encoded : ["#": "record", "value": encoded]
  }

  // MARK: Replies

  /// Replies with the JSON `body` returns, or the error it throws.
  func run(_ body: () throws -> Any?) -> (() -> Void)? {
    do { send(try body(), done: true) } catch { fail(error) }
    return nil
  }

  /// Replies with the JSON `body` resolves to on the main actor, or the
  /// error it throws. The returned function cancels its task.
  func task(_ body: @escaping @MainActor () async throws -> Any?) -> () -> Void {
    let task = Task { @MainActor in
      do { send(try await body(), done: true) } catch { fail(error) }
    }
    return { task.cancel() }
  }

  /// Replies with each element of the sequence `body` returns on the main
  /// actor, encoded, then ends. The returned function cancels its task.
  func stream<S: AsyncSequence>(
    _ body: @escaping @MainActor () async throws -> S,
    encode: @escaping (S.Element) -> Any
  ) -> () -> Void {
    let task = Task {
      do {
        for try await element in try await body() { send(encode(element), done: false) }
        reply(TokamakPluginResult(payload: Data(), failed: false, done: true))
      } catch { fail(error) }
    }
    return { task.cancel() }
  }

  /// Keeps `type`'s implementation while the subscription lasts, and replies
  /// with its ID.
  func implement(_ type: TokamakBindingImplementation.Type) -> () -> Void {
    let scope = resources.open()
    send(resources.retain(type.init(self), in: scope), done: false)
    return { [resources] in resources.end(scope) }
  }

  /// Rejects the member the plugin does not bind.
  func unsupported(_ member: String) -> (() -> Void)? {
    fail(TokamakPluginError.notSupported("\(member) is not supported"))
    return nil
  }

  func send(_ value: Any?, done: Bool) {
    reply(TokamakPluginResult(encodePluginValue(value), done: done))
  }

  private func fail(_ error: any Error) {
    reply(TokamakPluginResult(.failure(Self.failure(error)), done: true))
  }

  /// `error` for the Worker: an `OperationError` whose cause has the
  /// platform error's domain and code.
  static func failure(_ error: any Error) -> TokamakPluginError {
    if let error = error as? TokamakPluginError { return error }
    let error = error as NSError
    return TokamakPluginError(
      name: "OperationError", message: error.localizedDescription,
      cause: ["domain": error.domain, "code": error.code])
  }

  /// The field `key` of `value` tagged `tag`.
  private func tagged(_ value: Any, _ tag: String, key: String = "value") throws -> Any {
    guard let fields = value as? [String: Any], fields["#"] as? String == tag,
      let field = fields[key]
    else { throw typeError("Expected a tagged \(tag)") }
    return field
  }

  /// The error of `value`, which is not a `type`.
  func mismatch(_ value: Any, _ type: Any.Type) -> TokamakPluginError {
    typeError("\(Swift.type(of: value)) is not \(type)")
  }

  /// The names of `value`'s class and superclasses, then of the Objective-C
  /// protocols they adopt, or of its type: Objective-C names, and
  /// module-qualified Swift names.
  private func types(of value: Any) -> [String] {
    let name = { (type: Any.Type) in
      let name = String(reflecting: type)
      return name.hasPrefix("__C.") ? String(name.dropFirst(4)) : name
    }
    guard let type = Swift.type(of: value) as? AnyClass else {
      return [name(Swift.type(of: value))]
    }
    let classes = Array(sequence(first: type, next: class_getSuperclass))
    let protocols = classes.flatMap(adopted).map(NSStringFromProtocol)
    var seen = Set<String>()
    return (classes.map(name) + protocols).filter { seen.insert($0).inserted }
  }
}

/// The Objective-C protocols `type` adopts, each followed by those it inherits.
private func adopted(by type: AnyClass) -> [Protocol] {
  protocols { class_copyProtocolList(type, $0) }.flatMap { [$0] + inherited(by: $0) }
}

private func inherited(by parent: Protocol) -> [Protocol] {
  protocols { protocol_copyProtocolList(parent, $0) }.flatMap { [$0] + inherited(by: $0) }
}

/// The protocols a runtime `copy` function lists.
private func protocols(
  _ copy: (UnsafeMutablePointer<UInt32>) -> AutoreleasingUnsafeMutablePointer<Protocol>?
) -> [Protocol] {
  var count: UInt32 = 0
  guard let list = copy(&count) else { return [] }
  defer { free(UnsafeMutableRawPointer(list)) }
  return (0..<Int(count)).map { list[$0] }
}

/// A native object that implements a protocol by calling the JavaScript
/// handlers of an implementation subscription. A method without a handler
/// keeps the platform's default.
class TokamakBindingImplementation: NSObject {
  private let binding: TokamakBinding
  private let handled: Set<String>

  required init(_ binding: TokamakBinding) {
    self.binding = binding
    handled = binding.methods
  }

  /// The handler each optional Objective-C method calls, by selector.
  class var selectors: [Selector: String] { [:] }

  override func responds(to selector: Selector!) -> Bool {
    guard let method = Self.selectors[selector] else { return super.responds(to: selector) }
    return handled.contains(method)
  }

  /// Calls the handler `method`, when it has one, with the `arguments` it
  /// encodes.
  func call(_ method: String, _ arguments: (TokamakBinding) -> [Any]) {
    guard handled.contains(method) else { return }
    binding.send(["method": method, "arguments": arguments(binding)], done: false)
  }
}

private func typeError(_ message: String) -> TokamakPluginError {
  TokamakPluginError(name: "TypeError", message: message)
}
