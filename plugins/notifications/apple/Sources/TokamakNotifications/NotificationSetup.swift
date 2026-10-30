import Foundation
import TokamakPlugin
import UserNotifications

/// The parts of the configuration `setup` applied that Apple platforms use.
struct TokamakNotificationSetup {
  typealias Plugin = TokamakNotificationsPlugin

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

  init(_ config: Plugin.SetupConfig) {
    categories = Set((config.categories ?? []).map(Self.category))
    presentation = Self.presentation(config.showInForeground)
  }

  private static func category(_ category: Plugin.Category) -> UNNotificationCategory {
    var options: UNNotificationCategoryOptions = []
    if category.customDismissAction == true { options.insert(.customDismissAction) }
    if category.hiddenPreviewsShowTitle == true { options.insert(.hiddenPreviewsShowTitle) }
    if category.hiddenPreviewsShowSubtitle == true { options.insert(.hiddenPreviewsShowSubtitle) }
    #if os(iOS)
      if category.allowInCarPlay == true { options.insert(.allowInCarPlay) }
    #endif
    return UNNotificationCategory(
      identifier: category.id,
      actions: category.actions.map(Self.action),
      intentIdentifiers: [],
      hiddenPreviewsBodyPlaceholder: category.hiddenPreviewsBodyPlaceholder,
      categorySummaryFormat: category.summaryFormat,
      options: options
    )
  }

  private static func action(_ action: Plugin.NotificationAction) -> UNNotificationAction {
    var options: UNNotificationActionOptions = []
    if action.foreground == true { options.insert(.foreground) }
    if action.destructive == true { options.insert(.destructive) }
    if action.authenticationRequired == true { options.insert(.authenticationRequired) }
    let icon = action.systemImage.map { UNNotificationActionIcon(systemImageName: $0) }
    guard let input = action.input else {
      return UNNotificationAction(
        identifier: action.id, title: action.title, options: options, icon: icon)
    }
    return UNTextInputNotificationAction(
      identifier: action.id,
      title: action.title,
      options: options,
      icon: icon,
      textInputButtonTitle: input.button ?? action.title,
      textInputPlaceholder: input.placeholder ?? ""
    )
  }

  /// True shows every part, false none, and an object the parts it names.
  private static func presentation(_ shown: Plugin.ShowInForeground?)
    -> UNNotificationPresentationOptions
  {
    switch shown {
    case nil, .all(true):
      return initial.presentation
    case .all(false):
      return []
    case .parts(let parts):
      var presentation: UNNotificationPresentationOptions = []
      if parts.banner == true { presentation.insert(.banner) }
      if parts.list == true { presentation.insert(.list) }
      if parts.sound == true { presentation.insert(.sound) }
      if parts.badge == true { presentation.insert(.badge) }
      return presentation
    }
  }
}
