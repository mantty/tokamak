import Foundation
import LocalAuthentication

final class TokamakLocalAuthenticationPlugin: TokamakPlugin {
  let id = "local-authentication"

  func call(
    method: String,
    arguments: Any,
    reply: @escaping TokamakPluginReply
  ) {
    let arguments = arguments as? [String: Any] ?? [:]
    do throws(TokamakPluginError) {
      switch method {
      case "status":
        reply(.success(status(try policy(arguments))))
      case "authenticate":
        try authenticate(policy(arguments), prompt: prompt(arguments), reply: reply)
      default:
        throw .notSupported("\(id).\(method) is not supported")
      }
    } catch {
      reply(.failure(error))
    }
  }

  private func status(_ policy: LAPolicy) -> String {
    var error: NSError?
    if LAContext().canEvaluatePolicy(policy, error: &error) {
      return "available"
    }
    switch (error as? LAError)?.code {
    case .biometryNotEnrolled, .passcodeNotSet:
      return "notEnrolled"
    default:
      return "unavailable"
    }
  }

  private func authenticate(
    _ policy: LAPolicy,
    prompt: String,
    reply: @escaping TokamakPluginReply
  ) {
    let context = LAContext()
    if policy == .deviceOwnerAuthenticationWithBiometrics {
      // Hides the passcode fallback button, which this policy cannot use.
      context.localizedFallbackTitle = ""
    }
    context.evaluatePolicy(policy, localizedReason: prompt) { success, error in
      // A deallocated context cancels its evaluation.
      withExtendedLifetime(context) {
        reply(success ? .success(nil) : .failure(Self.failure(error)))
      }
    }
  }

  private static func failure(_ error: Error?) -> TokamakPluginError {
    let message = error?.localizedDescription ?? "Authentication failed"
    switch (error as? LAError)?.code {
    case .biometryNotEnrolled, .passcodeNotSet:
      return TokamakPluginError(name: "InvalidStateError", message: message)
    case .biometryNotAvailable:
      return .notSupported(message)
    default:
      return TokamakPluginError(name: "NotAllowedError", message: message)
    }
  }

  private func policy(_ arguments: [String: Any]) throws(TokamakPluginError) -> LAPolicy {
    switch arguments["authentication"] as? String {
    case "biometricsOrPasscode":
      return .deviceOwnerAuthentication
    case "biometrics":
      return .deviceOwnerAuthenticationWithBiometrics
    default:
      throw TokamakPluginError(
        name: "TypeError",
        message: "authentication must be biometricsOrPasscode or biometrics"
      )
    }
  }

  private func prompt(_ arguments: [String: Any]) throws(TokamakPluginError) -> String {
    guard let prompt = arguments["prompt"] as? String, !prompt.isEmpty else {
      throw TokamakPluginError(name: "TypeError", message: "prompt must be a non-empty string")
    }
    return prompt
  }
}
