import Foundation

final class {{class}}: TokamakPlugin {
  let id = "{{id}}"
  private let queue = DispatchQueue(label: "tokamak.{{id}}")
  init(host: TokamakHost) {}
  func executor(for method: String) -> DispatchQueue { queue }

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
