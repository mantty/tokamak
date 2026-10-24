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

#if os(iOS)
  private let pushEntitlement = "aps-environment"
#else
  private let pushEntitlement = "com.apple.developer.aps-environment"
#endif

final class TokamakNotificationsPlugin: NSObject, TokamakPlugin,
  UNUserNotificationCenterDelegate
{
  let id = "notifications"
  let host: TokamakHost

  private let center = UNUserNotificationCenter.current()
  private let defaults = UserDefaults.standard
  private let messages = TokamakListeners<Message>()
  private let opened = TokamakListeners<OpenedNotification>(holding: true)
  private let dismissed = TokamakListeners<DismissedNotification>(holding: true)
  private let settingsRequests = TokamakListeners<OpenSettingsEvent>(holding: true)
  private let subscriptionChanges = TokamakListeners<Subscription>()
  /// The `subscribeToPush` calls waiting for the device token.
  private var registering: [CheckedContinuation<Subscription, Error>] = []
  private var configuration = TokamakNotificationSetup.initial
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
      let config = try? JSONDecoder().decode(SetupConfig.self, from: stored)
    {
      apply(TokamakNotificationSetup(config))
    }
    if storedSubscription != nil {
      registerForRemoteNotifications()
    }
  }

  func setup(_ config: SetupConfig) -> SetupResult {
    apply(TokamakNotificationSetup(config))
    defaults.set(try? JSONEncoder().encode(config), forKey: setupKey)
    return SetupResult(unapplied: [])
  }

  func permission() async -> Permission {
    Self.permission(await center.notificationSettings().authorizationStatus)
  }

  /// Asks for `options` when the system has not asked yet, which shows its
  /// prompt, to turn a provisional grant into a full one, or to add critical
  /// alerts or the settings link to a grant. A provisional request shows no
  /// prompt.
  @MainActor
  func requestPermission(_ options: PermissionOptions?) async throws -> Permission {
    let options = Self.authorizationOptions(options)
    let provisional = options.contains(.provisional)
    let settings = await center.notificationSettings()
    let status = settings.authorizationStatus
    let adds =
      (options.contains(.criticalAlert) && settings.criticalAlertSetting == .notSupported)
      || (options.contains(.providesAppNotificationSettings)
        && !settings.providesAppNotificationSettings)
    let asks =
      status == .notDetermined || (status == .provisional && !provisional)
      || (status != .denied && adds)
    guard asks else { return Self.permission(status) }
    if !provisional {
      try host.requireUI()
    }
    _ = try? await center.requestAuthorization(options: options)
    return await permission()
  }

  func settings() async -> NotificationSettings {
    Self.settings(await center.notificationSettings())
  }

  func setBadge(_ count: Int) async throws {
    guard count >= 0 else {
      throw TokamakPluginError(name: "TypeError", message: "count must be at least 0")
    }
    try await center.setBadgeCount(count)
  }

  @MainActor
  func openSettings() async throws {
    try host.requireUI()
    #if os(iOS)
      let url = URL(string: UIApplication.openNotificationSettingsURLString)!
      let opened = await UIApplication.shared.open(url)
    #else
      // Undocumented; it opens the app's own page of Notifications settings.
      let page =
        "com.apple.Notifications-Settings.extension?id=\(Bundle.main.bundleIdentifier ?? "")"
      let opened = URL(string: "x-apple.systempreferences:\(page)").map(NSWorkspace.shared.open)
    #endif
    guard opened == true else {
      throw TokamakPluginError(
        name: "OperationError", message: "The notification settings could not be opened")
    }
  }

  @MainActor
  func show(_ notification: NotificationContent) async throws {
    try await add(TokamakNotificationRequest(notification))
  }

  @MainActor
  func schedule(_ notification: NotificationContent, trigger: Trigger) async throws {
    try await add(TokamakNotificationRequest(notification, trigger: trigger))
  }

  func getScheduled() async -> [ScheduledNotification] {
    await center.pendingNotificationRequests().map(\.scheduled)
  }

  func getDelivered() async -> [NotificationContent] {
    await center.deliveredNotifications().map { TokamakNotificationFields($0.request).content }
  }

  func remove(_ id: String) {
    adding[id] = nil
    center.removePendingNotificationRequests(withIdentifiers: [id])
    center.removeDeliveredNotifications(withIdentifiers: [id])
  }

  /// Registers for push messages, resolving to the subscription once the
  /// system gives the device token.
  @MainActor
  func subscribeToPush() async throws -> Subscription {
    #if os(macOS)
      guard Self.entitlement("com.apple.application-identifier") != nil else {
        throw TokamakPluginError.notSupported("Push requires a team-signed build")
      }
      guard Self.apsEnvironment != nil else {
        throw TokamakPluginError(
          name: "InvalidStateError",
          message: "Push requires the app to declare the \(pushEntitlement) entitlement"
        )
      }
    #endif
    return try await withCheckedThrowingContinuation { continuation in
      registering.append(continuation)
      registerForRemoteNotifications()
    }
  }

  func getSubscription() -> Subscription? {
    storedSubscription
  }

  /// Stops push delivery, failing a `subscribeToPush` still waiting for its token.
  func unsubscribe() {
    unregisterForRemoteNotifications()
    defaults.removeObject(forKey: subscriptionKey)
    finishRegistering(
      .failure(TokamakPluginError(name: "AbortError", message: "unsubscribe was called")))
  }

  func onMessage() -> AsyncStream<Message> {
    messages.stream()
  }

  func onNotificationOpened() -> AsyncStream<OpenedNotification> {
    opened.stream()
  }

  func onNotificationDismissed() -> AsyncStream<DismissedNotification> {
    dismissed.stream()
  }

  func onOpenSettings() -> AsyncStream<OpenSettingsEvent> {
    settingsRequests.stream()
  }

  func onSubscriptionChange() -> AsyncStream<Subscription> {
    subscriptionChanges.stream()
  }

  func didRegisterForRemoteNotifications(deviceToken: Data) {
    let previous = storedSubscription
    guard previous != nil || !registering.isEmpty else { return }
    let subscription = Subscription(
      service: .apns,
      token: deviceToken.map { String(format: "%02x", $0) }.joined(),
      environment: Self.apsEnvironment ?? .development
    )
    defaults.set(
      [
        "service": subscription.service.rawValue, "token": subscription.token,
        "environment": subscription.environment.rawValue,
      ], forKey: subscriptionKey)
    finishRegistering(.success(subscription))
    if let previous, previous.token != subscription.token {
      subscriptionChanges.send(subscription)
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
    let message = fields.message
    messages.send(message)
    let deadline = Date() + pushDeadline
    let showReply = { @MainActor (reply: NotificationContent?) async in
      await self.show(reply: reply, attachmentTimeout: deadline.timeIntervalSinceNow)
      completion(.newData)
    }
    host.emit("push", event: message, reply: NotificationContent.self, timeout: pushDeadline) {
      result in
      switch result {
      case .success(let reply):
        Task { await showReply(reply) }
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
    let fields = TokamakNotificationFields(notification.request)
    DispatchQueue.main.async {
      if fields.isPush {
        self.messages.send(fields.message)
      }
      completionHandler(self.configuration.presentation)
    }
  }

  func userNotificationCenter(
    _ center: UNUserNotificationCenter,
    didReceive response: UNNotificationResponse,
    withCompletionHandler completionHandler: @escaping () -> Void
  ) {
    let fields = TokamakNotificationFields(response.notification.request)
    let action = response.actionIdentifier
    let input = (response as? UNTextInputNotificationResponse)?.userText
    let delivered = { (result: Result<Void, Error>) in
      if case .failure(let error) = result {
        print("notification response failed: \(error)")
      }
      completionHandler()
    }
    DispatchQueue.main.async {
      if action == UNNotificationDismissActionIdentifier {
        let dismissed = fields.dismissed
        self.dismissed.send(dismissed)
        self.host.emit(
          "dismissed", event: dismissed, timeout: responseDeadline, completion: delivered)
      } else {
        let opened = fields.opened(
          action: action == UNNotificationDefaultActionIdentifier ? nil : action, input: input)
        self.opened.send(opened)
        self.host.emit("opened", event: opened, timeout: responseDeadline, completion: delivered)
      }
    }
  }

  /// Reports the user's request, from the system's settings, for the app's own. The system offers
  /// it once `requestPermission` asked for `providesAppNotificationSettings`.
  func userNotificationCenter(
    _ center: UNUserNotificationCenter, openSettingsFor notification: UNNotification?
  ) {
    let event = OpenSettingsEvent(notification: notification?.request.identifier)
    DispatchQueue.main.async { self.settingsRequests.send(event) }
  }

  /// Uses `setup` from now on, registering its categories.
  private func apply(_ setup: TokamakNotificationSetup) {
    configuration = setup
    center.setNotificationCategories(setup.categories)
  }

  /// The authorisation `requestPermission`'s options name: alerts, sounds and badges unless
  /// turned off, and the others when turned on.
  private static func authorizationOptions(_ given: PermissionOptions?) -> UNAuthorizationOptions {
    var named: [(Bool?, UNAuthorizationOptions, Bool)] = [
      (given?.alert, .alert, true), (given?.sound, .sound, true), (given?.badge, .badge, true),
      (given?.criticalAlert, .criticalAlert, false), (given?.provisional, .provisional, false),
      (given?.providesAppNotificationSettings, .providesAppNotificationSettings, false),
    ]
    #if os(iOS)
      named.append((given?.carPlay, .carPlay, false))
    #endif
    return named.reduce(into: []) { options, entry in
      if entry.0 ?? entry.2 { options.insert(entry.1) }
    }
  }

  private static func settings(_ settings: UNNotificationSettings) -> NotificationSettings {
    #if os(iOS)
      let (carPlay, announcement) = (
        setting(settings.carPlaySetting), setting(settings.announcementSetting)
      )
    #else
      let (carPlay, announcement): (Setting?, Setting?) = (nil, nil)
    #endif
    return NotificationSettings(
      permission: permission(settings.authorizationStatus),
      authorizationStatus: authorizationStatus(settings.authorizationStatus),
      alertStyle: [.none: .none, .banner: .banner, .alert: .alert][settings.alertStyle] ?? .none,
      showPreviews: [.always: .always, .whenAuthenticated: .whenAuthenticated, .never: .never][
        settings.showPreviewsSetting] ?? .always,
      alert: setting(settings.alertSetting),
      sound: setting(settings.soundSetting),
      badge: setting(settings.badgeSetting),
      lockScreen: setting(settings.lockScreenSetting),
      notificationCenter: setting(settings.notificationCenterSetting),
      criticalAlert: setting(settings.criticalAlertSetting),
      timeSensitive: setting(settings.timeSensitiveSetting),
      scheduledDelivery: setting(settings.scheduledDeliverySetting),
      directMessages: setting(settings.directMessagesSetting),
      carPlay: carPlay,
      announcement: announcement,
      providesAppNotificationSettings: settings.providesAppNotificationSettings
    )
  }

  private static func authorizationStatus(_ status: UNAuthorizationStatus) -> AuthorizationStatus {
    switch status {
    case .denied: .denied
    case .authorized: .authorized
    case .provisional: .provisional
    case .ephemeral: .ephemeral
    default: .notDetermined
    }
  }

  private static func setting(_ setting: UNNotificationSetting) -> Setting {
    switch setting {
    case .enabled: .enabled
    case .disabled: .disabled
    default: .notSupported
    }
  }

  private static func permission(_ status: UNAuthorizationStatus) -> Permission {
    switch status {
    case .denied: .denied
    case .notDetermined: .prompt
    default: .granted
    }
  }

  /// Adds `request` with the attachments that load within `timeout`, unless `remove` or a later
  /// add of its id comes first.
  @MainActor
  private func add(
    _ request: TokamakNotificationRequest,
    attachmentTimeout timeout: TimeInterval = attachmentDeadline
  ) async throws {
    let token = UUID()
    adding[request.content.id] = token
    do {
      try await checkAddable(request)
    } catch {
      _ = endAdding(request.content.id, token)
      throw error
    }
    let attachments = await withCheckedContinuation { loaded in
      self.attachments.load(request.content, timeout: timeout) { loaded.resume(returning: $0) }
    }
    guard endAdding(request.content.id, token) else {
      Self.removeFiles(of: attachments)
      return
    }
    do {
      try await center.add(try request.request(attachments: attachments))
    } catch {
      Self.removeFiles(of: attachments)
      throw TokamakPluginError(error)
    }
  }

  /// Whether `token` is still the latest add of notification `id`, which this ends.
  @MainActor
  private func endAdding(_ id: String, _ token: UUID) -> Bool {
    guard adding[id] == token else { return false }
    adding[id] = nil
    return true
  }

  /// Throws unless `request` may be added: without permission, or for a scheduled request beyond
  /// the system's limit.
  private func checkAddable(_ request: TokamakNotificationRequest) async throws(TokamakPluginError)
  {
    guard await permission() == .granted else {
      throw TokamakPluginError(
        name: "NotAllowedError", message: "Notification permission has not been granted")
    }
    guard request.trigger != nil else { return }
    let pending = await center.pendingNotificationRequests()
    guard
      pending.contains(where: { $0.identifier == request.content.id })
        || pending.count < pendingLimit
    else {
      throw TokamakPluginError(
        name: "QuotaExceededError",
        message: "The app has \(pendingLimit) scheduled notifications, the most allowed")
    }
  }

  /// Deletes the temporary files of `attachments` that were not added. The system moves the files
  /// of the attachments it adds.
  private static func removeFiles(of attachments: [UNNotificationAttachment]) {
    for attachment in attachments { try? FileManager.default.removeItem(at: attachment.url) }
  }

  /// Shows the notification `onPush` listeners reply with, if any, with the attachments that load
  /// within `timeout`.
  @MainActor
  private func show(
    reply: NotificationContent?, attachmentTimeout timeout: TimeInterval
  ) async {
    guard let reply else { return }
    guard let request = try? TokamakNotificationRequest(reply) else {
      print("tokamak push reply has no id")
      return
    }
    try? await add(request, attachmentTimeout: timeout)
  }

  /// The subscription `subscribeToPush` last gave, stored as a dictionary.
  private var storedSubscription: Subscription? {
    guard let stored = defaults.dictionary(forKey: subscriptionKey),
      let token = stored["token"] as? String,
      let environment = (stored["environment"] as? String).flatMap(PushEnvironment.init)
    else { return nil }
    return Subscription(service: .apns, token: token, environment: environment)
  }

  /// Resumes every `subscribeToPush` waiting for its token.
  private func finishRegistering(_ result: Result<Subscription, Error>) {
    let waiting = registering
    registering.removeAll()
    for continuation in waiting {
      continuation.resume(with: result)
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

  /// The APNs environment the app is signed for, or nil without a readable push entitlement.
  private static let apsEnvironment: PushEnvironment? = {
    #if targetEnvironment(simulator)
      return .development
    #elseif os(iOS)
      // App Store and TestFlight builds have no embedded profile and use production.
      guard
        let profileURL = Bundle.main.url(forResource: "embedded", withExtension: "mobileprovision")
      else {
        return .production
      }
      return profileEntitlement(pushEntitlement, in: profileURL).flatMap(PushEnvironment.init)
    #else
      return (entitlement(pushEntitlement) as? String).flatMap(PushEnvironment.init)
    #endif
  }()

  #if os(macOS)
    private static func entitlement(_ key: String) -> Any? {
      guard let task = SecTaskCreateFromSelf(nil) else { return nil }
      return SecTaskCopyValueForEntitlement(task, key as CFString, nil)
    }
  #endif

  #if os(iOS) && !targetEnvironment(simulator)
    /// An entitlement in a provisioning profile, which signing takes the push
    /// environment from.
    private static func profileEntitlement(_ key: String, in url: URL) -> String? {
      guard
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
    return TokamakPluginError(error)
  }
}
