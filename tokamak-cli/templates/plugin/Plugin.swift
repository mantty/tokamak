import Foundation

final class {{class}}: TokamakPlugin {
  /// A message the native implementation echoed.
  struct Echo: Codable {
    let message: String
  }

  let id = "{{id}}"
  let host: TokamakHost

  init(host: TokamakHost) {
    self.host = host
  }

  /// Returns `message`, which the Worker's `onEchoed` listeners also receive.
  func echo(_ message: String) -> String {
    host.emit("echoed", event: Echo(message: message), timeout: 10) { _ in }
    return message
  }
}
