import Foundation
import UserNotifications

/// The parts of the configuration `setup` applied that Apple platforms use.
struct TokamakNotificationSetup {
  let categories: Set<UNNotificationCategory>
  /// How notifications show while the app is on screen.
  let presentation: UNNotificationPresentationOptions

  /// The configuration before any `setup`: no categories, and notifications shown in full.
  static let initial = Self(categories: [], presentation: [.banner, .list, .sound, .badge])

  private init(
    categories: Set<UNNotificationCategory>, presentation: UNNotificationPresentationOptions
  ) {
    self.categories = categories
    self.presentation = presentation
  }

  /// Parses the whole configuration, throwing `TypeError` before anything applies it.
  init(_ arguments: Any) throws(TokamakPluginError) {
    guard let config = arguments as? [String: Any] else {
      throw typeError("setup takes a configuration object")
    }
    categories = Set(try list(config["categories"]).map(Self.category))
    presentation = try Self.presentation(config["showInForeground"])
  }

  private static func category(_ value: Any) throws(TokamakPluginError) -> UNNotificationCategory {
    let category = try record(value)
    var options: UNNotificationCategoryOptions = []
    if try flag(category["customDismissAction"]) { options.insert(.customDismissAction) }
    if try flag(category["hiddenPreviewsShowTitle"]) { options.insert(.hiddenPreviewsShowTitle) }
    if try flag(category["hiddenPreviewsShowSubtitle"]) {
      options.insert(.hiddenPreviewsShowSubtitle)
    }
    #if os(iOS)
      if try flag(category["allowInCarPlay"]) { options.insert(.allowInCarPlay) }
    #endif
    return UNNotificationCategory(
      identifier: try text(category["id"]),
      actions: try list(category["actions"]).map(Self.action),
      intentIdentifiers: [],
      hiddenPreviewsBodyPlaceholder: try optionalText(category["hiddenPreviewsBodyPlaceholder"]),
      categorySummaryFormat: try optionalText(category["summaryFormat"]),
      options: options
    )
  }

  private static func action(_ value: Any) throws(TokamakPluginError) -> UNNotificationAction {
    let action = try record(value)
    let id = try text(action["id"])
    let title = try text(action["title"])
    var options: UNNotificationActionOptions = []
    if try flag(action["foreground"]) { options.insert(.foreground) }
    if try flag(action["destructive"]) { options.insert(.destructive) }
    if try flag(action["authenticationRequired"]) { options.insert(.authenticationRequired) }
    let icon = try optionalText(action["systemImage"]).map {
      UNNotificationActionIcon(systemImageName: $0)
    }
    guard let input = action["input"] else {
      return UNNotificationAction(identifier: id, title: title, options: options, icon: icon)
    }
    let field = try record(input)
    return UNTextInputNotificationAction(
      identifier: id,
      title: title,
      options: options,
      icon: icon,
      textInputButtonTitle: try optionalText(field["button"]) ?? title,
      textInputPlaceholder: try optionalText(field["placeholder"]) ?? ""
    )
  }

  /// True shows every part, false none, and an object the parts it names.
  private static func presentation(_ value: Any?) throws(TokamakPluginError)
    -> UNNotificationPresentationOptions
  {
    switch value {
    case nil:
      return initial.presentation
    case let shown as Bool:
      return shown ? initial.presentation : []
    case let parts as [String: Any]:
      var presentation: UNNotificationPresentationOptions = []
      if try flag(parts["banner"]) { presentation.insert(.banner) }
      if try flag(parts["list"]) { presentation.insert(.list) }
      if try flag(parts["sound"]) { presentation.insert(.sound) }
      if try flag(parts["badge"]) { presentation.insert(.badge) }
      return presentation
    default:
      throw typeError("showInForeground must be a boolean or an object")
    }
  }
}

// The plugin's TypeScript validates the configuration first and names the
// invalid value; these checks stop any other caller's from applying in part.

private func list(_ value: Any?) throws(TokamakPluginError) -> [Any] {
  guard let value else { return [] }
  guard let list = value as? [Any] else { throw typeError("a setup list must be an array") }
  return list
}

private func record(_ value: Any) throws(TokamakPluginError) -> [String: Any] {
  guard let record = value as? [String: Any] else {
    throw typeError("a setup entry must be an object")
  }
  return record
}

private func text(_ value: Any?) throws(TokamakPluginError) -> String {
  guard let text = value as? String else { throw typeError("a setup value must be a string") }
  return text
}

private func optionalText(_ value: Any?) throws(TokamakPluginError) -> String? {
  guard let value else { return nil }
  return try text(value)
}

private func flag(_ value: Any?) throws(TokamakPluginError) -> Bool {
  guard let value else { return false }
  guard let flag = value as? Bool else { throw typeError("a setup flag must be a boolean") }
  return flag
}

private func typeError(_ message: String) -> TokamakPluginError {
  TokamakPluginError(name: "TypeError", message: message)
}
