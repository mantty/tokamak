import Foundation
import LocalAuthentication
import Security

#if os(iOS)
  import UIKit
#endif

final class TokamakSecureStoragePlugin: TokamakPlugin {
  let id = "secure-storage"

  private let service = "tokamak.secure-storage"
  /// Keychain calls block while the system authentication prompt is shown.
  private let queue = DispatchQueue(label: "tokamak.secure-storage")
  private let host: TokamakHost

  init(host: TokamakHost) {
    self.host = host
  }

  private static let teamSigningRequired = TokamakPluginError.notSupported(
    "Secure storage requires a team-signed build"
  )

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
    let prompting =
      isUnlocked ? Result { () throws(TokamakPluginError) in try host.requireUI() } : nil
    queue.async {
      reply(
        Result { () throws(TokamakPluginError) in
          try self.perform(method, arguments, prompting: prompting)
        })
    }
  }

  /// Whether the device's protected data is available, which it is not while
  /// an iPhone is locked. Read it on the main thread.
  private var isUnlocked: Bool {
    #if os(iOS)
      UIApplication.shared.isProtectedDataAvailable
    #else
      true
    #endif
  }

  /// `prompting` is whether a read or update that needs authentication may
  /// show the system prompt, decided on the main thread as the call arrives:
  /// nil while the device is locked, where either fails without UI anyway.
  private func perform(
    _ method: String,
    _ arguments: [String: Any],
    prompting: Result<Void, TokamakPluginError>?
  ) throws(TokamakPluginError) -> Any? {
    #if os(macOS)
      // Unentitled reads report errSecItemNotFound rather than a missing entitlement.
      guard Self.hasApplicationIdentifier else { throw Self.teamSigningRequired }
    #endif
    switch method {
    case "set":
      try set(
        requiredString(arguments, "name"),
        value: requiredString(arguments, "value"),
        readable: requiredString(arguments, "readable"),
        thisDeviceOnly: requiredBool(arguments, "thisDeviceOnly"),
        authentication: optionalString(arguments, "authentication"),
        prompt: optionalString(arguments, "prompt"),
        prompting: prompting
      )
      return nil
    case "get":
      return try get(
        requiredString(arguments, "name"),
        prompt: optionalString(arguments, "prompt"),
        prompting: prompting
      )
    case "delete":
      try deleteItems(query(requiredString(arguments, "name")))
      return nil
    case "clear":
      try deleteItems(serviceQuery())
      return nil
    default:
      throw .notSupported("\(id).\(method) is not supported")
    }
  }

  /// Updates an existing value in place or adds a new one, so a failure
  /// leaves the previous value. Checks without UI first, so only replacing a
  /// value saved with authentication needs the system prompt.
  private func set(
    _ name: String,
    value: String,
    readable: String,
    thisDeviceOnly: Bool,
    authentication: String?,
    prompt: String?,
    prompting: Result<Void, TokamakPluginError>?
  ) throws(TokamakPluginError) {
    let accessibility = try accessibility(readable, thisDeviceOnly: thisDeviceOnly)
    // Access control even without authentication, so an update removes any the existing value had.
    let stored: [CFString: Any] = [
      kSecValueData: Data(value.utf8),
      kSecAttrAccessControl: try accessControl(accessibility, authentication),
    ]
    var existing = query(name)
    existing[kSecUseAuthenticationContext] = Self.contextWithoutUI()
    let status = SecItemCopyMatching(existing as CFDictionary, nil)
    if status == errSecItemNotFound {
      try check(SecItemAdd(query(name).merging(stored) { _, new in new } as CFDictionary, nil))
      return
    }
    if status == errSecInteractionNotAllowed, let prompting {
      try prompting.get()
      existing[kSecUseAuthenticationContext] = Self.context(prompt: prompt)
    } else {
      try check(status)
    }
    try check(SecItemUpdate(existing as CFDictionary, stored as CFDictionary))
  }

  /// Reads without UI first, so only a value saved with authentication needs
  /// the app able to show the system prompt.
  private func get(
    _ name: String,
    prompt: String?,
    prompting: Result<Void, TokamakPluginError>?
  ) throws(TokamakPluginError) -> String? {
    var item = query(name)
    item[kSecReturnData] = true
    item[kSecUseAuthenticationContext] = Self.contextWithoutUI()
    var data: CFTypeRef?
    var status = SecItemCopyMatching(item as CFDictionary, &data)
    if status == errSecItemNotFound {
      return nil
    }
    if status == errSecInteractionNotAllowed, let prompting {
      try prompting.get()
      item[kSecUseAuthenticationContext] = Self.context(prompt: prompt)
      status = SecItemCopyMatching(item as CFDictionary, &data)
      // The first read found the value, so it exists but the system can no longer decrypt it.
      if status == errSecItemNotFound {
        throw TokamakPluginError(
          name: "NotReadableError",
          message: "The stored value can no longer be read"
        )
      }
    }
    try check(status)
    guard let data = data as? Data else {
      throw TokamakPluginError(name: "NotReadableError", message: "The stored value is not data")
    }
    return String(decoding: data, as: UTF8.self)
  }

  private func deleteItems(_ query: [CFString: Any]) throws(TokamakPluginError) {
    let status = SecItemDelete(query as CFDictionary)
    if status != errSecItemNotFound {
      try check(status)
    }
  }

  /// A context in which reads that need authentication fail with
  /// `errSecInteractionNotAllowed` rather than show the system prompt.
  private static func contextWithoutUI() -> LAContext {
    let context = LAContext()
    context.interactionNotAllowed = true
    return context
  }

  /// A context whose system prompt shows `prompt`, or nil for the default prompt.
  private static func context(prompt: String?) -> LAContext? {
    guard let prompt, !prompt.isEmpty else { return nil }
    let context = LAContext()
    context.localizedReason = prompt
    return context
  }

  /// All of the plugin's items. The data protection keychain is the only
  /// keychain on iOS; macOS needs a team-signed build to use it.
  private func serviceQuery() -> [CFString: Any] {
    [
      kSecClass: kSecClassGenericPassword,
      kSecAttrService: service,
      kSecUseDataProtectionKeychain: true,
    ]
  }

  private func query(_ name: String) -> [CFString: Any] {
    serviceQuery().merging([kSecAttrAccount: name]) { _, name in name }
  }

  /// `ThisDeviceOnly` items restore only to the device they were stored on.
  private func accessibility(
    _ readable: String,
    thisDeviceOnly: Bool
  ) throws(TokamakPluginError) -> CFString {
    switch (readable, thisDeviceOnly) {
    case ("whenUnlocked", true): kSecAttrAccessibleWhenUnlockedThisDeviceOnly
    case ("whenUnlocked", false): kSecAttrAccessibleWhenUnlocked
    case ("afterFirstUnlock", true): kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
    case ("afterFirstUnlock", false): kSecAttrAccessibleAfterFirstUnlock
    default: throw unsupportedArgument("readable", readable)
    }
  }

  private func accessControl(
    _ accessibility: CFString,
    _ authentication: String?
  ) throws(TokamakPluginError) -> SecAccessControl {
    let (flags, policy): (SecAccessControlCreateFlags, LAPolicy?) =
      switch authentication {
      case nil: ([], nil)
      case "biometricsOrPasscode": (.userPresence, .deviceOwnerAuthentication)
      case "biometrics": (.biometryAny, .deviceOwnerAuthenticationWithBiometrics)
      case "currentBiometrics": (.biometryCurrentSet, .deviceOwnerAuthenticationWithBiometrics)
      case let authentication?: throw unsupportedArgument("authentication", authentication)
      }
    if let policy {
      try requireSetUp(policy)
    }
    guard let control = SecAccessControlCreateWithFlags(nil, accessibility, flags, nil) else {
      throw TokamakPluginError(name: "OperationError", message: "Access control is unavailable")
    }
    return control
  }

  private func requireSetUp(_ policy: LAPolicy) throws(TokamakPluginError) {
    var error: NSError?
    if LAContext().canEvaluatePolicy(policy, error: &error) {
      return
    }
    let message = error?.localizedDescription ?? "Authentication is unavailable"
    if (error as? LAError)?.code == .biometryNotAvailable {
      throw .notSupported(message)
    }
    throw TokamakPluginError(name: "InvalidStateError", message: message)
  }

  private func check(_ status: OSStatus) throws(TokamakPluginError) {
    guard status != errSecSuccess else { return }
    let message = SecCopyErrorMessageString(status, nil) as String? ?? "Keychain error \(status)"
    switch status {
    case errSecMissingEntitlement:
      throw Self.teamSigningRequired
    case errSecUserCanceled, errSecAuthFailed, errSecInteractionNotAllowed:
      throw TokamakPluginError(name: "NotAllowedError", message: message)
    default:
      throw TokamakPluginError(name: "OperationError", message: message)
    }
  }

  private func requiredString(
    _ arguments: [String: Any],
    _ key: String
  ) throws(TokamakPluginError) -> String {
    guard let value = arguments[key] as? String else {
      throw TokamakPluginError(name: "TypeError", message: "\(key) must be a string")
    }
    return value
  }

  private func requiredBool(
    _ arguments: [String: Any],
    _ key: String
  ) throws(TokamakPluginError) -> Bool {
    guard let value = arguments[key] as? Bool else {
      throw TokamakPluginError(name: "TypeError", message: "\(key) must be a boolean")
    }
    return value
  }

  /// Missing and null arguments are nil; other non-string values are rejected.
  private func optionalString(
    _ arguments: [String: Any],
    _ key: String
  ) throws(TokamakPluginError) -> String? {
    guard let value = arguments[key], !(value is NSNull) else { return nil }
    return try requiredString(arguments, key)
  }

  private func unsupportedArgument(_ key: String, _ value: String) -> TokamakPluginError {
    TokamakPluginError(name: "TypeError", message: "\(key) '\(value)' is not supported")
  }
}
