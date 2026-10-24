import Foundation
import WebKit

/// Runs the Worker's requests through the fixture plugin's generated dispatch.
@main
enum DispatchTests {
  static func main() {
    let app = TokamakApp()
    let plugin = TokamakExamplePlugin(host: TokamakHost(app: app, plugin: "example"))
    let replies = Replies()
    var pulled = 0
    let requests = TokamakPluginRequests<Int>(
      plugins: ["example": plugin], send: replies.receive, pull: { _ in pulled += 1 })
    var next = 0
    func request(
      _ method: String, _ payload: String, _ kind: TokamakPluginRequest = .call
    ) -> Int {
      next += 1
      requests.run(next, kind, plugin: "example", method: method, payload: Data(payload.utf8))
      return next
    }
    func reply(_ id: Int) -> TokamakPluginResult { replies.wait(for: id).last! }
    func text(_ result: TokamakPluginResult) -> String {
      String(decoding: result.payload, as: UTF8.self)
    }

    let region =
      #"{"id":"home","radius":50,"tags":["a",null],"counts":{"visits":2},"data":{"n":[1,true,null]}}"#
    let added = reply(request("addRegion", #"{"region":\#(region)}"#))
    precondition(!added.failed && added.done && text(added) == "null", text(added))
    let regions = try! JSONSerialization.jsonObject(with: reply(request("regions", "{}")).payload)
    precondition((regions as! [[String: Any]]).map { $0["id"] as! String } == ["home"])
    // A nil field is absent, as Codable encodes it.
    precondition((regions as! [[String: Any]])[0]["onEntry"] == nil)
    let data = (regions as! [[String: Any]])[0]["data"] as! [String: [Any]]
    precondition(data["n"]![0] as! Double == 1 && data["n"]![1] as! Bool && data["n"]![2] is NSNull)

    let invalid = reply(request("addRegion", #"{"region":{"id":1}}"#))
    precondition(invalid.failed && text(invalid).contains("region.id"), text(invalid))
    let missing = reply(request("addRegion", "{}"))
    precondition(missing.failed && text(missing).contains("region is missing"), text(missing))
    #if os(macOS)
      let unsupported = reply(request("permission", #"{"precision":"precise"}"#))
      precondition(text(unsupported).contains("NotSupportedError"), text(unsupported))
    #endif

    precondition(reply(request("read", #"{"path":"a.txt"}"#)).payload == Data([9]))
    precondition(text(reply(request("read", #"{"path":"b.txt"}"#))).contains("NotFoundError"))
    precondition(text(reply(request("digest", "\u{1}\u{2}"))) == #""0102""#)
    precondition(reply(request("copy", "\u{0}\u{FF}")).payload == Data("\u{0}\u{FF}".utf8))
    let mirrored = reply(request("mirror", #"{"value":{"a":[1,null]}}"#))
    precondition(text(mirrored) == #"{"a":[1,null]}"#, text(mirrored))

    // A union's form is chosen by its JSON, and encodes as it decodes, in any key order.
    func json(_ text: String) -> NSObject {
      try! JSONSerialization.jsonObject(with: Data(text.utf8), options: .fragmentsAllowed)
        as! NSObject
    }
    let circle = #"{"kind":"circle","radius":2}"#
    for (alert, encoded) in [
      (#""muted""#, #"["muted","silent"]"#), ("0.5", "[0.5]"), ("[1,2]", "[[1,2]]"),
      ("true", "[true]"), (#"{"frequency":440}"#, #"[{"frequency":440}]"#),
      (#"{"colour":"red","volume":1}"#, #"[{"colour":"red","volume":1}]"#),
    ] {
      let zone = alert == #""muted""# ? #"{"kind":"everywhere"}"# : circle
      let alerted = reply(request("alert", #"{"alert":\#(alert),"zone":\#(zone)}"#))
      precondition(json(text(alerted)) == json(encoded), text(alerted))
    }
    let zone = text(reply(request("zone", #"{"zone":\#(circle)}"#)))
    precondition(json(zone) == json(circle), zone)
    for mark in [#""none""#, #""approximate""#, #"{"a":"b"}"#] {
      let marked = text(reply(request("mark", #"{"mark":\#(mark)}"#)))
      precondition(json(marked) == json(mark), marked)
    }
    for (alert, zone, problem) in [
      (#""loud""#, circle, "alert must be one of Alert's forms"),
      (#"{"volume":1}"#, circle, "alert must be one of Alert's forms"),
      ("1", #"{"kind":"square"}"#, "zone must be one of Zone's forms"),
      ("1", #"{"radius":2}"#, "zone.kind is missing"),
    ] {
      let rejected = reply(request("alert", #"{"alert":\#(alert),"zone":\#(zone)}"#))
      precondition(rejected.failed && text(rejected).contains(problem), text(rejected))
    }

    let positions = replies.wait(
      for: request("watchPosition", #"{"every":5}"#, .subscribe), count: 2)
    precondition(text(positions[0]).contains(#""id":"5.0""#) && !positions[0].done)
    precondition(positions[1].payload.isEmpty && positions[1].done)
    let visits = reply(request("onVisit", "{}", .subscribe))
    precondition(visits.failed && visits.done && text(visits).contains("NotAllowedError"))

    let frame = request("capture", #"{"quality":7}"#, .subscribe)
    let id = try! JSONSerialization.jsonObject(
      with: reply(frame).payload, options: .fragmentsAllowed)
    precondition(text(reply(request("encode", #"{"frame":"\#(id)"}"#))) == "2")
    requests.cancel(frame)
    drain()
    let released = reply(request("encode", #"{"frame":"\#(id)"}"#))
    precondition(text(released).contains("The native reference is closed"), text(released))

    let upload = request("upload", #"{"name":"a.txt"}"#, .callWithBody)
    let deadline = Date().addingTimeInterval(2)
    while pulled == 0 && Date() < deadline { drain() }
    requests.chunk(
      upload, TokamakPluginResult(payload: Data([1, 2, 3]), failed: false, done: false))
    precondition(text(reply(upload)) == "3")

    testCancellation()
    testListeners()
    testJSON()
    testEvents()
    print("dispatch: ok")
  }

  /// Cancelling a call or a stream ends the method's work, and a resource
  /// whose method returns after cancellation is released.
  static func testCancellation() {
    let ignore: TokamakPluginBytesReply = { _ in }
    let ended = Flag()
    let stopStream = try! tokamakStream(ignore) {
      AsyncStream<Int> { continuation in continuation.onTermination = { _ in ended.set() } }
    }
    stopStream()
    wait(until: ended)

    let cancelled = Flag()
    let stopCall = tokamakCall(ignore) { () async throws -> Void in
      do {
        try await Task.sleep(for: .seconds(10))
      } catch {
        cancelled.set()
        throw error
      }
    }
    stopCall!()
    wait(until: cancelled)

    let resources = TokamakResources()
    let replies = Replies()
    let reply: TokamakPluginBytesReply = { replies.receive(0, $0) }
    let stopResource = tokamakResource(resources, reply) { () async throws -> Any in
      try? await Task.sleep(for: .milliseconds(20))
      return Data([1])
    }
    stopResource()
    let id = try! JSONSerialization.jsonObject(
      with: replies.wait(for: 0).last!.payload, options: .fragmentsAllowed)
    precondition((try? resources.get(id, as: Data.self)) == nil)
  }

  /// Held values reach the first listener only, after which values wait
  /// again until a listener starts; values without listeners are dropped
  /// unless the listeners hold them, and a slow reader misses none.
  static func testListeners() {
    let replies = Replies()
    var next = 0
    func listen(_ stream: AsyncStream<Int>) -> (id: Int, stop: () -> Void) {
      next += 1
      let id = next
      let reply: TokamakPluginBytesReply = { replies.receive(id, $0) }
      return (id, try! tokamakStream(reply) { stream })
    }
    func values(_ id: Int, _ count: Int) -> [String] {
      replies.wait(for: id, count: count).map { String(decoding: $0.payload, as: UTF8.self) }
    }

    let held = TokamakListeners<Int>(holding: true)
    held.send(1)
    held.send(2)
    let first = listen(held.stream())
    let second = listen(held.stream())
    held.send(3)
    precondition(values(first.id, 3) == ["1", "2", "3"] && values(second.id, 1) == ["3"])
    first.stop()
    second.stop()
    held.send(4)
    precondition(values(listen(held.stream()).id, 1) == ["4"])

    // Without listeners, the newest 64 values wait.
    let many = TokamakListeners<Int>(holding: true)
    for value in 0..<70 { many.send(value) }
    precondition(values(listen(many.stream()).id, 64) == (6..<70).map(String.init))

    let dropping = TokamakListeners<Int>()
    dropping.send(1)
    let listener = listen(dropping.stream())
    dropping.send(2)
    precondition(values(listener.id, 1) == ["2"])

    // The Worker's subscription, not the stream, applies the overflow policy.
    let burst = TokamakListeners<Int>()
    let unread = burst.stream()
    for value in 0..<70 { burst.send(value) }
    precondition(values(listen(unread).id, 70) == (0..<70).map(String.init))
  }

  /// Values read as `JSONSerialization` gives them, which code written for it casts.
  static func testJSON() {
    let json = TokamakJSON.object(["year": .number(2027), "on": .bool(true), "half": .number(0.5)])
    let value = json.value as! [String: Any]
    precondition(value["year"] as? Int == 2027 && value["on"] as? Bool == true)
    precondition(value["half"] as? Int == nil && value["half"] as? Double == 0.5)
    precondition(TokamakJSON(json.value) == json)
  }

  /// The plugin's events reach the Worker as its registration lists them,
  /// and replies decode as the emitter asks.
  static func testEvents() {
    let app = TokamakApp()
    let plugin = TokamakExamplePlugin(host: TokamakHost(app: app, plugin: "example"))
    app.plugins["example"] = plugin
    let region = Region(id: "home", radius: 50, onEntry: nil, tags: [], counts: [:], data: nil)
    let entered = String(decoding: try! plugin.eventPayload("entered", region), as: UTF8.self)
    precondition(entered.contains(#""id":"home""#) && !entered.contains("onEntry"), entered)
    let chose = try! plugin.eventPayload("chose", Precision.approximate)
    precondition(chose == Data(#""approximate""#.utf8), String(decoding: chose, as: UTF8.self))
    for (name, event, message) in [
      ("entered", "home" as Any, "event entered takes Region, not String"),
      ("left", region, "apple/Exports.swift lists no event left"),
    ] {
      do {
        _ = try plugin.eventPayload(name, event)
        preconditionFailure("encoded \(name)")
      } catch {
        precondition(error.name == "TypeError" && error.message == message, error.message)
      }
    }

    /// The listeners' `reply` to the event `name` as a `Reply`, or the failure's message.
    func answer<Reply>(_ reply: String, as type: Reply.Type, to name: String = "asked") -> String {
      app.reply = Data(reply.utf8)
      var answer = "no completion"
      plugin.host.emit(name, event: region, reply: type, timeout: 1) { result in
        answer =
          switch result {
          case .success(let reply): reply.map { "\($0)" } ?? "nil"
          case .failure(let error): (error as? TokamakPluginError)?.message ?? "\(error)"
          }
      }
      return answer
    }
    precondition(answer(#""precise""#, as: Precision.self) == "precise")
    precondition(app.emitted.last == "example.apple.asked")
    precondition(
      answer("", as: Precision.self) == "nil" && answer("null", as: Precision.self) == "nil")
    precondition(answer(#""vague""#, as: Precision.self).hasPrefix("reply "))
    let mismatch = answer(#""precise""#, as: String.self)
    precondition(mismatch == "event asked replies with Precision, not String", mismatch)
    let unlisted = answer(#""precise""#, as: Precision.self, to: "entered")
    precondition(unlisted == "apple/Exports.swift lists no reply to event entered", unlisted)
    // An event without a reply ignores what its listeners return.
    var delivered = false
    plugin.host.emit("entered", event: region, timeout: 1) { delivered = (try? $0.get()) != nil }
    precondition(delivered)
  }

  static func wait(until flag: Flag) {
    let deadline = Date().addingTimeInterval(2)
    while !flag.isSet && Date() < deadline { drain() }
    precondition(flag.isSet)
  }

  static func drain() {
    RunLoop.main.run(until: Date().addingTimeInterval(0.001))
  }
}

final class Flag: @unchecked Sendable {
  private let lock = NSLock()
  private var value = false

  var isSet: Bool { lock.withLock { value } }

  func set() { lock.withLock { value = true } }
}

/// The replies the Worker receives, by request.
final class Replies: @unchecked Sendable {
  private let lock = NSLock()
  private var results: [Int: [TokamakPluginResult]] = [:]

  func receive(_ id: Int, _ result: TokamakPluginResult) {
    lock.withLock { results[id, default: []].append(result) }
  }

  /// The first `count` replies to `id`, running the main queue until they arrive.
  func wait(for id: Int, count: Int = 1) -> [TokamakPluginResult] {
    let deadline = Date().addingTimeInterval(2)
    while lock.withLock({ results[id, default: []].count }) < count && Date() < deadline {
      DispatchTests.drain()
    }
    let received = lock.withLock { results[id, default: []] }
    precondition(received.count >= count, "request \(id) has \(received.count) replies")
    return received
  }
}

// The request dispatcher does not use the app's runtime or background-event facilities.
final class TokamakApp {
  weak var webView: WKWebView?
  let resources = TokamakResources()
  var isInForeground = true
  var plugins: [String: any TokamakPlugin] = [:]
  /// The events emitted, by name, which the Worker's listeners answer with `reply`.
  var emitted: [String] = []
  var reply = Data()
  func emit(
    _ name: String, payload: Data, timeout: TimeInterval,
    completion: @escaping (Result<Data, Error>) -> Void
  ) {
    emitted.append(name)
    completion(.success(reply))
  }
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
