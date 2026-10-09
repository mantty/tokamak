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
}

protocol TokamakPlugin: AnyObject {
  var id: String { get }
  init(host: TokamakHost)
  func call(method: String, arguments: Any, reply: @escaping TokamakPluginReply)
  func subscribe(
    method: String,
    arguments: Any,
    reply: @escaping TokamakPluginReply
  ) -> (() -> Void)
}

extension TokamakPlugin {
  func subscribe(
    method: String,
    arguments: Any,
    reply: @escaping TokamakPluginReply
  ) -> (() -> Void) {
    reply(.failure(.notSupported("\(id).\(method) is not supported")))
    return {}
  }
}

@main struct App { static func main() {} }
