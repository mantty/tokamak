import Foundation
import UserNotifications

/// Keys the plugin stores in a local notification's `userInfo`.
private let atKey = "tokamak.at"
/// Every field of the notification, which `getScheduled` and `getDelivered` return.
private let contentKey = "tokamak.content"
/// The data of a notification scheduled without its content stored.
private let dataKey = "tokamak.data"

/// A notification the page asked to show or schedule.
struct TokamakNotificationRequest {
  let id: String
  let title: String
  let body: String?
  /// A `setup` category, whose actions the notification shows.
  let category: String?
  /// Milliseconds since the epoch; nil to show now.
  let at: Double?
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
    guard scheduled else {
      at = nil
      return
    }
    guard let at = (arguments["at"] as? NSNumber)?.doubleValue, at.isFinite else {
      throw typeError("at must be a number of milliseconds since the epoch")
    }
    self.at = at
  }

  /// The system request, due now when `at` is absent or has passed.
  func request() throws(TokamakPluginError) -> UNNotificationRequest {
    let content = UNMutableNotificationContent()
    content.title = title
    content.body = body ?? ""
    content.subtitle = fields["subtitle"] as? String ?? ""
    content.threadIdentifier = fields["group"] as? String ?? ""
    content.badge = fields["badge"] as? NSNumber
    content.sound = fields["silent"] as? Bool == true ? nil : .default
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
    var userInfo: [String: Any] = [contentKey: try json(fields)]
    var trigger: UNNotificationTrigger?
    if let at, at > Date().timeIntervalSince1970 * 1000 {
      userInfo[atKey] = at
      trigger = UNCalendarNotificationTrigger(dateMatching: components(at), repeats: false)
    }
    content.userInfo = userInfo
    return UNNotificationRequest(identifier: id, content: content, trigger: trigger)
  }

  @available(iOS 15, macOS 12, *)
  private static let interruptionLevels: [String: UNNotificationInterruptionLevel] = [
    "passive": .passive, "active": .active, "timeSensitive": .timeSensitive, "critical": .critical,
  ]

  private func components(_ at: Double) -> DateComponents {
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
      stored.removeValue(forKey: "at")
      return stored
    }
    var content: [String: Any] = ["id": id, "title": title ?? "", "data": data]
    content["body"] = body
    content["category"] = category
    return content
  }
}

extension UNNotificationRequest {
  /// The page's view of a pending request.
  var scheduled: [String: Any] {
    let at =
      content.userInfo[atKey] as? Double
      ?? (trigger as? UNCalendarNotificationTrigger)?.nextTriggerDate().map {
        $0.timeIntervalSince1970 * 1000
      } ?? 0
    if let stored = storedContent(content.userInfo) {
      return stored.merging(["at": at]) { _, at in at }
    }
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
