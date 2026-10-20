import Foundation

final class {{class}}: TokamakPlugin {
  let id = "{{id}}"
  init(host: TokamakHost) {}

  func call(method: String, arguments: Any, reply: @escaping TokamakPluginReply) -> (() -> Void)? {
    switch method {
    case "echo":
      reply(.success(arguments))
    default:
      reply(.failure(.notSupported("\(id).\(method) is not supported")))
    }
    return nil
  }
}
