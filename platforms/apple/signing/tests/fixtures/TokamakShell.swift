import Foundation

struct TokamakPluginError: Error {
  let name: String
  let message: String

  static func notSupported(_ message: String) -> Self {
    Self(name: "NotSupportedError", message: message)
  }
}

typealias TokamakPluginReply = (Result<Any?, TokamakPluginError>) -> Void

final class TokamakApp {}

final class TokamakHost {
  init(app: TokamakApp, plugin: String) {}

  func requireUI() throws(TokamakPluginError) {}
}

protocol TokamakPlugin: AnyObject {
  var id: String { get }
  init(host: TokamakHost)
  func call(method: String, arguments: Any, reply: @escaping TokamakPluginReply)
  func subscribe(
    method: String,
    arguments: Any,
    reply: @escaping TokamakPluginReply
  ) throws(TokamakPluginError) -> (() -> Void)
}

extension TokamakPlugin {
  func subscribe(
    method: String,
    arguments: Any,
    reply: @escaping TokamakPluginReply
  ) throws(TokamakPluginError) -> (() -> Void) {
    throw .notSupported("\(id).\(method) is not supported")
  }
}

@main struct App { static func main() {} }
