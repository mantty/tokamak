import Foundation
import Security
import WebKit

#if os(iOS)
  import UIKit
#else
  import AppKit
#endif

struct TokamakPluginError: Error {
  let name: String
  let message: String
  /// JSON details the Worker receives as the error's `cause`.
  var cause: Any? = nil

  static func notSupported(_ message: String) -> Self {
    Self(name: "NotSupportedError", message: message)
  }

  /// A native screen was needed while the app has no window.
  static let noWindow = Self(name: "InvalidStateError", message: "The app has no window")

  /// UI was needed while the app cannot show it.
  static let needsUI = Self(
    name: "NeedsUIError",
    message: "The app cannot show UI while it is in the background"
  )
}

extension TokamakPluginError {
  /// `error` for the Worker: itself, or an `OperationError` whose cause has
  /// the platform error's domain and code.
  init(_ error: any Error) {
    if let error = error as? Self {
      self = error
      return
    }
    let error = error as NSError
    self.init(
      name: "OperationError", message: error.localizedDescription,
      cause: ["domain": error.domain, "code": error.code])
  }

  /// The error as JSON: its name, message and cause.
  var json: [String: Any] {
    var json: [String: Any] = ["name": name, "message": message]
    json["cause"] = cause
    return json
  }
}

typealias TokamakPluginReply = (Result<Any?, TokamakPluginError>) -> Void
typealias TokamakPluginBytesReply = (TokamakPluginResult) -> Void

struct TokamakPluginResult {
  let payload: Data
  let failed: Bool
  let done: Bool
}

extension TokamakPluginResult {
  init(_ result: Result<Data, TokamakPluginError>, done: Bool) {
    self.done = done
    switch result {
    case .success(let payload):
      self.payload = payload
      failed = false
    case .failure(let error):
      payload = try! JSONSerialization.data(withJSONObject: error.json)
      failed = true
    }
  }
}

/// `value` as a payload: `Data` as it is, and any other value as JSON.
func encodePluginValue(_ value: Any?) -> Result<Data, TokamakPluginError> {
  if let data = value as? Data { return .success(data) }
  let value = value ?? NSNull()
  // JSONSerialization raises, rather than throws, for values it cannot encode.
  guard JSONSerialization.isValidJSONObject([value]),
    let json = try? JSONSerialization.data(withJSONObject: value, options: .fragmentsAllowed)
  else {
    return .failure(.init(name: "DataCloneError", message: "The value cannot be encoded as JSON"))
  }
  return .success(json)
}

func decodePluginValue(_ payload: Data) throws(TokamakPluginError) -> Any {
  do { return try JSONSerialization.jsonObject(with: payload, options: .fragmentsAllowed) } catch {
    throw .init(name: "DataCloneError", message: error.localizedDescription)
  }
}

#if os(macOS)
  /// The value of the app's entitlement `key`, or nil when its signature lacks it.
  func tokamakEntitlement(_ key: String) -> Any? {
    guard let task = SecTaskCreateFromSelf(nil) else { return nil }
    return SecTaskCopyValueForEntitlement(task, key as CFString, nil)
  }
#endif

/// The app as one plugin sees it.
final class TokamakHost {
  private unowned let app: TokamakApp
  private let plugin: String

  init(app: TokamakApp, plugin: String) {
    self.app = app
    self.plugin = plugin
  }

  var webView: WKWebView? { app.webView }

  /// The native objects plugins share with each other by ID.
  var resources: TokamakResources { app.resources }

  /// Throws `NeedsUIError` unless the app can show UI now: on iOS, while it
  /// is in the foreground. A hidden or minimised macOS app still shows system
  /// prompts. Call it on the main thread, where the plugin decides to show
  /// UI.
  func requireUI() throws(TokamakPluginError) {
    #if os(iOS)
      guard app.isInForeground else { throw .needsUI }
    #endif
  }

  #if os(iOS)
    /// The view controller to present a native screen from: the top of the
    /// app window's presentation stack. Throws `NeedsUIError` unless the app
    /// can show UI now. Call it on the main thread.
    func presenter() throws(TokamakPluginError) -> UIViewController {
      try requireUI()
      guard var top = webView?.window?.rootViewController else { throw .noWindow }
      while let presented = top.presentedViewController { top = presented }
      return top
    }
  #else
    /// The app window, to present a native screen over. Call it on the main
    /// thread.
    func presenter() throws(TokamakPluginError) -> NSWindow {
      guard let window = webView?.window else { throw .noWindow }
      return window
    }
  #endif

  /// Delivers the plugin's event `name`, which the Worker's listeners
  /// receive as `<plugin>.apple.<name>`, with its encoded `payload`, and calls
  /// `completion` on the main thread with their encoded reply, which is empty
  /// when none replied. Fails unless they return within `timeout`, which
  /// includes runtime startup. Call it on the main thread.
  func emitBytes(
    _ name: String,
    payload: Data,
    timeout: TimeInterval,
    completion: @escaping (Result<Data, Error>) -> Void
  ) {
    app.emit("\(plugin).apple.\(name)", payload: payload, timeout: timeout, completion: completion)
  }

  /// Delivers `event` to the Worker's listeners of the plugin's event `name`,
  /// which `apple/Exports.swift` lists, then calls `completion` on the main
  /// thread. Fails unless they return within `timeout`, which includes runtime
  /// startup. Call it on the main thread.
  func emit(
    _ name: String,
    event: some Encodable,
    timeout: TimeInterval,
    completion: @escaping (Result<Void, Error>) -> Void
  ) {
    send(name, event, timeout: timeout) { completion($0.map { _ in }) }
  }

  /// `emit` of an event whose listeners may reply, calling `completion` with
  /// the reply as the plugin's generated dispatch decodes it, which is a
  /// `Reply`, or nil when none replied.
  func emit<Reply>(
    _ name: String,
    event: some Encodable,
    reply: Reply.Type,
    timeout: TimeInterval,
    completion: @escaping (Result<Reply?, Error>) -> Void
  ) {
    send(name, event, timeout: timeout) { result in
      completion(
        result.flatMap { reply in
          Result<Reply?, Error> { try self.decoded(reply, to: name, as: Reply.self) }
        })
    }
  }

  /// Delivers `event` as the plugin's generated dispatch encodes it.
  private func send(
    _ name: String, _ event: Any, timeout: TimeInterval,
    completion: @escaping (Result<Data, Error>) -> Void
  ) {
    let payload: Data
    do throws(TokamakPluginError) {
      payload = try exports(name).eventPayload(name, event)
    } catch {
      return completion(.failure(error))
    }
    emitBytes(name, payload: payload, timeout: timeout, completion: completion)
  }

  /// The listeners' `reply` to the event `name` as a `Reply`, or nil when none replied.
  private func decoded<Reply>(
    _ reply: Data, to name: String, as type: Reply.Type
  ) throws(TokamakPluginError) -> Reply? {
    guard !reply.isEmpty, let value = try exports(name).eventReply(name, reply) else { return nil }
    guard let reply = value as? Reply else {
      throw TokamakPluginError(
        name: "TypeError",
        message: "event \(name) replies with \(Swift.type(of: value)), not \(Reply.self)")
    }
    return reply
  }

  /// The plugin, whose generated dispatch encodes the event `name`.
  private func exports(_ name: String) throws(TokamakPluginError) -> any TokamakExports {
    guard let exports = app.plugins[plugin] as? any TokamakExports else {
      throw .unlistedEvent(name)
    }
    return exports
  }

  /// Fetches what the app serves at `path`, as its WebView would load it: a
  /// file the app ships or the Worker's response. Calls `completion` on the
  /// main thread with the body. Fails unless it arrives within `timeout`,
  /// which includes runtime startup. Call it on the main thread.
  func fetch(
    _ path: String,
    timeout: TimeInterval,
    completion: @escaping (Result<Data, Error>) -> Void
  ) {
    app.fetch(path, timeout: timeout, completion: completion)
  }
}

/// A native plugin, created once per process.
protocol TokamakPlugin: AnyObject {
  var id: String { get }

  init(host: TokamakHost)

  /// Serial executor for this operation and its cancellation.
  func executor(for method: String) -> DispatchQueue

  /// Throws when the call cannot start, which ends it.
  func callBytes(method: String, payload: Data, reply: @escaping TokamakPluginBytesReply)
    throws(TokamakPluginError) -> (() -> Void)?
  func callBytes(
    method: String, payload: Data, body: TokamakPluginBody,
    reply: @escaping TokamakPluginBytesReply
  ) throws(TokamakPluginError) -> (() -> Void)?
  func subscribeBytes(method: String, payload: Data, reply: @escaping TokamakPluginBytesReply)
    throws(TokamakPluginError) -> (() -> Void)

  /// Returns cancellation for pending work when the platform supports it.
  func call(
    method: String,
    arguments: Any,
    reply: @escaping TokamakPluginReply
  ) -> (() -> Void)?

  /// A call whose `body` of bytes the Worker streams, which the plugin reads
  /// one chunk at a time.
  func call(
    method: String,
    arguments: Any,
    body: TokamakPluginBody,
    reply: @escaping TokamakPluginReply
  ) -> (() -> Void)?

  /// Replies with each value until the returned function is called. Throws
  /// when the subscription cannot start, which ends it.
  func subscribe(
    method: String,
    arguments: Any,
    reply: @escaping TokamakPluginReply
  ) throws(TokamakPluginError) -> (() -> Void)

  func didRegisterForRemoteNotifications(deviceToken: Data)

  func didFailToRegisterForRemoteNotifications(error: Error)

  /// Handles a remote notification delivered to the app, calling `completion`
  /// once when finished with it.
  func didReceiveRemoteNotification(
    _ userInfo: [AnyHashable: Any],
    completion: @escaping (TokamakBackgroundResult) -> Void
  )
}

extension TokamakPlugin {
  func executor(for method: String) -> DispatchQueue { .main }

  func callBytes(method: String, payload: Data, reply: @escaping TokamakPluginBytesReply)
    throws(TokamakPluginError) -> (() -> Void)?
  {
    try call(method: method, arguments: decodePluginValue(payload)) {
      reply(TokamakPluginResult($0.flatMap(encodePluginValue), done: true))
    }
  }

  func callBytes(
    method: String, payload: Data, body: TokamakPluginBody,
    reply: @escaping TokamakPluginBytesReply
  ) throws(TokamakPluginError) -> (() -> Void)? {
    try call(method: method, arguments: decodePluginValue(payload), body: body) {
      reply(TokamakPluginResult($0.flatMap(encodePluginValue), done: true))
    }
  }

  func subscribeBytes(method: String, payload: Data, reply: @escaping TokamakPluginBytesReply)
    throws(TokamakPluginError) -> (() -> Void)
  {
    try subscribe(method: method, arguments: decodePluginValue(payload)) {
      reply(TokamakPluginResult($0.flatMap(encodePluginValue), done: false))
    }
  }

  func call(
    method: String,
    arguments: Any,
    reply: @escaping TokamakPluginReply
  ) -> (() -> Void)? {
    reply(.failure(.notSupported("\(id).\(method) is not supported")))
    return nil
  }

  func call(
    method: String,
    arguments: Any,
    body: TokamakPluginBody,
    reply: @escaping TokamakPluginReply
  ) -> (() -> Void)? {
    reply(.failure(.notSupported("\(id).\(method) does not take a body")))
    return nil
  }

  func subscribe(
    method: String,
    arguments: Any,
    reply: @escaping TokamakPluginReply
  ) throws(TokamakPluginError) -> (() -> Void) {
    throw .notSupported("\(id).\(method) is not supported")
  }

  func didRegisterForRemoteNotifications(deviceToken: Data) {}

  func didFailToRegisterForRemoteNotifications(error: Error) {}

  func didReceiveRemoteNotification(
    _ userInfo: [AnyHashable: Any],
    completion: @escaping (TokamakBackgroundResult) -> Void
  ) {
    completion(.noData)
  }
}

/// The bytes the Worker streams to a call, which the plugin reads one chunk
/// at a time.
final class TokamakPluginBody {
  typealias Chunk = (Result<Data?, TokamakPluginError>) -> Void

  private let queue: DispatchQueue
  private let pull: () -> Void
  private let lock = NSLock()
  private var reading: Chunk?

  init(queue: DispatchQueue, pull: @escaping () -> Void) {
    self.queue = queue
    self.pull = pull
  }

  /// Reads the next chunk: its bytes, nil after the last, or the Worker's
  /// error. `handler` runs on the call's executor; read again only after it
  /// has.
  func read(_ handler: @escaping Chunk) {
    lock.withLock { reading = handler }
    pull()
  }

  /// Answers the pending read with the chunk the Worker sent.
  fileprivate func receive(_ chunk: TokamakPluginResult) {
    let handler = lock.withLock {
      defer { reading = nil }
      return reading
    }
    guard let handler else { return }
    queue.async {
      if chunk.failed {
        let error = try? JSONSerialization.jsonObject(with: chunk.payload) as? [String: String]
        handler(.failure(.init(name: error?["name"] ?? "Error", message: error?["message"] ?? "")))
      } else {
        handler(.success(chunk.done ? nil : chunk.payload))
      }
    }
  }
}

/// What the Worker asked a plugin for.
enum TokamakPluginRequest {
  case call, callWithBody, subscribe
}

/// Runs the Worker's requests on the executor each plugin chooses, and passes
/// replies straight to the runtime.
final class TokamakPluginRequests<Key: Hashable> {
  /// A pending call or subscription. Only its serial executor touches
  /// `cancellation`, so a cancellation runs after the operation started.
  private final class Request {
    let queue: DispatchQueue
    var body: TokamakPluginBody?
    var cancellation: (() -> Void)?
    init(queue: DispatchQueue) { self.queue = queue }
  }

  private let plugins: [String: any TokamakPlugin]
  private let send: (Key, TokamakPluginResult) -> Void
  private let pull: (Key) -> Void
  private let lock = NSLock()
  private var pending: [Key: Request] = [:]

  /// Routes requests to `plugins`, sending their results with `send` and
  /// asking the Worker for the next chunk of a call's body with `pull`.
  init(
    plugins: [String: any TokamakPlugin],
    send: @escaping (Key, TokamakPluginResult) -> Void,
    pull: @escaping (Key) -> Void = { _ in }
  ) {
    self.plugins = plugins
    self.send = send
    self.pull = pull
  }

  func run(
    _ key: Key, _ kind: TokamakPluginRequest, plugin: String, method: String, payload: Data
  ) {
    guard let target = plugins[plugin], !method.isEmpty else {
      send(key, TokamakPluginResult(.failure(.notSupported("Plugin is not supported")), done: true))
      return
    }
    let request = Request(queue: target.executor(for: method))
    if kind == .callWithBody {
      request.body = TokamakPluginBody(queue: request.queue) { [weak self] in self?.pull(key) }
    }
    lock.withLock { pending[key] = request }
    let subscribe = kind == .subscribe
    // A weak request lets a completed call release its cancellation.
    let reply: TokamakPluginBytesReply = { [weak self, weak request] result in
      guard let self, let request else { return }
      let last = !subscribe || result.done
      guard last ? remove(key, request) : isPending(key, request) else { return }
      send(key, result)
      if subscribe && result.done { cancel(request) }
    }
    request.queue.async { [self] in
      guard isPending(key, request) else { return }
      do throws(TokamakPluginError) {
        if subscribe {
          request.cancellation = try target.subscribeBytes(
            method: method, payload: payload, reply: reply)
        } else if let body = request.body {
          request.cancellation = try target.callBytes(
            method: method, payload: payload, body: body, reply: reply)
        } else {
          request.cancellation = try target.callBytes(
            method: method, payload: payload, reply: reply)
        }
      } catch {
        if remove(key, request) { send(key, TokamakPluginResult(.failure(error), done: true)) }
      }
    }
  }

  /// Answers the pending read of the call `key`'s body with the chunk the
  /// Worker sent.
  func chunk(_ key: Key, _ chunk: TokamakPluginResult) {
    lock.withLock { pending[key]?.body }?.receive(chunk)
  }

  func cancel(_ key: Key) {
    if let request = lock.withLock({ pending.removeValue(forKey: key) }) { cancel(request) }
  }

  func cancelAll() {
    let requests = lock.withLock {
      defer { pending.removeAll() }
      return Array(pending.values)
    }
    for request in requests { cancel(request) }
  }

  /// Runs the operation's cancellation on its executor.
  private func cancel(_ request: Request) {
    request.queue.async {
      request.cancellation?()
      request.cancellation = nil
    }
  }

  private func isPending(_ key: Key, _ request: Request) -> Bool {
    lock.withLock { pending[key] === request }
  }

  /// Removes `request`, returning whether it was pending.
  private func remove(_ key: Key, _ request: Request) -> Bool {
    lock.withLock {
      guard pending[key] === request else { return false }
      pending[key] = nil
      return true
    }
  }
}
