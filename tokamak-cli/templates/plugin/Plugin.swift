import Foundation

final class {{class}}: TokamakPlugin {
  let id = "{{id}}"
  private let host: TokamakHost
  private let queue = DispatchQueue(label: "tokamak.{{id}}")
  init(host: TokamakHost) { self.host = host }
  func executor(for method: String) -> DispatchQueue { queue }

  func call(method: String, arguments: Any, reply: @escaping TokamakPluginReply) -> (() -> Void)? {
    switch method {
    case "echo":
      reply(.success(arguments))
      DispatchQueue.main.async { [host] in
        host.emit("echoed", event: ["message": arguments], timeout: 10) { _ in }
      }
    default:
      reply(.failure(.notSupported("\(id).\(method) is not supported")))
    }
    return nil
  }
}
