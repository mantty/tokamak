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
      guard Self.hasApplicationIdentifier else {
        throw .notSupported("Secure storage requires a team-signed build")
      }
    #endif
    let group = try optionalString(arguments, "keychainGroup")
    switch method {
    case "create":
      try check(add(requiredString(arguments, "name"), group: group, stored(arguments)))
      return nil
    case "replace":
      try check(
        update(
          requiredString(arguments, "name"),
          group: group,
          stored(arguments),
          prompt: optionalString(arguments, "prompt"),
          prompting: prompting
        ))
      return nil
    case "set":
      let name = try requiredString(arguments, "name")
      let stored = try stored(arguments)
      let status = try update(
        name,
        group: group,
        stored,
        prompt: optionalString(arguments, "prompt"),
        prompting: prompting
      )
      try check(status == errSecItemNotFound ? add(name, group: group, stored) : status)
      return nil
    case "get":
      return try get(
        requiredString(arguments, "name"),
        group: group,
        prompt: optionalString(arguments, "prompt"),
        prompting: prompting
      )
    case "delete":
      try deleteItems(query(requiredString(arguments, "name"), group: group))
      return nil
    case "clear":
      try deleteItems(serviceQuery(group: group))
      return nil
    default:
      throw .notSupported("\(id).\(method) is not supported")
    }
  }

  /// The value and its protection, as keychain attributes. Access control even
  /// without authentication, so an update removes any the existing value had.
  private func stored(_ arguments: [String: Any]) throws(TokamakPluginError) -> [CFString: Any] {
    let accessibility = try accessibility(
      requiredString(arguments, "readable"),
      thisDeviceOnly: requiredBool(arguments, "thisDeviceOnly")
    )
    return [
      kSecValueData: Data(try requiredString(arguments, "value").utf8),
      kSecAttrAccessControl: try accessControl(
        accessibility, optionalString(arguments, "authentication")),
    ]
  }

  /// Adds `name`'s value in one keychain call: errSecDuplicateItem when it has one.
  private func add(_ name: String, group: String?, _ stored: [CFString: Any]) -> OSStatus {
    SecItemAdd(query(name, group: group).merging(stored) { _, new in new } as CFDictionary, nil)
  }

  /// Updates `name`'s value in place, so a failure leaves the previous value:
  /// errSecItemNotFound when it has none. Tries without UI
  /// first, so only a value saved with authentication needs the system prompt.
  private func update(
    _ name: String,
    group: String?,
    _ stored: [CFString: Any],
    prompt: String?,
    prompting: Result<Void, TokamakPluginError>?
  ) throws(TokamakPluginError) -> OSStatus {
    var item = query(name, group: group)
    item[kSecUseAuthenticationContext] = Self.contextWithoutUI()
    let status = SecItemUpdate(item as CFDictionary, stored as CFDictionary)
    guard status == errSecInteractionNotAllowed, let prompting else { return status }
    try prompting.get()
    item[kSecUseAuthenticationContext] = Self.context(prompt: prompt)
    return SecItemUpdate(item as CFDictionary, stored as CFDictionary)
  }

  /// Reads without UI first, so only a value saved with authentication needs
  /// the app able to show the system prompt.
  private func get(
    _ name: String,
    group: String?,
    prompt: String?,
    prompting: Result<Void, TokamakPluginError>?
  ) throws(TokamakPluginError) -> String? {
    var item = query(name, group: group)
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

  /// A context in which keychain calls that need authentication fail with
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

  /// The plugin's items in keychain access group `group`, or in every group
  /// of the app when nil, where an add uses the app's first group. The data
  /// protection keychain is the only keychain on iOS; macOS needs a
  /// team-signed build to use it.
  private func serviceQuery(group: String?) -> [CFString: Any] {
    var query: [CFString: Any] = [
      kSecClass: kSecClassGenericPassword,
      kSecAttrService: service,
      kSecUseDataProtectionKeychain: true,
    ]
    query[kSecAttrAccessGroup] = group
    return query
  }

  private func query(_ name: String, group: String?) -> [CFString: Any] {
    serviceQuery(group: group).merging([kSecAttrAccount: name]) { _, name in name }
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
    case errSecDuplicateItem:
      throw TokamakPluginError(name: "DuplicateItemError", message: message)
    case errSecItemNotFound:
      throw TokamakPluginError(name: "ItemNotFoundError", message: message)
    case errSecMissingEntitlement:
      throw TokamakPluginError(name: "MissingEntitlementError", message: message)
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
