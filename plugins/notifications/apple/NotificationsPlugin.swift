import Foundation
import UserNotifications

#if os(iOS)
  import UIKit
#else
  import AppKit
#endif

private let subscriptionKey = "tokamak.notifications.subscription"
private let showInForegroundKey = "tokamak.notifications.show-in-foreground"
/// The system keeps only the soonest 64 pending requests.
private let pendingLimit = 64
/// The system allows about 30 seconds for a background remote notification.
private let pushDeadline: TimeInterval = 25
private let shown: UNNotificationPresentationOptions = [.banner, .list, .sound]

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
  /// Notifications opened while no page listened for them.
  private var heldOpened: [[String: Any]] = []
  private var registering: [TokamakPluginReply] = []

  /// Becomes the notification centre's delegate before launch finishes, which
  /// the system requires to deliver the notification that launched the app.
  init(host: TokamakHost) {
    self.host = host
    super.init()
    center.delegate = self
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
      case "permission":
        permission(reply)
      case "requestPermission":
        center.requestAuthorization(options: [.alert, .sound, .badge]) { _, _ in
          self.permission(reply)
        }
      case "show":
        add(try TokamakNotificationRequest(arguments, scheduled: false), reply: reply)
      case "schedule":
        schedule(try TokamakNotificationRequest(arguments, scheduled: true), reply: reply)
      case "getScheduled":
        center.getPendingNotificationRequests { reply(.success($0.map(\.scheduled))) }
      case "getDelivered":
        center.getDeliveredNotifications {
          reply(.success($0.map { TokamakNotificationFields($0).content }))
        }
      case "remove":
        let id = try identifier(arguments)
        center.removePendingNotificationRequests(withIdentifiers: [id])
        center.removeDeliveredNotifications(withIdentifiers: [id])
        reply(.success(nil))
      case "subscribe":
        try subscribeToPush(arguments, reply: reply)
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
  ) -> (() -> Void) {
    guard ["onMessage", "onNotificationOpened", "onSubscriptionChange"].contains(method) else {
      reply(.failure(.notSupported("\(id).\(method) is not supported")))
      return {}
    }
    let key = UUID()
    listeners[method, default: [:]][key] = reply
    if method == "onNotificationOpened" {
      let held = heldOpened
      heldOpened.removeAll()
      for opened in held {
        reply(.success(opened))
      }
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
      emit("onSubscriptionChange", subscription)
    }
  }

  func didFailToRegisterForRemoteNotifications(error: Error) {
    finishRegistering(.failure(Self.registrationError(error)))
  }

  /// Posts a data-only message to the Worker's push endpoint, showing the
  /// notification it returns.
  func didReceiveRemoteNotification(
    _ userInfo: [AnyHashable: Any],
    completion: @escaping (TokamakBackgroundResult) -> Void
  ) {
    let fields = TokamakNotificationFields(remoteNotification: userInfo)
    guard fields.title == nil, fields.body == nil else {
      completion(.noData)
      return
    }
    emit("onMessage", fields.message)
    host.call("push", body: fields.message, timeout: pushDeadline) { result in
      switch result {
      case .success(let response):
        self.show(response) { completion(.newData) }
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
      guard fields.isPush else {
        completionHandler(shown)
        return
      }
      self.emit("onMessage", fields.message)
      completionHandler(self.defaults.bool(forKey: showInForegroundKey) ? shown : [])
    }
  }

  func userNotificationCenter(
    _ center: UNUserNotificationCenter,
    didReceive response: UNNotificationResponse,
    withCompletionHandler completionHandler: @escaping () -> Void
  ) {
    let opened = TokamakNotificationFields(response.notification).opened
    DispatchQueue.main.async {
      self.emit("onNotificationOpened", opened)
      completionHandler()
    }
  }

  private func emit(_ method: String, _ value: [String: Any]) {
    let current = listeners[method] ?? [:]
    if method == "onNotificationOpened" && current.isEmpty {
      heldOpened.append(value)
    }
    for reply in current.values {
      reply(.success(value))
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

  private func schedule(_ request: TokamakNotificationRequest, reply: @escaping TokamakPluginReply)
  {
    center.getPendingNotificationRequests { pending in
      let replaces = pending.contains { $0.identifier == request.id }
      guard replaces || pending.count < pendingLimit else {
        reply(
          .failure(
            TokamakPluginError(
              name: "QuotaExceededError",
              message: "The app has \(pendingLimit) scheduled notifications, the most allowed"
            )))
        return
      }
      self.add(request, reply: reply)
    }
  }

  private func add(_ request: TokamakNotificationRequest, reply: @escaping TokamakPluginReply) {
    let notification: UNNotificationRequest
    do {
      notification = try request.request()
    } catch {
      reply(.failure(error))
      return
    }
    center.getNotificationSettings { settings in
      guard Self.permission(settings.authorizationStatus) == "granted" else {
        reply(
          .failure(
            TokamakPluginError(
              name: "NotAllowedError",
              message: "Notification permission has not been granted"
            )))
        return
      }
      self.center.add(notification) { error in
        reply(error.map { .failure(Self.operationError($0)) } ?? .success(nil))
      }
    }
  }

  /// Shows the notification a push response returns, if any.
  private func show(_ response: Data, completion: @escaping () -> Void) {
    let returned = try? JSONSerialization.jsonObject(with: response, options: .fragmentsAllowed)
    guard !response.isEmpty, !(returned is NSNull) else {
      completion()
      return
    }
    guard let returned, let request = try? TokamakNotificationRequest(returned, scheduled: false)
    else {
      print("tokamak push response is not a notification")
      completion()
      return
    }
    add(request) { _ in completion() }
  }

  private func subscribeToPush(
    _ arguments: Any,
    reply: @escaping TokamakPluginReply
  ) throws(TokamakPluginError) {
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
    let options = arguments as? [String: Any] ?? [:]
    defaults.set(options["showInForeground"] as? Bool ?? false, forKey: showInForegroundKey)
    registering.append(reply)
    registerForRemoteNotifications()
  }

  /// Stops push delivery, failing a `subscribe` still waiting for its token.
  private func unsubscribe() {
    unregisterForRemoteNotifications()
    defaults.removeObject(forKey: subscriptionKey)
    defaults.removeObject(forKey: showInForegroundKey)
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
