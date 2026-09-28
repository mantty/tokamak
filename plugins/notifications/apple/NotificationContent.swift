import Foundation
import UserNotifications

/// Keys the plugin stores in a local notification's `userInfo`.
private let dataKey = "tokamak.data"
private let atKey = "tokamak.at"

/// A notification the page asked to show or schedule.
struct TokamakNotificationRequest {
  let id: String
  let title: String
  let body: String?
  let data: [String: Any]
  /// Milliseconds since the epoch; nil to show now.
  let at: Double?

  init(_ arguments: Any, scheduled: Bool) throws(TokamakPluginError) {
    let arguments = arguments as? [String: Any] ?? [:]
    guard let id = arguments["id"] as? String, !id.isEmpty else {
      throw typeError("id must be a non-empty string")
    }
    guard let title = arguments["title"] as? String else {
      throw typeError("title must be a string")
    }
    self.id = id
    self.title = title
    body = arguments["body"] as? String
    data = arguments["data"] as? [String: Any] ?? [:]
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
    content.sound = .default
    var userInfo: [String: Any] = [dataKey: try json(data)]
    var trigger: UNNotificationTrigger?
    if let at, at > Date().timeIntervalSince1970 * 1000 {
      userInfo[atKey] = at
      trigger = UNCalendarNotificationTrigger(dateMatching: components(at), repeats: false)
    }
    content.userInfo = userInfo
    return UNNotificationRequest(identifier: id, content: content, trigger: trigger)
  }

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
  let isPush: Bool

  init(_ notification: UNNotification) {
    let content = notification.request.content
    id = notification.request.identifier
    title = content.title.isEmpty ? nil : content.title
    body = content.body.isEmpty ? nil : content.body
    (data, isPush) = Self.data(content.userInfo)
  }

  /// A push message delivered to the app without a notification.
  init(remoteNotification userInfo: [AnyHashable: Any]) {
    let alert = (userInfo["aps"] as? [String: Any])?["alert"]
    id = UUID().uuidString
    title = (alert as? [String: Any])?["title"] as? String
    body = (alert as? [String: Any])?["body"] as? String ?? alert as? String
    (data, isPush) = Self.data(userInfo)
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
    let json = (userInfo[dataKey] as? String)?.data(using: .utf8) ?? Data()
    return ((try? JSONSerialization.jsonObject(with: json)) as? [String: Any] ?? [:], false)
  }

  var message: [String: Any] {
    ["id": id, "title": title ?? NSNull(), "body": body ?? NSNull(), "data": data]
  }

  var opened: [String: Any] {
    message.merging(["source": isPush ? "push" : "local"]) { current, _ in current }
  }

  var content: [String: Any] {
    var content: [String: Any] = ["id": id, "title": title ?? "", "data": data]
    content["body"] = body
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
    var scheduled: [String: Any] = [
      "id": identifier,
      "title": content.title,
      "data": TokamakNotificationFields.data(content.userInfo).0,
      "at": at,
    ]
    scheduled["body"] = content.body.isEmpty ? nil : content.body
    return scheduled
  }
}

private func json(_ value: [String: Any]) throws(TokamakPluginError) -> String {
  guard
    JSONSerialization.isValidJSONObject(value),
    let data = try? JSONSerialization.data(withJSONObject: value)
  else {
    throw typeError("data must be a JSON object")
  }
  return String(decoding: data, as: UTF8.self)
}

private func typeError(_ message: String) -> TokamakPluginError {
  TokamakPluginError(name: "TypeError", message: message)
}
