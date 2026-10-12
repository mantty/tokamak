import Foundation
import UserNotifications

#if os(iOS)
  import CoreLocation
#endif

/// Keys the plugin stores in a local notification's `userInfo`: every field of the notification,
/// which `getScheduled` and `getDelivered` return.
private let contentKey = "tokamak.content"
/// The data of a notification scheduled without its content stored.
private let dataKey = "tokamak.data"

/// Fields of a scheduled notification's trigger, which shown ones leave out.
private let triggerKeys = ["at", "every", "on", "region", "repeats", "exact"]

/// A notification the page asked to show or schedule.
struct TokamakNotificationRequest {
  let id: String
  let title: String
  let body: String?
  /// A `setup` category, whose actions the notification shows.
  let category: String?
  /// When the notification shows; nil to show now.
  let trigger: UNNotificationTrigger?
  /// Every field of the notification, from the page or a Worker's `onPush` reply.
  let fields: [String: Any]

  init(_ arguments: Any, scheduled: Bool) throws(TokamakPluginError) {
    let arguments = arguments as? [String: Any] ?? [:]
    fields = arguments
    guard let id = arguments["id"] as? String, !id.isEmpty else {
      throw typeError("id must be a non-empty string")
    }
    guard let title = arguments["title"] as? String else {
      throw typeError("title must be a string")
    }
    self.id = id
    self.title = title
    body = arguments["body"] as? String
    category = arguments["category"] as? String
    guard category != nil || arguments["category"] == nil else {
      throw typeError("category must be a string")
    }
    trigger = scheduled ? try Self.trigger(arguments, id: id) : nil
  }

  /// The trigger `arguments` give: `at` a time, nil once it has passed; `every` interval; `on`
  /// the dates that match; or, on iOS, a `region`.
  private static func trigger(_ arguments: [String: Any], id: String) throws(TokamakPluginError)
    -> UNNotificationTrigger?
  {
    let repeats = arguments["repeats"] as? Bool ?? false
    if let at = (arguments["at"] as? NSNumber)?.doubleValue {
      guard at.isFinite else {
        throw typeError("at must be a number of milliseconds since the epoch")
      }
      guard at > Date().timeIntervalSince1970 * 1000 else { return nil }
      return UNCalendarNotificationTrigger(dateMatching: utcComponents(at), repeats: false)
    }
    if let every = arguments["every"] as? [String: Any] {
      let seconds = ["seconds": 1.0, "minutes": 60, "hours": 3600, "days": 86400].reduce(0.0) {
        total, unit in total + ((every[unit.key] as? NSNumber)?.doubleValue ?? 0) * unit.value
      }
      guard seconds >= 60 else { throw typeError("every must be at least 60 seconds") }
      return UNTimeIntervalNotificationTrigger(timeInterval: seconds, repeats: true)
    }
    if let on = arguments["on"] as? [String: Any] {
      // Android and the page number years and months as the Gregorian calendar does.
      var components = DateComponents(calendar: Calendar(identifier: .gregorian))
      components.year = on["year"] as? Int
      components.month = on["month"] as? Int
      components.day = on["day"] as? Int
      components.weekday = on["weekday"] as? Int
      components.hour = on["hour"] as? Int
      components.minute = on["minute"] as? Int
      components.second = on["second"] as? Int
      let trigger = UNCalendarNotificationTrigger(dateMatching: components, repeats: repeats)
      guard trigger.nextTriggerDate() != nil else { throw typeError("on matches no time from now") }
      return trigger
    }
    if let region = arguments["region"] as? [String: Any] {
      return try regionTrigger(region, id: id, repeats: repeats)
    }
    throw typeError("a scheduled notification must have one of at, every, on or region")
  }

  #if os(iOS)
    private static func regionTrigger(_ region: [String: Any], id: String, repeats: Bool)
      throws(TokamakPluginError) -> UNNotificationTrigger
    {
      let status = CLLocationManager().authorizationStatus
      guard status == .authorizedWhenInUse || status == .authorizedAlways else {
        throw TokamakPluginError(
          name: "NotAllowedError", message: "A region trigger needs location permission")
      }
      guard let latitude = region["latitude"] as? Double,
        let longitude = region["longitude"] as? Double,
        let radius = region["radius"] as? Double
      else {
        throw typeError("region must have a latitude, longitude and radius")
      }
      let circle = CLCircularRegion(
        center: CLLocationCoordinate2D(latitude: latitude, longitude: longitude), radius: radius,
        identifier: id)
      circle.notifyOnEntry = region["onEntry"] as? Bool ?? true
      circle.notifyOnExit = region["onExit"] as? Bool ?? false
      return UNLocationNotificationTrigger(region: circle, repeats: repeats)
    }
  #else
    private static func regionTrigger(_ region: [String: Any], id: String, repeats: Bool)
      throws(TokamakPluginError) -> UNNotificationTrigger
    {
      throw .notSupported("Region triggers are iOS only")
    }
  #endif

  /// The system request, due now when `at` is absent or has passed.
  func request(attachments: [UNNotificationAttachment]) throws(TokamakPluginError)
    -> UNNotificationRequest
  {
    let content = UNMutableNotificationContent()
    content.title = title
    content.attachments = attachments
    content.body = body ?? ""
    content.subtitle = fields["subtitle"] as? String ?? ""
    content.threadIdentifier = fields["group"] as? String ?? ""
    content.badge = fields["badge"] as? NSNumber
    content.sound = fields["silent"] as? Bool == true ? nil : try Self.sound(fields["sound"])
    content.categoryIdentifier = category ?? ""
    if #available(iOS 15, macOS 12, *) {
      if let level = (fields["interruptionLevel"] as? String).flatMap({
        Self.interruptionLevels[$0]
      }) {
        content.interruptionLevel = level
      }
      if let score = fields["relevanceScore"] as? Double {
        content.relevanceScore = score
      }
    }
    content.userInfo = [contentKey: try json(fields)]
    return UNNotificationRequest(identifier: id, content: content, trigger: trigger)
  }

  /// The sound `value` names: the default, the default critical sound, or a file the app bundles
  /// from the `sounds` build setting.
  private static func sound(_ value: Any?) throws(TokamakPluginError) -> UNNotificationSound {
    switch value {
    case nil, "default" as String:
      return .default
    case "defaultCritical" as String:
      return .defaultCritical
    case let sound as [String: Any]:
      guard let name = sound["name"] as? String else {
        throw typeError("sound.name must be a string")
      }
      guard Bundle.main.url(forResource: name, withExtension: nil) != nil else {
        throw TokamakPluginError(
          name: "NotFoundError",
          message: "The app has no sound \(name); add it to the notifications.sounds build setting")
      }
      let named = UNNotificationSoundName(name)
      guard sound["critical"] as? Bool == true else { return UNNotificationSound(named: named) }
      let volume = (sound["volume"] as? NSNumber)?.floatValue ?? 1
      return .criticalSoundNamed(named, withAudioVolume: volume)
    default:
      throw typeError("sound must be \"default\", \"defaultCritical\" or an object")
    }
  }

  @available(iOS 15, macOS 12, *)
  private static let interruptionLevels: [String: UNNotificationInterruptionLevel] = [
    "passive": .passive, "active": .active, "timeSensitive": .timeSensitive, "critical": .critical,
  ]

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
  let data: [String: Any]
  let category: String?
  let isPush: Bool
  /// Every field of a local notification, if it has them stored.
  let stored: [String: Any]?

  init(_ notification: UNNotification) {
    let content = notification.request.content
    id = notification.request.identifier
    title = content.title.isEmpty ? nil : content.title
    body = content.body.isEmpty ? nil : content.body
    category = content.categoryIdentifier.isEmpty ? nil : content.categoryIdentifier
    (data, isPush) = Self.data(content.userInfo)
    stored = isPush ? nil : storedContent(content.userInfo)
  }

  /// A push message delivered to the app without a notification.
  init(remoteNotification userInfo: [AnyHashable: Any]) {
    let alert = (userInfo["aps"] as? [String: Any])?["alert"]
    id = UUID().uuidString
    title = (alert as? [String: Any])?["title"] as? String
    body = (alert as? [String: Any])?["body"] as? String ?? alert as? String
    category = (userInfo["aps"] as? [String: Any])?["category"] as? String
    (data, isPush) = Self.data(userInfo)
    stored = nil
  }

  /// A push message's data is every top-level key but `aps`; a local
  /// notification's is the data the page gave it.
  static func data(_ userInfo: [AnyHashable: Any]) -> ([String: Any], isPush: Bool) {
    if userInfo["aps"] != nil {
      var data: [String: Any] = [:]
      for case (let key as String, let value) in userInfo where key != "aps" {
        data[key] = value
      }
      return (data, true)
    }
    if let stored = storedContent(userInfo) {
      return (stored["data"] as? [String: Any] ?? [:], false)
    }
    let json = (userInfo[dataKey] as? String)?.data(using: .utf8) ?? Data()
    return ((try? JSONSerialization.jsonObject(with: json)) as? [String: Any] ?? [:], false)
  }

  var message: [String: Any] {
    ["id": id, "title": title ?? NSNull(), "body": body ?? NSNull(), "data": data]
  }

  var dismissed: [String: Any] {
    message.merging(["source": isPush ? "push" : "local"]) { current, _ in current }
  }

  func opened(action: String?, input: String?) -> [String: Any] {
    dismissed.merging(["action": action ?? NSNull(), "input": input ?? NSNull()]) {
      current, _ in current
    }
  }

  var content: [String: Any] {
    if var stored {
      for key in triggerKeys { stored.removeValue(forKey: key) }
      return stored
    }
    var content: [String: Any] = ["id": id, "title": title ?? "", "data": data]
    content["body"] = body
    content["category"] = category
    return content
  }
}

extension UNNotificationRequest {
  /// The page's view of a pending request: its fields, which name its trigger.
  var scheduled: [String: Any] {
    if let stored = storedContent(content.userInfo) {
      return stored
    }
    let at =
      (trigger as? UNCalendarNotificationTrigger)?.nextTriggerDate().map {
        $0.timeIntervalSince1970 * 1000
      } ?? 0
    var scheduled: [String: Any] = [
      "id": identifier,
      "title": content.title,
      "data": TokamakNotificationFields.data(content.userInfo).0,
      "at": at,
    ]
    scheduled["body"] = content.body.isEmpty ? nil : content.body
    scheduled["category"] = content.categoryIdentifier.isEmpty ? nil : content.categoryIdentifier
    return scheduled
  }
}

/// Every field of a local notification, if it has them stored.
private func storedContent(_ userInfo: [AnyHashable: Any]) -> [String: Any]? {
  guard let json = (userInfo[contentKey] as? String)?.data(using: .utf8) else { return nil }
  return (try? JSONSerialization.jsonObject(with: json)) as? [String: Any]
}

private func json(_ value: [String: Any]) throws(TokamakPluginError) -> String {
  guard
    JSONSerialization.isValidJSONObject(value),
    let data = try? JSONSerialization.data(withJSONObject: value)
  else {
    throw typeError("a notification's fields must be JSON")
  }
  return String(decoding: data, as: UTF8.self)
}

private func typeError(_ message: String) -> TokamakPluginError {
  TokamakPluginError(name: "TypeError", message: message)
}
