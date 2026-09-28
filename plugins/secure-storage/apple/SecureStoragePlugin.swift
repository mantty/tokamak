import Foundation
import LocalAuthentication
import Security

final class TokamakSecureStoragePlugin: TokamakPlugin {
  let id = "secure-storage"

  private let service = "tokamak.secure-storage"
  /// Keychain calls block while the system authentication prompt is shown.
  private let queue = DispatchQueue(label: "tokamak.secure-storage")

  init(host: TokamakHost) {}

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
      guard Self.hasApplicationIdentifier else { throw Self.teamSigningRequired }
    #endif
    switch method {
    case "set":
      try set(
        requiredString(arguments, "name"),
        value: requiredString(arguments, "value"),
        accessibility: accessibility(
          requiredString(arguments, "readable"),
          thisDeviceOnly: requiredBool(arguments, "thisDeviceOnly")
        ),
        authentication: optionalString(arguments, "authentication")
      )
      return nil
    case "get":
      return try get(
        requiredString(arguments, "name"),
        prompt: optionalString(arguments, "prompt")
      )
    case "delete":
      try deleteItems(query(requiredString(arguments, "name")))
      return nil
    case "keys":
      return try keys()
    case "clear":
      try deleteItems(serviceQuery())
      return nil
    default:
      throw .notSupported("\(id).\(method) is not supported")
    }
  }

  /// Replaces an existing value only once the new item can be added, so a
  /// failed write keeps the old value.
  private func set(
    _ name: String,
    value: String,
    accessibility: CFString,
    authentication: String?
  ) throws(TokamakPluginError) {
    var item = query(name)
    item[kSecValueData] = Data(value.utf8)
    if let authentication {
      item[kSecAttrAccessControl] = try accessControl(accessibility, authentication)
    } else {
      item[kSecAttrAccessible] = accessibility
    }
    var status = SecItemAdd(item as CFDictionary, nil)
    if status == errSecDuplicateItem {
      try deleteItems(query(name))
      status = SecItemAdd(item as CFDictionary, nil)
    }
    try check(status)
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

  /// Reads attributes only, which needs no authentication.
  private func keys() throws(TokamakPluginError) -> [String] {
    var items = serviceQuery()
    items[kSecMatchLimit] = kSecMatchLimitAll
    items[kSecReturnAttributes] = true
    items[kSecUseAuthenticationContext] = nonInteractiveContext()
    var attributes: CFTypeRef?
    let status = SecItemCopyMatching(items as CFDictionary, &attributes)
    if status == errSecItemNotFound {
      return []
    }
    try check(status)
    let names = (attributes as? [[String: Any]] ?? []).compactMap {
      $0[kSecAttrAccount as String] as? String
    }
    return names.sorted()
  }

  private func deleteItems(_ query: [CFString: Any]) throws(TokamakPluginError) {
    let status = SecItemDelete(query as CFDictionary)
    if status != errSecItemNotFound {
      try check(status)
    }
  }

  /// Reads attributes only, which needs no authentication.
  private func exists(_ name: String) throws(TokamakPluginError) -> Bool {
    var item = query(name)
    item[kSecReturnAttributes] = true
    item[kSecUseAuthenticationContext] = nonInteractiveContext()
    let status = SecItemCopyMatching(item as CFDictionary, nil)
    if status == errSecItemNotFound {
      return false
    }
    try check(status)
    return true
  }

  private func nonInteractiveContext() -> LAContext {
    let context = LAContext()
    context.interactionNotAllowed = true
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
    _ authentication: String
  ) throws(TokamakPluginError) -> SecAccessControl {
    let (flags, policy): (SecAccessControlCreateFlags, LAPolicy) =
      switch authentication {
      case "biometricsOrPasscode": (.userPresence, .deviceOwnerAuthentication)
      case "biometrics": (.biometryAny, .deviceOwnerAuthenticationWithBiometrics)
      case "currentBiometrics": (.biometryCurrentSet, .deviceOwnerAuthenticationWithBiometrics)
      default: throw unsupportedArgument("authentication", authentication)
      }
    try requireSetUp(policy)
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
