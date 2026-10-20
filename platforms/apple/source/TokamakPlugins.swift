import Foundation
import WebKit

#if os(iOS)
  import UIKit
#else
  import AppKit
#endif

struct TokamakPluginError: Error {
  let name: String
  let message: String

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

typealias TokamakPluginReply = (Result<Any?, TokamakPluginError>) -> Void
typealias TokamakPluginBytesReply = (TokamakPluginResult) -> Void

struct TokamakPluginResult {
  let payload: Data
  let failed: Bool
  let done: Bool

  init(payload: Data, failed: Bool, done: Bool) {
    self.payload = payload
    self.failed = failed
    self.done = done
  }

  init(_ result: Result<Data, TokamakPluginError>, done: Bool) {
    self.done = done
    switch result {
    case .success(let payload):
      self.payload = payload
      failed = false
    case .failure(let error):
      payload = try! JSONSerialization.data(
        withJSONObject: ["name": error.name, "message": error.message])
      failed = true
    }
  }
}

/// `value` as a payload: `Data` as it is, and any other value as JSON.
private func encodePluginValue(_ value: Any?) -> Result<Data, TokamakPluginError> {
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

/// Decodes `payload` as JSON for `call`, and encodes its reply.
private func callJSON(
  _ payload: Data,
  _ reply: @escaping TokamakPluginBytesReply,
  _ call: (Any, @escaping TokamakPluginReply) -> (() -> Void)?
) -> (() -> Void)? {
  do {
    return call(try decodePluginValue(payload)) {
      reply(TokamakPluginResult($0.flatMap(encodePluginValue), done: true))
    }
  } catch {
    reply(TokamakPluginResult(.failure(error), done: true))
    return nil
  }
}

private func decodePluginValue(_ payload: Data) throws(TokamakPluginError) -> Any {
  do { return try JSONSerialization.jsonObject(with: payload, options: .fragmentsAllowed) } catch {
    throw .init(name: "DataCloneError", message: error.localizedDescription)
  }
}

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
  /// receive as `<plugin>.<name>`, with its encoded `payload`, and calls
  /// `completion` on the main thread with their encoded reply, which is empty
  /// when none replied. Fails unless they return within `timeout`, which
  /// includes runtime startup. Call it on the main thread.
  func emitBytes(
    _ name: String,
    payload: Data,
    timeout: TimeInterval,
    completion: @escaping (Result<Data, Error>) -> Void
  ) {
    app.emit("\(plugin).\(name)", payload: payload, timeout: timeout, completion: completion)
  }

  /// `emitBytes` with `event` encoded as JSON, calling `completion` with the
  /// reply parsed from JSON, or nil when none replied.
  func emit(
    _ name: String,
    event: Any,
    timeout: TimeInterval,
    completion: @escaping (Result<Any?, Error>) -> Void
  ) {
    let payload: Data
    switch encodePluginValue(event) {
    case .success(let encoded): payload = encoded
    case .failure(let error): return completion(.failure(error))
    }
    emitBytes(name, payload: payload, timeout: timeout) { result in
      completion(
        result.flatMap { reply in
          Result { () throws(TokamakPluginError) -> Any? in
            let value = reply.isEmpty ? NSNull() : try decodePluginValue(reply)
            return value is NSNull ? nil : value
          }.mapError { $0 }
        })
    }
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

  func callBytes(method: String, payload: Data, reply: @escaping TokamakPluginBytesReply) -> (
    () -> Void
  )?
  func callBytes(
    method: String, payload: Data, body: TokamakPluginBody,
    reply: @escaping TokamakPluginBytesReply
  ) -> (() -> Void)?
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

  func callBytes(method: String, payload: Data, reply: @escaping TokamakPluginBytesReply) -> (
    () -> Void
  )? {
    callJSON(payload, reply) { call(method: method, arguments: $0, reply: $1) }
  }

  func callBytes(
    method: String, payload: Data, body: TokamakPluginBody,
    reply: @escaping TokamakPluginBytesReply
  ) -> (() -> Void)? {
    callJSON(payload, reply) { call(method: method, arguments: $0, body: body, reply: $1) }
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
          request.cancellation = target.callBytes(
            method: method, payload: payload, body: body, reply: reply)
        } else {
          request.cancellation = target.callBytes(method: method, payload: payload, reply: reply)
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
