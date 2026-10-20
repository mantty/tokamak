import Foundation
import WebKit

struct TokamakPluginError: Error {
  let name: String
  let message: String

  static func notSupported(_ message: String) -> Self {
    Self(name: "NotSupportedError", message: message)
  }

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
  do {
    return .success(
      try JSONSerialization.data(
        withJSONObject: value ?? NSNull(), options: .fragmentsAllowed))
  } catch {
    return .failure(.init(name: "DataCloneError", message: error.localizedDescription))
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

  /// Delivers the plugin's event `name`, which the Worker's listeners
  /// receive as `<plugin>.<name>`, with `event`, JSON-serialisable, and calls
  /// `completion` on the main thread with their reply, parsed from JSON, or
  /// nil. Fails unless they return within `timeout`, which includes runtime
  /// startup. Call it on the main thread.
  func emit(
    _ name: String,
    event: Any,
    timeout: TimeInterval,
    completion: @escaping (Result<Any?, Error>) -> Void
  ) {
    app.emit("\(plugin).\(name)", event: event, timeout: timeout, completion: completion)
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
  func subscribeBytes(method: String, payload: Data, reply: @escaping TokamakPluginBytesReply)
    throws(TokamakPluginError) -> (() -> Void)

  /// Returns cancellation for pending work when the platform supports it.
  func call(
    method: String,
    arguments: Any,
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
    do {
      return call(method: method, arguments: try decodePluginValue(payload)) {
        reply(TokamakPluginResult($0.flatMap(encodePluginValue), done: true))
      }
    } catch {
      reply(TokamakPluginResult(.failure(error), done: true))
      return nil
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

/// Runs the Worker's requests on the executor each plugin chooses, and passes
/// replies straight to the runtime.
final class TokamakPluginRequests<Key: Hashable> {
  /// A pending call or subscription. Only its serial executor touches
  /// `cancellation`, so a cancellation runs after the operation started.
  private final class Request {
    let queue: DispatchQueue
    var cancellation: (() -> Void)?
    init(queue: DispatchQueue) { self.queue = queue }
  }

  private let plugins: [String: any TokamakPlugin]
  private let send: (Key, TokamakPluginResult) -> Void
  private let lock = NSLock()
  private var pending: [Key: Request] = [:]

  init(plugins: [String: any TokamakPlugin], send: @escaping (Key, TokamakPluginResult) -> Void) {
    self.plugins = plugins
    self.send = send
  }

  func run(_ key: Key, subscribe: Bool, plugin: String, method: String, payload: Data) {
    guard let target = plugins[plugin], !method.isEmpty else {
      send(key, TokamakPluginResult(.failure(.notSupported("Plugin is not supported")), done: true))
      return
    }
    let request = Request(queue: target.executor(for: method))
    lock.withLock { pending[key] = request }
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
        request.cancellation =
          try subscribe
          ? target.subscribeBytes(method: method, payload: payload, reply: reply)
          : target.callBytes(method: method, payload: payload, reply: reply)
      } catch {
        if remove(key, request) { send(key, TokamakPluginResult(.failure(error), done: true)) }
      }
    }
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
