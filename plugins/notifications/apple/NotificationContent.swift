import Foundation
import UserNotifications

#if os(iOS)
  import CoreLocation
#endif

/// The key under which the plugin stores a local notification's content in its `userInfo`, with
/// a scheduled notification's trigger in the same object, which `getScheduled` and
/// `getDelivered` return.
private let contentKey = "tokamak.content"

/// A notification the page asked to show or schedule.
struct TokamakNotificationRequest {
  typealias Plugin = TokamakNotificationsPlugin

  let content: Plugin.NotificationContent
  /// When the notification shows; nil to show now.
  let trigger: UNNotificationTrigger?
  /// The content and trigger as `userInfo` stores them.
  private let stored: String

  init(_ content: Plugin.NotificationContent, trigger: Plugin.Trigger? = nil)
    throws(TokamakPluginError)
  {
    guard !content.id.isEmpty else { throw typeError("id must be a non-empty string") }
    self.content = content
    if let trigger {
      self.trigger = try Self.trigger(trigger, id: content.id)
    } else {
      self.trigger = nil
    }
    stored = try json(content, trigger)
  }

  /// The system trigger of `trigger`: a time, nil once it has passed; an interval; the dates that
  /// match; or, on iOS, a region.
  private static func trigger(_ trigger: Plugin.Trigger, id: String) throws(TokamakPluginError)
    -> UNNotificationTrigger?
  {
    switch trigger {
    case .at(let at):
      guard at.at > Date().timeIntervalSince1970 * 1000 else { return nil }
      return UNCalendarNotificationTrigger(dateMatching: utcComponents(at.at), repeats: false)
    case .every(let every):
      let interval = every.every
      let seconds = [
        interval.seconds, interval.minutes.map { $0 * 60 }, interval.hours.map { $0 * 3600 },
        interval.days.map { $0 * 86400 },
      ].reduce(0) { $0 + ($1 ?? 0) }
      guard seconds >= 60 else { throw typeError("every must be at least 60 seconds") }
      return UNTimeIntervalNotificationTrigger(timeInterval: seconds, repeats: true)
    case .on(let on):
      // Android and the page number years and months as the Gregorian calendar does.
      let match = on.on
      var components = DateComponents(calendar: Calendar(identifier: .gregorian))
      components.year = match.year
      components.month = match.month
      components.day = match.day
      components.weekday = match.weekday
      components.hour = match.hour
      components.minute = match.minute
      components.second = match.second
      let trigger = UNCalendarNotificationTrigger(
        dateMatching: components, repeats: on.repeats ?? false)
      guard trigger.nextTriggerDate() != nil else { throw typeError("on matches no time from now") }
      return trigger
    case .region(let region):
      return try regionTrigger(region.region, id: id, repeats: region.repeats ?? false)
    }
  }

  #if os(iOS)
    private static func regionTrigger(_ region: Plugin.Region, id: String, repeats: Bool)
      throws(TokamakPluginError) -> UNNotificationTrigger
    {
      let status = CLLocationManager().authorizationStatus
      guard status == .authorizedWhenInUse || status == .authorizedAlways else {
        throw TokamakPluginError(
          name: "NotAllowedError", message: "A region trigger needs location permission")
      }
      let circle = CLCircularRegion(
        center: CLLocationCoordinate2D(latitude: region.latitude, longitude: region.longitude),
        radius: region.radius, identifier: id)
      circle.notifyOnEntry = region.onEntry ?? true
      circle.notifyOnExit = region.onExit ?? false
      return UNLocationNotificationTrigger(region: circle, repeats: repeats)
    }
  #else
    private static func regionTrigger(_ region: Plugin.Region, id: String, repeats: Bool)
      throws(TokamakPluginError) -> UNNotificationTrigger
    {
      throw .notSupported("Region triggers are iOS only")
    }
  #endif

  /// The system request, due now without a trigger or once its time has passed.
  func request(attachments: [UNNotificationAttachment]) throws(TokamakPluginError)
    -> UNNotificationRequest
  {
    let notification = UNMutableNotificationContent()
    notification.title = content.title
    notification.attachments = attachments
    notification.body = content.body ?? ""
    notification.subtitle = content.subtitle ?? ""
    notification.threadIdentifier = content.group ?? ""
    notification.badge = content.badge.map { $0 as NSNumber }
    notification.sound = content.silent == true ? nil : try Self.sound(content.sound)
    notification.categoryIdentifier = content.category ?? ""
    if #available(iOS 15, macOS 12, *) {
      if let level = content.interruptionLevel {
        notification.interruptionLevel = Self.interruptionLevel(level)
      }
      if let score = content.relevanceScore {
        notification.relevanceScore = score
      }
    }
    notification.userInfo = [contentKey: stored]
    return UNNotificationRequest(identifier: content.id, content: notification, trigger: trigger)
  }

  /// The system sound of `sound`: the default, the default critical sound, or a file the app
  /// bundles from the `sounds` build setting.
  private static func sound(_ sound: Plugin.Sound?) throws(TokamakPluginError)
    -> UNNotificationSound
  {
    switch sound {
    case nil, .default:
      return .default
    case .defaultCritical:
      return .defaultCritical
    case .named(let sound):
      guard Bundle.main.url(forResource: sound.name, withExtension: nil) != nil else {
        throw TokamakPluginError(
          name: "NotFoundError",
          message:
            "The app has no sound \(sound.name); add it to the notifications.sounds build setting")
      }
      let named = UNNotificationSoundName(sound.name)
      guard sound.critical == true else { return UNNotificationSound(named: named) }
      return .criticalSoundNamed(named, withAudioVolume: Float(sound.volume ?? 1))
    }
  }

  @available(iOS 15, macOS 12, *)
  private static func interruptionLevel(_ level: Plugin.InterruptionLevel)
    -> UNNotificationInterruptionLevel
  {
    switch level {
    case .passive: .passive
    case .active: .active
    case .timeSensitive: .timeSensitive
    case .critical: .critical
    }
  }

  private static func utcComponents(_ at: Double) -> DateComponents {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = TimeZone(identifier: "UTC")!
    return calendar.dateComponents(
      [.calendar, .timeZone, .year, .month, .day, .hour, .minute, .second],
      from: Date(timeIntervalSince1970: at / 1000)
    )
  }
}

/// The page's view of a notification: its content, and whether it came from a
/// push message.
struct TokamakNotificationFields {
  let id: String
  let title: String?
  let body: String?
  let data: [String: TokamakJSON]
  let category: String?
  let isPush: Bool
  /// The content of a local notification, if it has it stored.
  let stored: TokamakNotificationsPlugin.NotificationContent?

  init(_ request: UNNotificationRequest) {
    let content = request.content
    id = request.identifier
    title = content.title.isEmpty ? nil : content.title
    body = content.body.isEmpty ? nil : content.body
    category = content.categoryIdentifier.isEmpty ? nil : content.categoryIdentifier
    (data, isPush, stored) = Self.contents(content.userInfo)
  }

  /// A push message delivered to the app without a notification.
  init(remoteNotification userInfo: [AnyHashable: Any]) {
    let alert = (userInfo["aps"] as? [String: Any])?["alert"]
    id = UUID().uuidString
    title = (alert as? [String: Any])?["title"] as? String
    body = (alert as? [String: Any])?["body"] as? String ?? alert as? String
    category = (userInfo["aps"] as? [String: Any])?["category"] as? String
    (data, isPush, _) = Self.contents(userInfo)
    stored = nil
  }

  /// A push message's data is every top-level key but `aps`; a local
  /// notification's is the data the page gave it, stored with its content.
  private static func contents(_ userInfo: [AnyHashable: Any]) -> (
    [String: TokamakJSON], isPush: Bool, stored: TokamakNotificationsPlugin.NotificationContent?
  ) {
    if userInfo["aps"] != nil {
      var data: [String: TokamakJSON] = [:]
      for case (let key as String, let value) in userInfo where key != "aps" {
        data[key] = TokamakJSON(value)
      }
      return (data, true, nil)
    }
    let stored = storedContent(userInfo)?.content
    return (stored?.data ?? [:], false, stored)
  }

  var message: TokamakNotificationsPlugin.Message {
    TokamakNotificationsPlugin.Message(id: id, title: title, body: body, data: data)
  }

  var dismissed: TokamakNotificationsPlugin.DismissedNotification {
    TokamakNotificationsPlugin.DismissedNotification(
      id: id, title: title, body: body, data: data, source: isPush ? .push : .local)
  }

  func opened(action: String?, input: String?) -> TokamakNotificationsPlugin.OpenedNotification {
    TokamakNotificationsPlugin.OpenedNotification(
      id: id, title: title, body: body, data: data, source: isPush ? .push : .local,
      action: action, input: input)
  }

  var content: TokamakNotificationsPlugin.NotificationContent {
    stored
      ?? TokamakNotificationsPlugin.NotificationContent(
        id: id, title: title ?? "", body: body, data: data, category: category)
  }
}

extension UNNotificationRequest {
  /// The page's view of a pending request: its content and trigger, or the time it shows.
  var scheduled: TokamakNotificationsPlugin.ScheduledNotification {
    if let stored = storedContent(content.userInfo), let trigger = stored.trigger {
      return TokamakNotificationsPlugin.ScheduledNotification(
        content: stored.content, trigger: trigger)
    }
    let at =
      (trigger as? UNCalendarNotificationTrigger)?.nextTriggerDate().map {
        $0.timeIntervalSince1970 * 1000
      } ?? 0
    return TokamakNotificationsPlugin.ScheduledNotification(
      content: TokamakNotificationFields(self).content,
      trigger: .at(TokamakNotificationsPlugin.AtTrigger(at: at)))
  }
}

extension TokamakNotificationsPlugin.NotificationContent {
  /// A notification with only the fields every notification has.
  init(id: String, title: String, body: String?, data: [String: TokamakJSON], category: String?) {
    self.init(
      id: id, title: title, body: body, data: data, category: category, group: nil, image: nil,
      attachments: nil, badge: nil, silent: nil, sound: nil, subtitle: nil, interruptionLevel: nil,
      relevanceScore: nil)
  }
}

/// The content of a local notification, and a scheduled one's trigger, if it has them stored.
private func storedContent(_ userInfo: [AnyHashable: Any]) -> (
  content: TokamakNotificationsPlugin.NotificationContent,
  trigger: TokamakNotificationsPlugin.Trigger?
)? {
  guard let json = (userInfo[contentKey] as? String)?.data(using: .utf8),
    let content = try? JSONDecoder().decode(
      TokamakNotificationsPlugin.NotificationContent.self, from: json)
  else { return nil }
  return (content, try? JSONDecoder().decode(TokamakNotificationsPlugin.Trigger.self, from: json))
}

/// `content` and `trigger`'s fields as one JSON object, as the page gives a scheduled
/// notification.
private func json(
  _ content: TokamakNotificationsPlugin.NotificationContent,
  _ trigger: TokamakNotificationsPlugin.Trigger?
) throws(TokamakPluginError) -> String {
  do {
    var fields = try object(content)
    if let trigger { fields.merge(try object(trigger)) { _, field in field } }
    return String(decoding: try JSONSerialization.data(withJSONObject: fields), as: UTF8.self)
  } catch {
    throw typeError("a notification's fields must be JSON")
  }
}

private func object(_ value: some Encodable) throws -> [String: Any] {
  try JSONSerialization.jsonObject(with: JSONEncoder().encode(value)) as? [String: Any] ?? [:]
}

private func typeError(_ message: String) -> TokamakPluginError {
  TokamakPluginError(name: "TypeError", message: message)
}
