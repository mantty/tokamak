import Foundation
import TokamakPlugin
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
        id, subscribe ? .subscribe : .call, plugin: "test", method: method,
        payload: Data("null".utf8))
      drain()
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
    drain()
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
    requests.cancel(4)
    plugin.replies["stream"]?(.success("late"))
    drain()
    precondition(received.count == 3)
    precondition(Set(plugin.cancelled) == ["abandoned", "call", "stream"])

    requests.run(5, .call, plugin: "missing", method: "call", payload: Data())
    precondition(received.count == 4 && received[3].0 == 5)
    precondition(received[3].1.failed && received[3].1.done)
    requests.run(
      6, .call, plugin: "binary", method: "echo", payload: Data([0, 255, 192, 175]))
    drain()
    precondition(received[4].1.payload == Data([0, 255, 192, 175]))
    precondition(!received[4].1.failed && received[4].1.done)
    requests.run(7, .call, plugin: "test", method: "immediate", payload: Data([255]))
    drain()
    precondition(received[5].1.failed && received[5].1.done)
    let replied = DispatchSemaphore(value: 0)
    let background = TokamakPluginRequests<Int>(plugins: [
      "binary": BinaryPlugin(host: TokamakHost(app: app, plugin: "binary"))
    ]) { _, result in
      precondition(!Thread.isMainThread)
      precondition(result.payload == Data([0, 255]))
      replied.signal()
    }
    background.run(
      8, .call, plugin: "binary", method: "background", payload: Data([0, 255]))
    precondition(
      replied.wait(timeout: .now() + 2) == .success, "A native call required the blocked UI thread")
    for method in ["immediate", "deferred"] {
      weak var released: TestPlugin?
      do {
        let completing = TestPlugin(host: TokamakHost(app: app, plugin: "test"))
        released = completing
        let calls = TokamakPluginRequests<Int>(plugins: ["test": completing]) { _, _ in }
        calls.run(9, .call, plugin: "test", method: method, payload: Data("null".utf8))
        drain()
        if method == "deferred" { completing.replies[method]?(.success("done")) }
        drain()
      }
      drain()
      precondition(released == nil, "A completed call retained its plugin through cancellation")
    }
    for subscribe in [false, true] {
      let gated = GatedPlugin(host: TokamakHost(app: app, plugin: "gated"))
      let calls = TokamakPluginRequests<Int>(plugins: ["gated": gated]) { _, _ in
        preconditionFailure("A cancelled operation delivered a late reply")
      }
      calls.run(
        10, subscribe ? .subscribe : .call, plugin: "gated", method: "wait", payload: Data())
      precondition(gated.entered.wait(timeout: .now() + 2) == .success)
      calls.cancel(10)
      calls.cancel(10)
      gated.release.signal()
      precondition(gated.cancelled.wait(timeout: .now() + 2) == .success)
      gated.queue.sync {}
      precondition(gated.cancellations == 1)
    }
    let queued = GatedPlugin(host: TokamakHost(app: app, plugin: "gated"))
    queued.queue.async {
      queued.entered.signal()
      queued.release.wait()
    }
    precondition(queued.entered.wait(timeout: .now() + 2) == .success)
    let cancelledBeforeStart = TokamakPluginRequests<Int>(plugins: ["gated": queued]) { _, _ in
      preconditionFailure("An unstarted operation replied")
    }
    cancelledBeforeStart.run(11, .call, plugin: "gated", method: "wait", payload: Data())
    cancelledBeforeStart.cancel(11)
    queued.release.signal()
    queued.queue.sync {}
    precondition(queued.calls == 0)
    testResources(app)
    testBodies(app)
    print(
      "Passed native bytes, codecs, completion, cancellation, isolation, late-reply, resource and body tests"
    )
  }

  /// A call reads its body one chunk per pull, on its executor, until the
  /// last chunk or the Worker's error.
  private static func testBodies(_ app: TokamakApp) {
    let reader = BodyReader(host: TokamakHost(app: app, plugin: "reader"))
    let lock = NSLock()
    var results: [Int: TokamakPluginResult] = [:]
    let pulled = DispatchSemaphore(value: 0)
    let replied = DispatchSemaphore(value: 0)
    let requests = TokamakPluginRequests<Int>(
      plugins: ["reader": reader],
      send: { key, result in
        lock.withLock { results[key] = result }
        replied.signal()
      },
      pull: { _ in pulled.signal() })
    func send(_ key: Int, _ chunks: [TokamakPluginResult]) -> TokamakPluginResult {
      requests.run(key, .callWithBody, plugin: "reader", method: "sum", payload: Data("null".utf8))
      for chunk in chunks {
        precondition(pulled.wait(timeout: .now() + 2) == .success, "The plugin did not pull")
        requests.chunk(key, chunk)
      }
      precondition(replied.wait(timeout: .now() + 2) == .success, "The plugin did not reply")
      return lock.withLock { results[key]! }
    }
    let summed = send(
      1,
      [
        TokamakPluginResult(payload: Data([1, 2]), failed: false, done: false),
        TokamakPluginResult(payload: Data([3]), failed: false, done: false),
        TokamakPluginResult(payload: Data(), failed: false, done: true),
      ])
    precondition(String(decoding: summed.payload, as: UTF8.self) == "6")
    let failed = send(
      2,
      [
        TokamakPluginResult(
          .failure(.init(name: "NotReadableError", message: "Stopped")), done: true)
      ])
    precondition(
      failed.failed && String(decoding: failed.payload, as: UTF8.self).contains("NotReadableError"))
  }

  /// A retained object reaches another plugin as itself, its bytes are read
  /// off the main thread, and cancelling the retaining subscription releases it.
  private static func testResources(_ app: TokamakApp) {
    let producer = Producer(host: TokamakHost(app: app, plugin: "producer"))
    let consumer = Consumer(host: TokamakHost(app: app, plugin: "consumer"))
    let lock = NSLock()
    var results: [Int: TokamakPluginResult] = [:]
    let replied = DispatchSemaphore(value: 0)
    let requests = TokamakPluginRequests<Int>(plugins: [
      "producer": producer, "consumer": consumer,
      "tokamak": TokamakCorePlugin(host: TokamakHost(app: app, plugin: "tokamak")),
    ]) { key, result in
      if key == 3 { precondition(!Thread.isMainThread, "Reading bytes waited for the UI thread") }
      lock.withLock { results[key] = result }
      replied.signal()
    }
    func request(
      _ key: Int, _ plugin: String, _ method: String, _ payload: String, subscribe: Bool = false
    )
      -> TokamakPluginResult
    {
      requests.run(
        key, subscribe ? .subscribe : .call, plugin: plugin, method: method,
        payload: Data(payload.utf8))
      while lock.withLock({ results[key] }) == nil {
        drain()
        _ = replied.wait(timeout: .now() + 0.01)
      }
      return lock.withLock { results.removeValue(forKey: key)! }
    }

    let retained = request(1, "producer", "frame", "null", subscribe: true)
    precondition(!retained.failed && !retained.done)
    let id = String(decoding: retained.payload, as: UTF8.self)
    let encoded = request(2, "consumer", "encode", "{\"frame\":\(id)}")
    precondition(String(decoding: encoded.payload, as: UTF8.self) == "3")
    precondition(consumer.received === producer.frame, "The consumer received a copy")
    let bytes = request(3, "tokamak", "resource.read", id)
    precondition(!bytes.failed && bytes.payload == Data([0, 255, 7]))
    let object = request(4, "producer", "object", "null", subscribe: true)
    let wrongType = request(
      5, "tokamak", "resource.read", String(decoding: object.payload, as: UTF8.self))
    precondition(
      wrongType.failed && String(decoding: wrongType.payload, as: UTF8.self).contains("TypeError"))
    requests.cancel(1)
    drain()
    let released = request(6, "consumer", "encode", "{\"frame\":\(id)}")
    precondition(
      released.failed
        && String(decoding: released.payload, as: UTF8.self).contains("InvalidStateError"))
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

private final class GatedPlugin: TokamakPlugin {
  let id = "gated"
  let queue = DispatchQueue(label: "test.gated")
  let entered = DispatchSemaphore(value: 0)
  let release = DispatchSemaphore(value: 0)
  let cancelled = DispatchSemaphore(value: 0)
  var cancellations = 0
  var calls = 0
  init(host: TokamakHost) {}
  func executor(for method: String) -> DispatchQueue { queue }
  func callBytes(method: String, payload: Data, reply: @escaping TokamakPluginBytesReply) -> (
    () -> Void
  )? {
    begin(reply)
  }
  func subscribeBytes(method: String, payload: Data, reply: @escaping TokamakPluginBytesReply)
    throws(TokamakPluginError) -> (() -> Void)
  { begin(reply) }
  private func begin(_ reply: @escaping TokamakPluginBytesReply) -> (() -> Void) {
    calls += 1
    entered.signal()
    precondition(release.wait(timeout: .now() + 2) == .success)
    return {
      dispatchPrecondition(condition: .onQueue(self.queue))
      self.cancellations += 1
      reply(TokamakPluginResult(.success(Data()), done: true))
      self.cancelled.signal()
    }
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

  func subscribeBytes(method: String, payload: Data, reply: @escaping TokamakPluginBytesReply)
    throws(TokamakPluginError) -> (() -> Void)
  {
    replies[method] = { reply(TokamakPluginResult($0.flatMap(encodePluginValue), done: false)) }
    return { self.cancelled.append(method) }
  }
}

// The request dispatcher does not use the app's runtime or background-event facilities.
final class TokamakApp: TokamakHostApp {
  weak var webView: WKWebView?
  let resources = TokamakResources()
  var isInForeground = true
  var plugins: [String: any TokamakPlugin] = [:]
  func emit(
    _ name: String, payload: Data, timeout: TimeInterval,
    completion: @escaping (Result<Data, Error>) -> Void
  ) { preconditionFailure("Unexpected event") }
  func fetch(
    _ path: String, timeout: TimeInterval,
    completion: @escaping (Result<Data, Error>) -> Void
  ) { preconditionFailure("Unexpected fetch") }
}

private final class BinaryPlugin: TokamakPlugin {
  let id = "binary"
  private let queue = DispatchQueue(label: "test.binary")
  init(host: TokamakHost) {}
  func executor(for method: String) -> DispatchQueue { method == "background" ? queue : .main }
  func callBytes(method: String, payload: Data, reply: @escaping TokamakPluginBytesReply) -> (
    () -> Void
  )? {
    reply(TokamakPluginResult(.success(payload), done: true))
    return nil
  }
}

private final class Producer: TokamakPlugin {
  let id = "producer"
  let frame = NSData(bytes: [0, 255, 7] as [UInt8], length: 3)
  private let host: TokamakHost
  init(host: TokamakHost) { self.host = host }
  func subscribeBytes(method: String, payload: Data, reply: @escaping TokamakPluginBytesReply)
    throws(TokamakPluginError) -> (() -> Void)
  {
    host.resources.retain(method == "frame" ? frame : NSObject(), reply: reply)
  }
}

private final class Consumer: TokamakPlugin {
  let id = "consumer"
  private(set) var received: NSData?
  private let host: TokamakHost
  init(host: TokamakHost) { self.host = host }
  func call(method: String, arguments: Any, reply: @escaping TokamakPluginReply) -> (() -> Void)? {
    reply(
      Result { () throws(TokamakPluginError) -> Any? in
        let frame = (arguments as? [String: Any])?["frame"] as Any
        received = try host.resources.get(frame, as: NSData.self)
        return received?.length
      })
    return nil
  }
}

private final class BodyReader: TokamakPlugin {
  let id = "reader"
  private let queue = DispatchQueue(label: "test.reader")
  init(host: TokamakHost) {}
  func executor(for method: String) -> DispatchQueue { queue }
  func callBytes(
    method: String, payload: Data, body: TokamakPluginBody,
    reply: @escaping TokamakPluginBytesReply
  ) -> (() -> Void)? {
    var sum = 0
    func next() {
      body.read { chunk in
        dispatchPrecondition(condition: .onQueue(self.queue))
        switch chunk {
        case .success(let bytes?):
          sum += bytes.reduce(0) { $0 + Int($1) }
          next()
        case .success(nil): reply(TokamakPluginResult(encodePluginValue(sum), done: true))
        case .failure(let error): reply(TokamakPluginResult(.failure(error), done: true))
        }
      }
    }
    next()
    return nil
  }
}
