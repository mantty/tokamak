import Foundation
import WebKit

/// Core device operations use the same endpoint and ownership as plugins.
final class TokamakCorePlugin: TokamakPlugin {
  let id = "tokamak"
  private let host: TokamakHost
  private let queue = DispatchQueue(label: "tokamak.core")

  init(host: TokamakHost) { self.host = host }

  /// WebView operations run on the main thread, which WebKit requires.
  func executor(for method: String) -> DispatchQueue {
    method.hasPrefix("webview.") ? .main : queue
  }

  func call(method: String, arguments: Any, reply: @escaping TokamakPluginReply) -> (() -> Void)? {
    switch method {
    case "resource.read":
      reply(
        Result { () throws(TokamakPluginError) -> Any? in
          try host.resources.get(arguments, as: Data.self)
        })
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
      reply(
        result.map { _ in nil }.mapError {
          TokamakPluginError(name: "OperationError", message: $0.localizedDescription)
        })
    }
  }

  private var noView: TokamakPluginError {
    .init(name: "InvalidStateError", message: "The app has no WebView")
  }
}
