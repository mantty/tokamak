import Foundation

struct TokamakPluginError: Error {
  let name: String
  let message: String
}

typealias TokamakPluginReply = (Result<Any?, TokamakPluginError>) -> Void

final class TokamakApp {}

final class TokamakHost {
  init(app: TokamakApp, plugin: String) {}
}

protocol TokamakPlugin: AnyObject {
  var id: String { get }
  init(host: TokamakHost)
  func call(method: String, arguments: Any, reply: @escaping TokamakPluginReply) -> (() -> Void)?
}

@main struct App { static func main() {} }
