import Foundation
import TokamakPlugin
import WebKit

/// A shell that creates the registry's plugins and starts nothing.
final class TokamakApp: TokamakHostApp {
  let webView: WKWebView? = nil
  let resources = TokamakResources()
  private(set) var plugins: [String: any TokamakPlugin] = [:]
  #if os(iOS)
    let isInForeground = true
  #endif

  init() {
    for plugin in tokamakPlugins(app: self) {
      plugins[plugin.id] = plugin
    }
  }

  func emit(
    _ name: String, payload: Data, timeout: TimeInterval,
    completion: @escaping (Result<Data, Error>) -> Void
  ) {}

  func fetch(
    _ path: String, timeout: TimeInterval, completion: @escaping (Result<Data, Error>) -> Void
  ) {}
}

@main struct App {
  static func main() {
    _ = TokamakApp()
  }
}
