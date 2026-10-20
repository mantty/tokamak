import Foundation
import WebKit

/// Core device operations use the same endpoint and ownership as plugins.
final class TokamakCorePlugin: TokamakPlugin {
  let id = "tokamak"
  private let host: TokamakHost

  init(host: TokamakHost) { self.host = host }

  func call(method: String, arguments: Any, reply: @escaping TokamakPluginReply) -> (() -> Void)? {
    guard let view = host.webView, let page = view.url else {
      reply(.failure(.init(name: "InvalidStateError", message: "No page is loaded")))
      return nil
    }
    switch method {
    case "webview.getCurrentPage":
      reply(.success(page.absoluteString))
    case "webview.navigate":
      guard let path = arguments as? String,
        let url = URL(string: path, relativeTo: page)?.absoluteURL,
        url.isAppOrigin(page.host ?? "")
      else {
        reply(.failure(.init(name: "TypeError", message: "Navigation requires an app URL")))
        return nil
      }
      view.load(URLRequest(url: url))
      reply(.success(nil))
    case "webview.postMessage":
      view.callAsyncJavaScript(
        "window.dispatchEvent(new MessageEvent('message', {data, origin: location.origin})); return true;",
        arguments: ["data": arguments], in: nil, in: .page
      ) { result in
        reply(
          result.map { _ in nil }.mapError {
            TokamakPluginError(name: "OperationError", message: $0.localizedDescription)
          })
      }
    default:
      reply(.failure(.notSupported("\(id).\(method) is not supported")))
    }
    return nil
  }
}
