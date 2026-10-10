import Foundation
import LocalAuthentication
import Security

#if os(iOS)
  import UIKit
#endif

final class TokamakSecureStoragePlugin: TokamakPlugin {
  let id = "secure-storage"

  /// Holds each value under a random ID.
  private let values = "tokamak.secure-storage"
  /// Holds each name's record: the ID of its value. Records have no access
  /// control, so `keys()` needs no authentication; on a device, reading even the
  /// attributes of an item saved with access control does.
  private let names = "tokamak.secure-storage.names"
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

  /// `prompting` is whether a read that needs authentication may show the
  /// system prompt, decided on the main thread as the call arrives: nil while
  /// the device is locked, where such a read fails without UI anyway.
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
        authentication: optionalString(arguments, "authentication")
      )
      return nil
    case "get":
      return try get(
        requiredString(arguments, "name"),
        prompt: optionalString(arguments, "prompt"),
        prompting: prompting
      )
    case "delete":
      try delete(requiredString(arguments, "name"))
      return nil
    case "keys":
      return try keys()
    case "clear":
      try deleteItems(serviceQuery(values))
      try deleteItems(serviceQuery(names))
      return nil
    default:
      throw .notSupported("\(id).\(method) is not supported")
    }
  }

  /// Stores the value under a new ID before pointing the name's record at it, so
  /// a failed write leaves the previous value readable.
  private func set(
    _ name: String,
    value: String,
    readable: String,
    thisDeviceOnly: Bool,
    authentication: String?
  ) throws(TokamakPluginError) {
    let accessibility = try accessibility(readable, thisDeviceOnly: thisDeviceOnly)
    let id = UUID().uuidString
    var item = query(values, id)
    item[kSecValueData] = Data(value.utf8)
    if let authentication {
      item[kSecAttrAccessControl] = try accessControl(accessibility, authentication)
    } else {
      item[kSecAttrAccessible] = accessibility
    }
    let previous = try valueID(name)
    try check(SecItemAdd(item as CFDictionary, nil))
    do throws(TokamakPluginError) {
      try point(name, at: id, thisDeviceOnly: thisDeviceOnly)
    } catch {
      SecItemDelete(query(values, id) as CFDictionary)
      throw error
    }
    if let previous {
      try deleteItems(query(values, previous))
    }
  }

  /// Points `name`'s record at `id`. Records are readable after the first
  /// unlock, so `keys()` works while the device is locked.
  private func point(
    _ name: String,
    at id: String,
    thisDeviceOnly: Bool
  ) throws(TokamakPluginError) {
    let record: [CFString: Any] = [
      kSecValueData: Data(id.utf8),
      kSecAttrAccessible: thisDeviceOnly
        ? kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly : kSecAttrAccessibleAfterFirstUnlock,
    ]
    var status = SecItemUpdate(query(names, name) as CFDictionary, record as CFDictionary)
    if status == errSecItemNotFound {
      status = SecItemAdd(query(names, name).merging(record) { _, new in new } as CFDictionary, nil)
    }
    try check(status)
  }

  /// The ID of `name`'s value, or nil when `name` has no value.
  private func valueID(_ name: String) throws(TokamakPluginError) -> String? {
    var read = query(names, name)
    read[kSecReturnData] = true
    var data: CFTypeRef?
    let status = SecItemCopyMatching(read as CFDictionary, &data)
    if status == errSecItemNotFound {
      return nil
    }
    try check(status)
    return (data as? Data).map { String(decoding: $0, as: UTF8.self) }
  }

  /// Reads without UI first, so only a value saved with authentication needs
  /// the app able to show the system prompt.
  private func get(
    _ name: String,
    prompt: String?,
    prompting: Result<Void, TokamakPluginError>?
  ) throws(TokamakPluginError) -> String? {
    guard let id = try valueID(name) else { return nil }
    var item = query(values, id)
    item[kSecReturnData] = true
    item[kSecUseAuthenticationContext] = Self.contextWithoutUI()
    var data: CFTypeRef?
    var status = SecItemCopyMatching(item as CFDictionary, &data)
    if status == errSecInteractionNotAllowed, let prompting {
      try prompting.get()
      item[kSecUseAuthenticationContext] = prompt.flatMap { prompt -> LAContext? in
        guard !prompt.isEmpty else { return nil }
        let context = LAContext()
        context.localizedReason = prompt
        return context
      }
      status = SecItemCopyMatching(item as CFDictionary, &data)
    }
    if status == errSecItemNotFound {
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

  /// Deletes the value before its record, so a failure leaves the name listed.
  private func delete(_ name: String) throws(TokamakPluginError) {
    guard let id = try valueID(name) else { return }
    try deleteItems(query(values, id))
    try deleteItems(query(names, name))
  }

  private func keys() throws(TokamakPluginError) -> [String] {
    var records = serviceQuery(names)
    records[kSecReturnAttributes] = true
    records[kSecMatchLimit] = kSecMatchLimitAll
    var attributes: CFTypeRef?
    let status = SecItemCopyMatching(records as CFDictionary, &attributes)
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

  /// A context in which reads that need authentication fail with
  /// `errSecInteractionNotAllowed` rather than show the system prompt.
  private static func contextWithoutUI() -> LAContext {
    let context = LAContext()
    context.interactionNotAllowed = true
    return context
  }

  /// All of the plugin's items in `service`. The data protection keychain is
  /// the only keychain on iOS; macOS needs a team-signed build to use it.
  private func serviceQuery(_ service: String) -> [CFString: Any] {
    [
      kSecClass: kSecClassGenericPassword,
      kSecAttrService: service,
      kSecUseDataProtectionKeychain: true,
    ]
  }

  private func query(_ service: String, _ account: String) -> [CFString: Any] {
    serviceQuery(service).merging([kSecAttrAccount: account]) { _, account in account }
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
