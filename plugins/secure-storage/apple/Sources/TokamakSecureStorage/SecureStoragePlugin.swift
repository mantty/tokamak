import Foundation
@preconcurrency import LocalAuthentication
import Security
import TokamakPlugin

#if os(iOS)
  import UIKit
#endif

extension TokamakSecureStoragePlugin {
  /// When a stored value can be read.
  enum Readable: String, Codable {
    case whenUnlocked
    case afterFirstUnlock
  }

  /// The authentication a read requires.
  enum Authentication: String, Codable {
    case biometricsOrPasscode
    case biometrics
    case currentBiometrics
  }

  /// How a value is stored.
  struct StoreOptions: Codable {
    let readable: Readable
    /// Reads need no authentication when nil.
    let authentication: Authentication?
    /// Whether backups restore the value only to this device.
    let thisDeviceOnly: Bool
    /// One of the app's `keychain-access-groups`: the first of them for a new value, and every one
    /// for an update, when nil.
    let keychainGroup: String?
    /// The reason the system prompt shows when replacing a value saved with authentication.
    let prompt: String?
  }

  struct ReadOptions: Codable {
    /// The reason the system prompt shows.
    let prompt: String?
    /// The keychain access group to read from; every one of the app's when nil.
    let keychainGroup: String?
  }

  struct GroupOptions: Codable {
    /// The keychain access group; every one of the app's when nil.
    let keychainGroup: String?
  }
}

public final class TokamakSecureStoragePlugin: TokamakPlugin {
  public let id = "secure-storage"
  public let host: TokamakHost

  private let service = "tokamak.secure-storage"
  /// Keychain calls block while the system authentication prompt is shown.
  private let queue = DispatchQueue(label: "tokamak.secure-storage")

  public init(host: TokamakHost) {
    self.host = host
  }

  public func executor(for method: String) -> DispatchQueue { queue }

  #if os(macOS)
    /// macOS grants the data protection keychain only to apps whose provisioning
    /// profile provides an application identifier.
    private static let hasApplicationIdentifier =
      tokamakEntitlement("com.apple.application-identifier") != nil
  #endif

  func create(_ name: String, _ value: String, _ options: StoreOptions) throws(TokamakPluginError) {
    try requireKeychain()
    try check(add(name, group: options.keychainGroup, stored(value, options)))
  }

  /// Replaces the value in place, asking for the authentication it was saved with.
  @MainActor
  func replace(_ name: String, _ value: String, _ options: StoreOptions) async throws {
    let prompting = self.prompting
    try await withAuthentication { authentication throws(TokamakPluginError) in
      try self.check(
        self.update(
          name, group: options.keychainGroup, self.stored(value, options), prompt: options.prompt,
          prompting: prompting, authentication: authentication))
    }
  }

  @MainActor
  func set(_ name: String, _ value: String, _ options: StoreOptions) async throws {
    let prompting = self.prompting
    try await withAuthentication { authentication throws(TokamakPluginError) in
      let stored = try self.stored(value, options)
      let status = try self.update(
        name, group: options.keychainGroup, stored, prompt: options.prompt, prompting: prompting,
        authentication: authentication)
      try self.check(
        status == errSecItemNotFound ? self.add(name, group: options.keychainGroup, stored) : status
      )
    }
  }

  /// Reads the value, asking for the authentication it was saved with.
  @MainActor
  func get(_ name: String, _ options: ReadOptions?) async throws -> String {
    let prompting = self.prompting
    return try await withAuthentication { authentication throws(TokamakPluginError) in
      try self.read(
        name, group: options?.keychainGroup, prompt: options?.prompt, prompting: prompting,
        authentication: authentication)
    }
  }

  func delete(_ name: String, _ options: GroupOptions?) throws(TokamakPluginError) {
    try requireKeychain()
    try check(SecItemDelete(query(name, group: options?.keychainGroup) as CFDictionary))
  }

  func clear(_ options: GroupOptions?) throws(TokamakPluginError) {
    try requireKeychain()
    try check(SecItemDelete(serviceQuery(group: options?.keychainGroup) as CFDictionary))
  }

  /// Whether a read or update that needs authentication may show the system
  /// prompt, decided on the main thread as the call arrives: nil while the
  /// device is locked, where either fails without UI anyway.
  @MainActor
  private var prompting: Result<Void, TokamakPluginError>? {
    isUnlocked ? Result { () throws(TokamakPluginError) in try host.requireUI() } : nil
  }

  /// Whether the device's protected data is available, which it is not while
  /// an iPhone is locked. Read it on the main thread.
  @MainActor
  private var isUnlocked: Bool {
    #if os(iOS)
      UIApplication.shared.isProtectedDataAvailable
    #else
      true
    #endif
  }

  /// Runs `work` on the plugin's queue, where keychain calls block, with an
  /// authentication context that cancelling the call invalidates.
  private func withAuthentication<T>(
    _ work: @escaping (LAContext) throws(TokamakPluginError) -> T
  ) async throws -> T {
    try requireKeychain()
    let authentication = LAContext()
    return try await withTaskCancellationHandler {
      try await onQueue { () throws(TokamakPluginError) in try work(authentication) }
    } onCancel: {
      authentication.invalidate()
    }
  }

  /// What `work` returns, run on the plugin's queue.
  private func onQueue<T>(_ work: @escaping () throws(TokamakPluginError) -> T) async throws -> T {
    try await withCheckedThrowingContinuation { continuation in
      queue.async { continuation.resume(with: Result(catching: work)) }
    }
  }

  private func requireKeychain() throws(TokamakPluginError) {
    #if os(macOS)
      // Unentitled reads report errSecItemNotFound rather than a missing entitlement.
      guard Self.hasApplicationIdentifier else {
        throw .notSupported("Secure storage requires a team-signed build")
      }
    #endif
  }

  /// The value and its protection, as keychain attributes. Access control even
  /// without authentication, so an update removes any the existing value had.
  private func stored(
    _ value: String, _ options: StoreOptions
  ) throws(TokamakPluginError) -> [CFString: Any] {
    let accessibility = accessibility(
      options.readable, thisDeviceOnly: options.thisDeviceOnly)
    return [
      kSecValueData: Data(value.utf8),
      kSecAttrAccessControl: try accessControl(accessibility, options.authentication),
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
    prompting: Result<Void, TokamakPluginError>?,
    authentication: LAContext
  ) throws(TokamakPluginError) -> OSStatus {
    var item = query(name, group: group)
    item[kSecUseAuthenticationContext] = Self.contextWithoutUI()
    let status = SecItemUpdate(item as CFDictionary, stored as CFDictionary)
    guard status == errSecInteractionNotAllowed, let prompting else { return status }
    try prompting.get()
    item[kSecUseAuthenticationContext] = Self.context(authentication, prompt: prompt)
    return SecItemUpdate(item as CFDictionary, stored as CFDictionary)
  }

  /// Reads without UI first, so only a value saved with authentication needs
  /// the app able to show the system prompt.
  private func read(
    _ name: String,
    group: String?,
    prompt: String?,
    prompting: Result<Void, TokamakPluginError>?,
    authentication: LAContext
  ) throws(TokamakPluginError) -> String {
    var item = query(name, group: group)
    item[kSecReturnData] = true
    item[kSecUseAuthenticationContext] = Self.contextWithoutUI()
    var data: CFTypeRef?
    var status = SecItemCopyMatching(item as CFDictionary, &data)
    if status == errSecInteractionNotAllowed, let prompting {
      try prompting.get()
      item[kSecUseAuthenticationContext] = Self.context(authentication, prompt: prompt)
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

  /// A context in which keychain calls that need authentication fail with
  /// `errSecInteractionNotAllowed` rather than show the system prompt.
  private static func contextWithoutUI() -> LAContext {
    let context = LAContext()
    context.interactionNotAllowed = true
    return context
  }

  /// The invocation's authentication context, with the requested prompt.
  private static func context(_ context: LAContext, prompt: String?) -> LAContext {
    if let prompt, !prompt.isEmpty { context.localizedReason = prompt }
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
  private func accessibility(_ readable: Readable, thisDeviceOnly: Bool) -> CFString {
    switch (readable, thisDeviceOnly) {
    case (.whenUnlocked, true): kSecAttrAccessibleWhenUnlockedThisDeviceOnly
    case (.whenUnlocked, false): kSecAttrAccessibleWhenUnlocked
    case (.afterFirstUnlock, true): kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
    case (.afterFirstUnlock, false): kSecAttrAccessibleAfterFirstUnlock
    }
  }

  private func accessControl(
    _ accessibility: CFString,
    _ authentication: Authentication?
  ) throws(TokamakPluginError) -> SecAccessControl {
    let (flags, policy): (SecAccessControlCreateFlags, LAPolicy?) =
      switch authentication {
      case nil: ([], nil)
      case .biometricsOrPasscode: (.userPresence, .deviceOwnerAuthentication)
      case .biometrics: (.biometryAny, .deviceOwnerAuthenticationWithBiometrics)
      case .currentBiometrics: (.biometryCurrentSet, .deviceOwnerAuthenticationWithBiometrics)
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
}
