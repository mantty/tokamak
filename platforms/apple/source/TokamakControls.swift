import Foundation
import WebKit

/// Core device operations use the same endpoint and ownership as plugins.
final class TokamakCorePlugin: TokamakPlugin {
  let id = "tokamak"
  private let host: TokamakHost
  private let queue = DispatchQueue(label: "tokamak.core")

  init(host: TokamakHost) { self.host = host }

  /// WebView operations and the presenter run on the main thread, which
  /// WebKit and UIKit require.
  func executor(for method: String) -> DispatchQueue {
    method.hasPrefix("webview.") || method == "resource.root" ? .main : queue
  }

  func call(method: String, arguments: Any, reply: @escaping TokamakPluginReply) -> (() -> Void)? {
    switch method {
    case "resource.read":
      reply(
        Result { () throws(TokamakPluginError) -> Any? in
          try host.resources.get(arguments, as: Data.self)
        })
    case "resource.release":
      host.resources.release(arguments)
      reply(.success(nil))
    case "resource.root":
      reply(Result { () throws(TokamakPluginError) -> Any? in try root(arguments) })
    case "webview.getCurrentPage":
      reply(.success(host.webView?.url?.absoluteString))
    case "webview.navigate":
      reply(navigate(to: arguments).map { nil })
    case "webview.postMessage":
      postMessage(arguments, reply: reply)
    default:
      reply(.failure(.notSupported("\(id).\(method) is not supported")))
    }
    return nil
  }

  /// `resource.scope` opens a scope for the objects an invocation receives,
  /// which ends with the subscription.
  func subscribe(method: String, arguments: Any, reply: @escaping TokamakPluginReply)
    throws(TokamakPluginError) -> () -> Void
  {
    guard method == "resource.scope" else {
      throw .notSupported("\(id).\(method) is not supported")
    }
    let scope = host.resources.open()
    reply(.success(scope))
    return { [host] in host.resources.end(scope) }
  }

  /// Retains the app object `name` in `scope` and returns its ID: the
  /// `presenter` a native screen is presented from.
  private func root(_ arguments: Any) throws(TokamakPluginError) -> String {
    let arguments = arguments as? [String: Any]
    guard let scope = arguments?["scope"] as? String, arguments?["name"] as? String == "presenter"
    else { throw .notSupported("\(id).resource.root has no such object") }
    return try host.resources.retain(host.presenter(), in: scope)
  }

  /// Starts navigation to the app URL `path`, relative to the current page or
  /// the app origin.
  private func navigate(to path: Any) -> Result<Void, TokamakPluginError> {
    guard let view = host.webView else { return .failure(noView) }
    let appHost = Bundle.main.tokamakHost ?? ""
    guard let path = path as? String,
      let base = view.url ?? URL(string: "https://\(appHost)/"),
      let url = URL(string: path, relativeTo: base)?.absoluteURL,
      url.isAppOrigin(appHost)
    else {
      return .failure(.init(name: "TypeError", message: "Navigation requires an app URL"))
    }
    view.load(URLRequest(url: url))
    return .success(())
  }

  private func postMessage(_ data: Any, reply: @escaping TokamakPluginReply) {
    guard let view = host.webView, view.url != nil else {
      return reply(.failure(.init(name: "InvalidStateError", message: "No page is loaded")))
    }
    view.callAsyncJavaScript(
      "window.dispatchEvent(new MessageEvent('message', {data, origin: location.origin})); return true;",
      arguments: ["data": data], in: nil, in: .page
    ) { result in
      reply(result.map { _ in nil }.mapError { TokamakPluginError($0) })
    }
  }

  private var noView: TokamakPluginError {
    .init(name: "InvalidStateError", message: "The app has no WebView")
  }
}
