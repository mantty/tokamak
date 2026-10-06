import Foundation
import WebKit

struct TokamakPluginError: Error {
  let name: String
  let message: String

  static func notSupported(_ message: String) -> Self {
    Self(name: "NotSupportedError", message: message)
  }
}

typealias TokamakPluginReply = (Result<Any?, TokamakPluginError>) -> Void

/// A native plugin, created once per process when the app launches.
protocol TokamakPlugin: AnyObject {
  var id: String { get }

  init(host: TokamakHost)

  func call(
    method: String,
    arguments: Any,
    reply: @escaping TokamakPluginReply
  )

  func subscribe(
    method: String,
    arguments: Any,
    reply: @escaping TokamakPluginReply
  ) -> (() -> Void)

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
  ) -> (() -> Void) {
    reply(.failure(.notSupported("\(id).\(method) is not supported")))
    return {}
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

final class TokamakPluginBridge: NSObject, WKScriptMessageHandler {
  private struct RequestKey: Hashable {
    let session: String
    let id: Int
  }

  private let host: String
  private let plugins: [String: any TokamakPlugin]
  private var cancellations: [RequestKey: () -> Void] = [:]
  weak var webView: WKWebView?

  init(host: String, plugins: [any TokamakPlugin]) {
    self.host = host
    var pluginsById: [String: any TokamakPlugin] = [:]
    for plugin in plugins {
      precondition(pluginsById[plugin.id] == nil, "Duplicate plugin ID")
      pluginsById[plugin.id] = plugin
    }
    self.plugins = pluginsById
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
    let cancelAll = Array(cancellations.values)
    cancellations.removeAll()
    for cancel in cancelAll {
      cancel()
    }
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
    if type == "cancel" {
      cancellations.removeValue(forKey: key)?()
      return
    }
    dispatch(type: type, key: key, request: request)
  }

  private func dispatch(type: String, key: RequestKey, request: [String: Any]) {
    guard
      let pluginId = request["plugin"] as? String,
      let method = request["method"] as? String,
      let plugin = plugins[pluginId]
    else {
      send(key: key, result: .failure(.notSupported("Plugin is not supported")), done: true)
      return
    }
    let arguments = request["arguments"] ?? NSNull()
    let reply: TokamakPluginReply = { [weak self] result in
      self?.send(key: key, result: result, done: type == "call")
    }
    switch type {
    case "call":
      plugin.call(method: method, arguments: arguments, reply: reply)
    case "subscribe":
      let cancellation = plugin.subscribe(
        method: method,
        arguments: arguments,
        reply: reply
      )
      cancellations[key] = cancellation
    default:
      send(
        key: key, result: .failure(.notSupported("Plugin operation is not supported")), done: true)
    }
  }

  /// Plugins reply from any thread; delivery happens on the main thread.
  private func send(key: RequestKey, result: Result<Any?, TokamakPluginError>, done: Bool) {
    DispatchQueue.main.async { [weak self] in
      self?.deliver(key: key, result: result, done: done)
    }
  }

  private func deliver(key: RequestKey, result: Result<Any?, TokamakPluginError>, done: Bool) {
    if done {
      cancellations.removeValue(forKey: key)?()
    }
    var response: [String: Any] = [
      "session": key.session,
      "id": key.id,
      "done": done,
    ]
    switch result {
    case .success(let value):
      response["value"] = value ?? NSNull()
    case .failure(let error):
      response["error"] = ["name": error.name, "message": error.message]
    }
    guard
      JSONSerialization.isValidJSONObject(response),
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
