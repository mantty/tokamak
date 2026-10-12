import Foundation
import UserNotifications

#if os(iOS)
  import UIKit
#else
  import AppKit
#endif

private let subscriptionKey = "tokamak.notifications.subscription"
/// The JSON of the configuration `setup` last applied.
private let setupKey = "tokamak.notifications.setup"
/// The system keeps only the soonest 64 pending requests.
private let pendingLimit = 64
/// The system allows about 30 seconds for a background remote notification.
private let pushDeadline: TimeInterval = 25
/// How long a notification's attachments may take to load.
private let attachmentDeadline: TimeInterval = 10
/// How long the Worker's listeners have for an opened or dismissed notification.
private let responseDeadline: TimeInterval = 10
/// Listener methods that receive what happened before a page listened.
private let heldMethods: Set = ["onNotificationOpened", "onNotificationDismissed"]

#if os(iOS)
  private let pushEntitlement = "aps-environment"
#else
  private let pushEntitlement = "com.apple.developer.aps-environment"
#endif

final class TokamakNotificationsPlugin: NSObject, TokamakPlugin,
  UNUserNotificationCenterDelegate
{
  let id = "notifications"

  private let host: TokamakHost
  private let center = UNUserNotificationCenter.current()
  private let defaults = UserDefaults.standard
  private var listeners: [String: [UUID: TokamakPluginReply]] = [:]
  /// Notifications opened or dismissed while no page listened for them, by listener method.
  private var held: [String: [[String: Any]]] = [:]
  private var registering: [TokamakPluginReply] = []
  private var setup = TokamakNotificationSetup.initial
  private let attachments: TokamakNotificationAttachments
  /// The latest add of each notification whose attachments are loading, which `remove` cancels.
  private var adding: [String: UUID] = [:]

  /// Becomes the notification centre's delegate and applies the last `setup`
  /// before launch finishes, which the system requires to deliver the
  /// notification that launched the app with its actions.
  init(host: TokamakHost) {
    self.host = host
    attachments = TokamakNotificationAttachments(host: host)
    super.init()
    center.delegate = self
    if let stored = defaults.data(forKey: setupKey),
      let config = try? JSONSerialization.jsonObject(with: stored),
      let setup = try? TokamakNotificationSetup(config)
    {
      apply(setup)
    }
    if defaults.dictionary(forKey: subscriptionKey) != nil {
      registerForRemoteNotifications()
    }
  }

  func call(
    method: String,
    arguments: Any,
    reply: @escaping TokamakPluginReply
  ) {
    do throws(TokamakPluginError) {
      switch method {
      case "setup":
        apply(try TokamakNotificationSetup(arguments))
        defaults.set(try? JSONSerialization.data(withJSONObject: arguments), forKey: setupKey)
        reply(.success(["unapplied": [Any]()]))
      case "permission":
        permission(reply)
      case "requestPermission":
        requestPermission(reply)
      case "show":
        add(try TokamakNotificationRequest(arguments, scheduled: false), reply: reply)
      case "schedule":
        add(try TokamakNotificationRequest(arguments, scheduled: true), reply: reply)
      case "getScheduled":
        center.getPendingNotificationRequests { reply(.success($0.map(\.scheduled))) }
      case "getDelivered":
        center.getDeliveredNotifications {
          reply(.success($0.map { TokamakNotificationFields($0).content }))
        }
      case "remove":
        let id = try identifier(arguments)
        adding[id] = nil
        center.removePendingNotificationRequests(withIdentifiers: [id])
        center.removeDeliveredNotifications(withIdentifiers: [id])
        reply(.success(nil))
      case "subscribe":
        try subscribeToPush(reply: reply)
      case "getSubscription":
        reply(.success(defaults.dictionary(forKey: subscriptionKey)))
      case "unsubscribe":
        unsubscribe()
        reply(.success(nil))
      default:
        throw .notSupported("\(id).\(method) is not supported")
      }
    } catch {
      reply(.failure(error))
    }
  }

  func subscribe(
    method: String,
    arguments: Any,
    reply: @escaping TokamakPluginReply
  ) throws(TokamakPluginError) -> (() -> Void) {
    let methods = [
      "onMessage", "onNotificationOpened", "onNotificationDismissed", "onSubscriptionChange",
    ]
    guard methods.contains(method) else {
      throw .notSupported("\(id).\(method) is not supported")
    }
    let key = UUID()
    listeners[method, default: [:]][key] = reply
    for value in held.removeValue(forKey: method) ?? [] {
      reply(.success(value))
    }
    return { [weak self] in
      self?.listeners[method]?.removeValue(forKey: key)
    }
  }

  func didRegisterForRemoteNotifications(deviceToken: Data) {
    let previous = defaults.dictionary(forKey: subscriptionKey)
    guard previous != nil || !registering.isEmpty else { return }
    let subscription: [String: Any] = [
      "service": "apns",
      "token": deviceToken.map { String(format: "%02x", $0) }.joined(),
      "environment": Self.apsEnvironment ?? "development",
    ]
    defaults.set(subscription, forKey: subscriptionKey)
    finishRegistering(.success(subscription))
    if let previous, previous["token"] as? String != subscription["token"] as? String {
      notify("onSubscriptionChange", subscription)
    }
  }

  func didFailToRegisterForRemoteNotifications(error: Error) {
    finishRegistering(.failure(Self.registrationError(error)))
  }

  /// Delivers a data-only message to the Worker's `onPush` listeners,
  /// showing the notification they reply with.
  func didReceiveRemoteNotification(
    _ userInfo: [AnyHashable: Any],
    completion: @escaping (TokamakBackgroundResult) -> Void
  ) {
    let fields = TokamakNotificationFields(remoteNotification: userInfo)
    guard fields.title == nil, fields.body == nil else {
      completion(.noData)
      return
    }
    notify("onMessage", fields.message)
    let deadline = Date() + pushDeadline
    host.emit("push", event: fields.message, timeout: pushDeadline) { result in
      switch result {
      case .success(let reply):
        self.show(reply, attachmentTimeout: deadline.timeIntervalSinceNow) { completion(.newData) }
      case .failure(let error):
        print("push notification failed: \(error)")
        completion(.failed)
      }
    }
  }

  func userNotificationCenter(
    _ center: UNUserNotificationCenter,
    willPresent notification: UNNotification,
    withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
  ) {
    let fields = TokamakNotificationFields(notification)
    DispatchQueue.main.async {
      if fields.isPush {
        self.notify("onMessage", fields.message)
      }
      completionHandler(self.setup.presentation)
    }
  }

  func userNotificationCenter(
    _ center: UNUserNotificationCenter,
    didReceive response: UNNotificationResponse,
    withCompletionHandler completionHandler: @escaping () -> Void
  ) {
    let fields = TokamakNotificationFields(response.notification)
    let (method, event, value) =
      switch response.actionIdentifier {
      case UNNotificationDismissActionIdentifier:
        ("onNotificationDismissed", "dismissed", fields.dismissed)
      case let action:
        (
          "onNotificationOpened", "opened",
          fields.opened(
            action: action == UNNotificationDefaultActionIdentifier ? nil : action,
            input: (response as? UNTextInputNotificationResponse)?.userText)
        )
      }
    DispatchQueue.main.async {
      self.notify(method, value)
      self.host.emit(event, event: value, timeout: responseDeadline) { result in
        if case .failure(let error) = result {
          print("\(event) notification failed: \(error)")
        }
        completionHandler()
      }
    }
  }

  /// Uses `setup` from now on, registering its categories.
  private func apply(_ setup: TokamakNotificationSetup) {
    self.setup = setup
    center.setNotificationCategories(setup.categories)
  }

  private func notify(_ method: String, _ value: [String: Any]) {
    let current = listeners[method] ?? [:]
    if heldMethods.contains(method) && current.isEmpty {
      held[method, default: []].append(value)
    }
    for reply in current.values {
      reply(.success(value))
    }
  }

  /// Asks for permission when the system has not yet, which shows its prompt.
  private func requestPermission(_ reply: @escaping TokamakPluginReply) {
    center.getNotificationSettings { settings in
      guard settings.authorizationStatus == .notDetermined else {
        reply(.success(Self.permission(settings.authorizationStatus)))
        return
      }
      DispatchQueue.main.async {
        do throws(TokamakPluginError) {
          try self.host.requireUI()
          self.center.requestAuthorization(options: [.alert, .sound, .badge]) { _, _ in
            self.permission(reply)
          }
        } catch {
          reply(.failure(error))
        }
      }
    }
  }

  private func permission(_ reply: @escaping TokamakPluginReply) {
    center.getNotificationSettings { settings in
      reply(.success(Self.permission(settings.authorizationStatus)))
    }
  }

  private static func permission(_ status: UNAuthorizationStatus) -> String {
    switch status {
    case .denied: return "denied"
    case .notDetermined: return "prompt"
    default: return "granted"
    }
  }

  /// Adds `request` with the attachments that load within `timeout`, unless `remove` or a later
  /// add of its id comes first. Call it on the main thread.
  private func add(
    _ request: TokamakNotificationRequest,
    attachmentTimeout timeout: TimeInterval = attachmentDeadline,
    reply: @escaping TokamakPluginReply
  ) {
    let token = UUID()
    adding[request.id] = token
    checkAddable(request) { error in
      if let error {
        _ = self.endAdding(request.id, token)
        reply(.failure(error))
        return
      }
      self.attachments.load(request.fields, timeout: timeout) { attachments in
        self.add(request, attachments: attachments, token: token, reply: reply)
      }
    }
  }

  /// Whether `token` is still the latest add of notification `id`, which this ends.
  private func endAdding(_ id: String, _ token: UUID) -> Bool {
    guard adding[id] == token else { return false }
    adding[id] = nil
    return true
  }

  /// Calls `completion` on the main thread with nil when `request` may be added, or with the
  /// error that prevents it: no permission, or a scheduled request beyond the system's limit.
  private func checkAddable(
    _ request: TokamakNotificationRequest,
    completion: @escaping (TokamakPluginError?) -> Void
  ) {
    let complete: (TokamakPluginError?) -> Void = { error in
      DispatchQueue.main.async { completion(error) }
    }
    center.getNotificationSettings { settings in
      guard Self.permission(settings.authorizationStatus) == "granted" else {
        complete(
          TokamakPluginError(
            name: "NotAllowedError", message: "Notification permission has not been granted"))
        return
      }
      guard request.at != nil else {
        complete(nil)
        return
      }
      self.center.getPendingNotificationRequests { pending in
        let replaces = pending.contains { $0.identifier == request.id }
        complete(
          replaces || pending.count < pendingLimit
            ? nil
            : TokamakPluginError(
              name: "QuotaExceededError",
              message: "The app has \(pendingLimit) scheduled notifications, the most allowed"))
      }
    }
  }

  /// Adds `request` with `attachments`, unless `remove` or a later add of its id ended add `token`.
  private func add(
    _ request: TokamakNotificationRequest, attachments: [UNNotificationAttachment], token: UUID,
    reply: @escaping TokamakPluginReply
  ) {
    guard endAdding(request.id, token) else {
      Self.removeFiles(of: attachments)
      reply(.success(nil))
      return
    }
    do throws(TokamakPluginError) {
      center.add(try request.request(attachments: attachments)) { error in
        if let error {
          Self.removeFiles(of: attachments)
          reply(.failure(Self.operationError(error)))
        } else {
          reply(.success(nil))
        }
      }
    } catch {
      Self.removeFiles(of: attachments)
      reply(.failure(error))
    }
  }

  /// Deletes the temporary files of `attachments` that were not added. The system moves the files
  /// of the attachments it adds.
  private static func removeFiles(of attachments: [UNNotificationAttachment]) {
    for attachment in attachments { try? FileManager.default.removeItem(at: attachment.url) }
  }

  /// Shows the notification `onPush` listeners reply with, if any, with the attachments that load
  /// within `timeout`.
  private func show(
    _ reply: Any?, attachmentTimeout timeout: TimeInterval, completion: @escaping () -> Void
  ) {
    guard let reply else {
      completion()
      return
    }
    guard let request = try? TokamakNotificationRequest(reply, scheduled: false) else {
      print("tokamak push reply is not a notification")
      completion()
      return
    }
    add(request, attachmentTimeout: timeout) { _ in completion() }
  }

  private func subscribeToPush(reply: @escaping TokamakPluginReply) throws(TokamakPluginError) {
    #if os(macOS)
      guard Self.entitlement("com.apple.application-identifier") != nil else {
        throw .notSupported("Push requires a team-signed build")
      }
    #endif
    #if !targetEnvironment(simulator)
      guard Self.apsEnvironment != nil else {
        throw TokamakPluginError(
          name: "InvalidStateError",
          message: "Push requires the app to declare the \(pushEntitlement) entitlement"
        )
      }
    #endif
    registering.append(reply)
    registerForRemoteNotifications()
  }

  /// Stops push delivery, failing a `subscribe` still waiting for its token.
  private func unsubscribe() {
    unregisterForRemoteNotifications()
    defaults.removeObject(forKey: subscriptionKey)
    finishRegistering(
      .failure(TokamakPluginError(name: "AbortError", message: "unsubscribe was called")))
  }

  /// Replies to every `subscribe` waiting for its token.
  private func finishRegistering(_ result: Result<Any?, TokamakPluginError>) {
    let replies = registering
    registering.removeAll()
    for reply in replies {
      reply(result)
    }
  }

  private func registerForRemoteNotifications() {
    #if os(iOS)
      UIApplication.shared.registerForRemoteNotifications()
    #else
      NSApplication.shared.registerForRemoteNotifications()
    #endif
  }

  private func unregisterForRemoteNotifications() {
    #if os(iOS)
      UIApplication.shared.unregisterForRemoteNotifications()
    #else
      NSApplication.shared.unregisterForRemoteNotifications()
    #endif
  }

  private func identifier(_ arguments: Any) throws(TokamakPluginError) -> String {
    guard let id = (arguments as? [String: Any])?["id"] as? String, !id.isEmpty else {
      throw TokamakPluginError(name: "TypeError", message: "id must be a non-empty string")
    }
    return id
  }

  /// The APNs environment the app is signed for, or nil without the push entitlement.
  private static let apsEnvironment: String? = {
    #if targetEnvironment(simulator)
      return "development"
    #elseif os(iOS)
      return profileEntitlement(pushEntitlement)
    #else
      return entitlement(pushEntitlement) as? String
    #endif
  }()

  #if os(macOS)
    private static func entitlement(_ key: String) -> Any? {
      guard let task = SecTaskCreateFromSelf(nil) else { return nil }
      return SecTaskCopyValueForEntitlement(task, key as CFString, nil)
    }
  #endif

  #if os(iOS) && !targetEnvironment(simulator)
    /// An entitlement in the embedded provisioning profile, which signing takes
    /// the push environment from.
    private static func profileEntitlement(_ key: String) -> String? {
      guard
        let url = Bundle.main.url(forResource: "embedded", withExtension: "mobileprovision"),
        let profile = try? Data(contentsOf: url),
        let start = profile.range(of: Data("<?xml".utf8)),
        let end = profile.range(of: Data("</plist>".utf8), in: start.lowerBound..<profile.endIndex),
        let plist = try? PropertyListSerialization.propertyList(
          from: profile[start.lowerBound..<end.upperBound],
          format: nil
        ) as? [String: Any]
      else {
        return nil
      }
      return (plist["Entitlements"] as? [String: Any])?[key] as? String
    }
  #endif

  private static func registrationError(_ error: Error) -> TokamakPluginError {
    // Cocoa reports a missing aps-environment entitlement as error 3000.
    let error = error as NSError
    if error.domain == NSCocoaErrorDomain && error.code == 3000 {
      return TokamakPluginError(name: "InvalidStateError", message: error.localizedDescription)
    }
    return operationError(error)
  }

  private static func operationError(_ error: Error) -> TokamakPluginError {
    TokamakPluginError(name: "OperationError", message: error.localizedDescription)
  }
}
