import Foundation
import WebKit

@main
enum PluginRequestsTests {
  static func main() {
    for address in ["https://app.example/next", "HTTPS://APP.EXAMPLE:443/next"] {
      precondition(URL(string: address)!.isAppOrigin("app.example"))
    }
    for address in [
      "https://app.example:444/next", "https://other.example/", "http://app.example/",
    ] {
      precondition(!URL(string: address)!.isAppOrigin("app.example"))
    }
    let app = TokamakApp()
    let plugin = TestPlugin(host: TokamakHost(app: app, plugin: "test"))
    var received: [(Int, TokamakPluginResult)] = []
    let requests = TokamakPluginRequests<Int>(plugins: [
      "test": plugin, "binary": BinaryPlugin(host: TokamakHost(app: app, plugin: "binary")),
    ]) {
      received.append(($0, $1))
    }
    func run(_ id: Int, _ method: String, subscribe: Bool = false) {
      requests.run(
        id, subscribe: subscribe, plugin: "test", method: method, payload: Data("null".utf8))
    }

    run(1, "abandoned")
    requests.cancel(1)
    requests.cancel(1)
    plugin.replies["abandoned"]?(.success("late"))
    drain()
    precondition(received.isEmpty)
    precondition(plugin.cancelled == ["abandoned"])

    run(2, "immediate")
    drain()
    precondition(received.count == 1 && received[0].0 == 2)
    precondition(received[0].1.done)
    requests.cancel(2)
    precondition(plugin.cancelled == ["abandoned"])

    run(3, "call")
    run(4, "stream", subscribe: true)
    requests.cancel(3)
    plugin.replies["call"]?(.success("late"))
    plugin.replies["stream"]?(.success("one"))
    plugin.replies["stream"]?(.success("two"))
    drain()
    precondition(received.map(\.0) == [2, 4, 4])
    precondition(received[1].1.done == false)
    requests.cancelAll()
    plugin.replies["stream"]?(.success("late"))
    drain()
    precondition(received.count == 3)
    precondition(Set(plugin.cancelled) == ["abandoned", "call", "stream"])

    run(5, "pending")
    requests.fail(5, .notSupported("failed"))
    plugin.replies["pending"]?(.success("late"))
    drain()
    precondition(received.count == 4 && received[3].0 == 5)
    precondition(received[3].1.done)
    precondition(plugin.cancelled.last == "pending")
    requests.run(
      6, subscribe: false, plugin: "binary", method: "echo", payload: Data([0, 255, 192, 175]))
    drain()
    precondition(received[4].1.payload == Data([0, 255, 192, 175]))
    precondition(!received[4].1.failed && received[4].1.done)
    requests.run(7, subscribe: false, plugin: "test", method: "immediate", payload: Data([255]))
    drain()
    precondition(received[5].1.failed && received[5].1.done)
    print("Passed native bytes, codecs, completion, cancellation, isolation, and late-reply tests")
  }

  private static func drain() {
    var delivered = false
    DispatchQueue.main.async { delivered = true }
    let deadline = Date().addingTimeInterval(2)
    while !delivered && Date() < deadline {
      RunLoop.main.run(until: Date().addingTimeInterval(0.001))
    }
    precondition(delivered, "The main queue did not drain")
  }
}

private final class TestPlugin: TokamakPlugin {
  let id = "test"
  var replies: [String: TokamakPluginReply] = [:]
  var cancelled: [String] = []
  init(host: TokamakHost) {}

  func call(
    method: String, arguments: Any, reply: @escaping TokamakPluginReply
  ) -> (() -> Void)? {
    replies[method] = reply
    if method == "immediate" { reply(.success("result")) }
    return { self.cancelled.append(method) }
  }

  func subscribe(
    method: String, arguments: Any, reply: @escaping TokamakPluginReply
  ) throws(TokamakPluginError) -> (() -> Void) {
    replies[method] = reply
    return { self.cancelled.append(method) }
  }
}

// The request dispatcher does not use the app's runtime or background-event facilities.
final class TokamakApp {
  weak var webView: WKWebView?
  var isInForeground = true
  func emit(
    _ name: String, event: Any, timeout: TimeInterval,
    completion: @escaping (Result<Any?, Error>) -> Void
  ) { preconditionFailure("Unexpected event") }
  func fetch(
    _ path: String, timeout: TimeInterval,
    completion: @escaping (Result<Data, Error>) -> Void
  ) { preconditionFailure("Unexpected fetch") }
}

enum TokamakBackgroundResult {
  case newData
  case noData
  case failed
}

private final class BinaryPlugin: TokamakPlugin {
  let id = "binary"
  init(host: TokamakHost) {}
  func callBytes(method: String, payload: Data, reply: @escaping TokamakPluginBytesReply) -> (
    () -> Void
  )? {
    reply(TokamakPluginResult(.success(payload), done: true))
    return nil
  }
}
