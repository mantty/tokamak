import Foundation
import LocalAuthentication
import TokamakPlugin

extension TokamakLocalAuthenticationPlugin {
  /// The authentication the device owner performs.
  enum Authentication: String, Codable {
    case biometricsOrPasscode
    case biometrics

    var policy: LAPolicy {
      switch self {
      case .biometricsOrPasscode: .deviceOwnerAuthentication
      case .biometrics: .deviceOwnerAuthenticationWithBiometrics
      }
    }
  }

  /// Whether the device can perform an authentication now.
  enum Status: String, Codable {
    case available
    case notEnrolled
    case unavailable
  }
}

public final class TokamakLocalAuthenticationPlugin: TokamakPlugin {
  public let id = "local-authentication"
  public let host: TokamakHost

  public init(host: TokamakHost) {
    self.host = host
  }

  func status(_ authentication: Authentication) -> Status {
    var error: NSError?
    if LAContext().canEvaluatePolicy(authentication.policy, error: &error) {
      return .available
    }
    switch (error as? LAError)?.code {
    case .biometryNotEnrolled, .passcodeNotSet: return .notEnrolled
    default: return .unavailable
    }
  }

  /// Shows the system prompt, which needs the app able to show UI, unless the
  /// authentication fails without one.
  @MainActor
  func authenticate(_ authentication: Authentication, prompt: String) async throws {
    guard !prompt.isEmpty else {
      throw TokamakPluginError(name: "TypeError", message: "prompt must be a non-empty string")
    }
    let context = LAContext()
    var error: NSError?
    guard context.canEvaluatePolicy(authentication.policy, error: &error) else {
      throw Self.failure(error)
    }
    try host.requireUI()
    if authentication == .biometrics {
      // Hides the passcode fallback button, which this policy cannot use.
      context.localizedFallbackTitle = ""
    }
    do {
      try await withTaskCancellationHandler {
        _ = try await context.evaluatePolicy(authentication.policy, localizedReason: prompt)
      } onCancel: {
        context.invalidate()
      }
    } catch {
      throw Self.failure(error)
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
}
