import Foundation

@main
enum PluginRequestsTests {
  static func main() {
    let app = TokamakApp()
    let plugin = TestPlugin(host: TokamakHost(app: app, plugin: "test"))
    var received: [(Int, [String: Any])] = []
    let requests = TokamakPluginRequests<Int>(plugins: ["test": plugin]) {
      received.append(($0, $1))
    }
    func run(_ id: Int, _ method: String, subscribe: Bool = false) {
      requests.run(id, subscribe: subscribe, plugin: "test", method: method, arguments: NSNull())
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
    precondition(received[0].1["done"] as? Bool == true)
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
    precondition(received[1].1["done"] as? Bool == false)
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
    precondition(received[3].1["done"] as? Bool == true)
    precondition(plugin.cancelled.last == "pending")
    print("Passed native request completion, cancellation, isolation, and late-reply tests")
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
