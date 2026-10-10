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

/// The app as one plugin sees it.
final class TokamakHost {
  private unowned let app: TokamakApp
  private let plugin: String

  init(app: TokamakApp, plugin: String) {
    self.app = app
    self.plugin = plugin
  }

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
}

/// A native plugin, created once per process when the app launches. The page
/// and the Worker call it on the main thread.
protocol TokamakPlugin: AnyObject {
  var id: String { get }

  init(host: TokamakHost)

  func call(
    method: String,
    arguments: Any,
    reply: @escaping TokamakPluginReply
  )

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
  func call(
    method: String,
    arguments: Any,
    reply: @escaping TokamakPluginReply
  ) {
    reply(.failure(.notSupported("\(id).\(method) is not supported")))
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

/// Runs one caller's calls and subscriptions, the page's or the Worker's, on
/// the app's plugins, keyed by `Key`. `send` receives each result on the main
/// thread, as the JSON object the page's native transport receives without its
/// session and ID. Use it on the main thread.
final class TokamakPluginRequests<Key: Hashable> {
  private let plugins: [String: any TokamakPlugin]
  private let send: (Key, [String: Any]) -> Void
  private var cancellations: [Key: () -> Void] = [:]

  init(plugins: [String: any TokamakPlugin], send: @escaping (Key, [String: Any]) -> Void) {
    self.plugins = plugins
    self.send = send
  }

  /// Calls `method` of `plugin`, or subscribes to it when `subscribe`.
  func run(_ key: Key, subscribe: Bool, plugin: String, method: String, arguments: Any) {
    guard let target = plugins[plugin], !method.isEmpty else {
      fail(key, .notSupported("Plugin is not supported"))
      return
    }
    // Plugins reply from any thread.
    let reply: TokamakPluginReply = { [weak self] result in
      DispatchQueue.main.async {
        self?.send(key, Self.result(result, done: !subscribe))
      }
    }
    guard subscribe else {
      target.call(method: method, arguments: arguments, reply: reply)
      return
    }
    do {
      cancellations[key] = try target.subscribe(method: method, arguments: arguments, reply: reply)
    } catch {
      fail(key, error)
    }
  }

  /// Ends the request `key` with `error`.
  func fail(_ key: Key, _ error: TokamakPluginError) {
    send(key, Self.result(.failure(error), done: true))
  }

  /// Ends the subscription `key`.
  func cancel(_ key: Key) {
    cancellations.removeValue(forKey: key)?()
  }

  /// Ends every subscription.
  func cancelAll() {
    let cancelAll = Array(cancellations.values)
    cancellations.removeAll()
    for cancel in cancelAll {
      cancel()
    }
  }

  /// `result` as JSON, or the failure to make it JSON.
  private static func result(
    _ result: Result<Any?, TokamakPluginError>,
    done: Bool
  ) -> [String: Any] {
    switch result {
    case .success(let value):
      let json: [String: Any] = ["value": value ?? NSNull(), "done": done]
      if JSONSerialization.isValidJSONObject(json) {
        return json
      }
      let error = TokamakPluginError(
        name: "OperationError", message: "The plugin's value is not JSON")
      return Self.result(.failure(error), done: done)
    case .failure(let error):
      return ["error": ["name": error.name, "message": error.message], "done": done]
    }
  }
}

final class TokamakPluginBridge: NSObject, WKScriptMessageHandler {
  private struct RequestKey: Hashable {
    let session: String
    let id: Int
  }

  private let host: String
  private let plugins: [String: any TokamakPlugin]
  private lazy var requests = TokamakPluginRequests<RequestKey>(plugins: plugins) {
    [weak self] key, result in
    self?.deliver(key: key, result: result)
  }
  weak var webView: WKWebView?

  init(host: String, plugins: [String: any TokamakPlugin]) {
    self.host = host
    self.plugins = plugins
  }

  func install(in controller: WKUserContentController) {
    controller.add(self, name: "tokamak")
    controller.addUserScript(
      WKUserScript(
        source: Self.bootstrap(host: host),
        injectionTime: .atDocumentStart,
        forMainFrameOnly: true
      )
    )
  }

  func close() {
    requests.cancelAll()
  }

  func userContentController(
    _ userContentController: WKUserContentController,
    didReceive message: WKScriptMessage
  ) {
    guard
      message.frameInfo.isMainFrame,
      message.frameInfo.securityOrigin.isAppOrigin(host),
      let encoded = message.body as? String,
      let data = encoded.data(using: .utf8),
      let request = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
      let type = request["type"] as? String,
      let session = request["session"] as? String
    else {
      return
    }

    if type == "reset" {
      close()
      return
    }
    guard let id = request["id"] as? Int else { return }
    let key = RequestKey(session: session, id: id)
    switch type {
    case "cancel":
      requests.cancel(key)
    case "call", "subscribe":
      requests.run(
        key,
        subscribe: type == "subscribe",
        plugin: request["plugin"] as? String ?? "",
        method: request["method"] as? String ?? "",
        arguments: request["arguments"] ?? NSNull()
      )
    default:
      requests.fail(key, .notSupported("Plugin operation is not supported"))
    }
  }

  private func deliver(key: RequestKey, result: [String: Any]) {
    var response = result
    response["session"] = key.session
    response["id"] = key.id
    guard
      let data = try? JSONSerialization.data(withJSONObject: response),
      let json = String(data: data, encoding: .utf8),
      webView?.url?.isAppOrigin(host) == true
    else {
      return
    }
    webView?.callAsyncJavaScript(
      "globalThis.__tokamakNative?.onmessage?.({ data })",
      arguments: ["data": json],
      in: nil,
      in: .page
    )
  }

  private static func bootstrap(host: String) -> String {
    """
    if (globalThis.location.origin === "https://\(host)") {
      globalThis.__tokamakNative = {
        onmessage: null,
        postMessage(message) {
          globalThis.webkit.messageHandlers.tokamak.postMessage(message);
        }
      };
    }
    """
  }
}
