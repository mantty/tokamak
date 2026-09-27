import Foundation
import LocalAuthentication
import Security

final class TokamakSecureStoragePlugin: TokamakPlugin {
  let id = "secure-storage"

  private let service = "tokamak.secure-storage"
  /// Keychain calls block while the system authentication prompt is shown.
  private let queue = DispatchQueue(label: "tokamak.secure-storage")

  #if os(macOS)
    /// macOS grants the data protection keychain only to apps whose provisioning
    /// profile provides an application identifier.
    private static let hasApplicationIdentifier: Bool = {
      guard let task = SecTaskCreateFromSelf(nil) else { return false }
      let entitlement = "com.apple.application-identifier" as CFString
      return SecTaskCopyValueForEntitlement(task, entitlement, nil) != nil
    }()
  #endif

  func call(
    method: String,
    arguments: Any,
    reply: @escaping TokamakPluginReply
  ) {
    let arguments = arguments as? [String: Any] ?? [:]
    queue.async {
      reply(Result { () throws(TokamakPluginError) in try self.perform(method, arguments) })
    }
  }

  private func perform(
    _ method: String,
    _ arguments: [String: Any]
  ) throws(TokamakPluginError) -> Any? {
    #if os(macOS)
      // Unentitled reads report errSecItemNotFound rather than a missing entitlement.
      guard Self.hasApplicationIdentifier else {
        throw .notSupported("Secure storage requires a team-signed build")
      }
    #endif
    switch method {
    case "set":
      try set(
        string(arguments, "name"),
        value: string(arguments, "value"),
        readable: string(arguments, "readable"),
        authentication: arguments["authentication"] as? String
      )
      return nil
    case "get":
      return try get(string(arguments, "name"), prompt: arguments["prompt"] as? String)
    case "delete":
      try delete(string(arguments, "name"))
      return nil
    default:
      throw .notSupported("\(id).\(method) is not supported")
    }
  }

  private func set(
    _ name: String,
    value: String,
    readable: String,
    authentication: String?
  ) throws(TokamakPluginError) {
    var item = query(name)
    item[kSecValueData] = Data(value.utf8)
    let accessibility = try accessibility(readable)
    if let authentication {
      item[kSecAttrAccessControl] = try accessControl(accessibility, authentication)
    } else {
      item[kSecAttrAccessible] = accessibility
    }
    try delete(name)
    try check(SecItemAdd(item as CFDictionary, nil))
  }

  private func get(_ name: String, prompt: String?) throws(TokamakPluginError) -> String? {
    var item = query(name)
    item[kSecReturnData] = true
    if let prompt, !prompt.isEmpty {
      let context = LAContext()
      context.localizedReason = prompt
      item[kSecUseAuthenticationContext] = context
    }
    var data: CFTypeRef?
    let status = SecItemCopyMatching(item as CFDictionary, &data)
    if status == errSecItemNotFound {
      guard try exists(name) else { return nil }
      throw TokamakPluginError(
        name: "NotReadableError",
        message: "The stored value can no longer be read"
      )
    }
    try check(status)
    guard let data = data as? Data else {
      throw TokamakPluginError(name: "NotReadableError", message: "The stored value is not data")
    }
    return String(decoding: data, as: UTF8.self)
  }

  private func delete(_ name: String) throws(TokamakPluginError) {
    let status = SecItemDelete(query(name) as CFDictionary)
    if status != errSecItemNotFound {
      try check(status)
    }
  }

  /// Reads attributes only, which needs no authentication.
  private func exists(_ name: String) throws(TokamakPluginError) -> Bool {
    let context = LAContext()
    context.interactionNotAllowed = true
    var item = query(name)
    item[kSecReturnAttributes] = true
    item[kSecUseAuthenticationContext] = context
    let status = SecItemCopyMatching(item as CFDictionary, nil)
    if status == errSecItemNotFound {
      return false
    }
    try check(status)
    return true
  }

  /// The data protection keychain is the only keychain on iOS; macOS needs a
  /// team-signed build to use it.
  private func query(_ name: String) -> [CFString: Any] {
    [
      kSecClass: kSecClassGenericPassword,
      kSecAttrService: service,
      kSecAttrAccount: name,
      kSecUseDataProtectionKeychain: true,
    ]
  }

  private func accessibility(_ readable: String) throws(TokamakPluginError) -> CFString {
    switch readable {
    case "whenUnlocked": kSecAttrAccessibleWhenUnlockedThisDeviceOnly
    case "afterFirstUnlock": kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
    default: throw invalidArgument("readable", readable)
    }
  }

  private func accessControl(
    _ accessibility: CFString,
    _ authentication: String
  ) throws(TokamakPluginError) -> SecAccessControl {
    let (flags, policy): (SecAccessControlCreateFlags, LAPolicy) =
      switch authentication {
      case "biometricsOrPasscode": (.userPresence, .deviceOwnerAuthentication)
      case "biometrics": (.biometryAny, .deviceOwnerAuthenticationWithBiometrics)
      case "currentBiometrics": (.biometryCurrentSet, .deviceOwnerAuthenticationWithBiometrics)
      default: throw invalidArgument("authentication", authentication)
      }
    var error: NSError?
    guard LAContext().canEvaluatePolicy(policy, error: &error) else {
      throw TokamakPluginError(
        name: "InvalidStateError",
        message: error?.localizedDescription ?? "Authentication is not set up"
      )
    }
    guard let control = SecAccessControlCreateWithFlags(nil, accessibility, flags, nil) else {
      throw TokamakPluginError(name: "OperationError", message: "Access control is unavailable")
    }
    return control
  }

  private func check(_ status: OSStatus) throws(TokamakPluginError) {
    let message = SecCopyErrorMessageString(status, nil) as String? ?? "Keychain error \(status)"
    switch status {
    case errSecSuccess:
      return
    case errSecMissingEntitlement:
      throw .notSupported("Secure storage requires a team-signed build")
    case errSecUserCanceled, errSecAuthFailed, errSecInteractionNotAllowed:
      throw TokamakPluginError(name: "NotAllowedError", message: message)
    default:
      throw TokamakPluginError(name: "OperationError", message: message)
    }
  }

  private func string(
    _ arguments: [String: Any],
    _ key: String
  ) throws(TokamakPluginError) -> String {
    guard let value = arguments[key] as? String else {
      throw TokamakPluginError(name: "TypeError", message: "\(key) must be a string")
    }
    return value
  }

  private func invalidArgument(_ key: String, _ value: String) -> TokamakPluginError {
    TokamakPluginError(name: "TypeError", message: "\(key) '\(value)' is not supported")
  }
}
