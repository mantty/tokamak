import Foundation

extension TokamakNotificationsPlugin {
  /// Whether the app may show notifications.
  enum Permission: String, Codable {
    case granted
    case denied
    case prompt
  }

  /// What `requestPermission` asks for. `alert`, `sound` and `badge` are true when nil.
  struct PermissionOptions: Codable {
    let alert: Bool?
    let sound: Bool?
    let badge: Bool?
    let carPlay: Bool?
    let criticalAlert: Bool?
    let provisional: Bool?
    let providesAppNotificationSettings: Bool?
  }

  enum AuthorizationStatus: String, Codable {
    case notDetermined
    case denied
    case authorized
    case provisional
    case ephemeral
  }

  enum AlertStyle: String, Codable {
    case `none`
    case banner
    case alert
  }

  enum ShowPreviews: String, Codable {
    case always
    case whenAuthenticated
    case never
  }

  /// A notification setting's state.
  enum Setting: String, Codable {
    case enabled
    case disabled
    case notSupported
  }

  struct NotificationSettings: Codable {
    let permission: Permission
    let authorizationStatus: AuthorizationStatus
    let alertStyle: AlertStyle
    let showPreviews: ShowPreviews
    let alert: Setting
    let sound: Setting
    let badge: Setting
    let lockScreen: Setting
    let notificationCenter: Setting
    let criticalAlert: Setting
    let timeSensitive: Setting
    let scheduledDelivery: Setting
    let directMessages: Setting
    /// iOS only.
    let carPlay: Setting?
    /// iOS only.
    let announcement: Setting?
    let providesAppNotificationSettings: Bool
  }

  /// Settings of an existing channel `setup` could not change, which Apple platforms never have.
  struct Unapplied: Codable {
    let channel: String
    let fields: [String]
  }

  struct SetupResult: Codable {
    let unapplied: [Unapplied]
  }

  enum PushService: String, Codable {
    case apns
  }

  enum PushEnvironment: String, Codable {
    case development
    case production
  }

  /// Where the app's server sends push messages for this installation.
  struct Subscription: Codable {
    let service: PushService
    /// The device token, in hexadecimal.
    let token: String
    let environment: PushEnvironment
  }

  /// A received push message. A data-only message has no title or body.
  struct Message: Codable {
    let id: String
    let title: String?
    let body: String?
    let data: [String: TokamakJSON]
  }

  enum Source: String, Codable {
    case local
    case push
  }

  struct DismissedNotification: Codable {
    let id: String
    let title: String?
    let body: String?
    let data: [String: TokamakJSON]
    let source: Source
  }

  struct OpenedNotification: Codable {
    let id: String
    let title: String?
    let body: String?
    let data: [String: TokamakJSON]
    let source: Source
    /// The action the user chose, or nil for the notification itself.
    let action: String?
    /// The text the user typed into the action.
    let input: String?
  }

  /// The user asked for the app's notification settings from the system's.
  struct OpenSettingsEvent: Codable {
    /// The id of the notification the user came from.
    let notification: String?
  }

  /// A notification's fields that Apple platforms show.
  struct NotificationContent: Codable {
    /// Showing or scheduling another notification with the same id replaces this one.
    let id: String
    let title: String
    let body: String?
    let data: [String: TokamakJSON]?
    /// A `setup` category, whose actions the notification shows.
    let category: String?
    /// Shows the notifications that name the same group together.
    let group: String?
    /// An image, from an `https` URL or a path the app serves, attached before `attachments`.
    let image: String?
    let attachments: [Attachment]?
    /// The app's badge count once the notification shows.
    let badge: Int?
    /// Shows the notification without sound.
    let silent: Bool?
    let sound: Sound?
    let subtitle: String?
    let interruptionLevel: InterruptionLevel?
    /// From 0 to 1, how the notification ranks in the notification summary.
    let relevanceScore: Double?
  }

  /// The sound a notification plays: the default, the default critical sound, or a file of the
  /// `notifications.sounds` build setting.
  enum Sound: TokamakUnion {
    case `default`
    case defaultCritical
    case named(NamedSound)
  }

  struct NamedSound: Codable {
    /// A file of the `notifications.sounds` build setting, such as `"chime.caf"`.
    let name: String
    /// Plays through the mute switch and Do Not Disturb, with the critical alerts entitlement.
    let critical: Bool?
    /// A critical sound's volume, from 0 to 1.
    let volume: Double?
  }

  /// A file shown with a notification: an image up to 10 MB, audio up to 5 MB or a movie up to
  /// 50 MB.
  struct Attachment: Codable {
    /// An `https` URL or a path the app serves.
    let url: String
    let hideThumbnail: Bool?
    /// The part of an image the thumbnail shows, as fractions of its width and height.
    let thumbnailClippingRect: ClippingRect?
    /// The time, in seconds, of the frame of a movie or animated image the thumbnail shows.
    let thumbnailTime: Double?
  }

  struct ClippingRect: Codable {
    let x: Double
    let y: Double
    let width: Double
    let height: Double
  }

  /// How urgently a notification interrupts; `timeSensitive` and `critical` need their
  /// entitlements.
  enum InterruptionLevel: String, Codable {
    case passive
    case active
    case timeSensitive
    case critical
  }

  /// When a scheduled notification shows.
  enum Trigger: TokamakUnion {
    case at(AtTrigger)
    case every(EveryTrigger)
    case on(DateTrigger)
    case region(RegionTrigger)
  }

  struct AtTrigger: Codable {
    /// Milliseconds since the epoch; the notification shows now once it has passed.
    let at: Double
  }

  struct EveryTrigger: Codable {
    /// Shows the notification once each interval, the first time one interval from now.
    let every: Interval
  }

  /// A repeating interval of at least 60 seconds, the sum of its parts.
  struct Interval: Codable {
    let seconds: Double?
    let minutes: Double?
    let hours: Double?
    let days: Double?
  }

  struct DateTrigger: Codable {
    /// Shows the notification at the next local time that matches.
    let on: DateMatch
    /// Shows it at every match.
    let repeats: Bool?
  }

  /// Parts of a local date and time in the Gregorian calendar; the parts left out match any value.
  struct DateMatch: Codable {
    let year: Int?
    let month: Int?
    let day: Int?
    /// 1 for Sunday to 7 for Saturday.
    let weekday: Int?
    let hour: Int?
    let minute: Int?
    let second: Int?
  }

  /// iOS only: shows the notification when the device enters or leaves `region`.
  struct RegionTrigger: Codable {
    let region: Region
    let repeats: Bool?
  }

  /// A circle on the Earth's surface.
  struct Region: Codable {
    let latitude: Double
    let longitude: Double
    /// In metres.
    let radius: Double
    /// Whether entering shows the notification; true by default.
    let onEntry: Bool?
    /// Whether leaving shows the notification; false by default.
    let onExit: Bool?
  }

  /// A notification not yet shown and its trigger.
  struct ScheduledNotification: Codable {
    let content: NotificationContent
    let trigger: Trigger
  }

  /// The configuration Apple platforms apply.
  struct SetupConfig: Codable {
    let categories: [Category]?
    /// Whether notifications show while the app is on screen, or the parts that do; true by
    /// default.
    let showInForeground: ShowInForeground?
  }

  enum ShowInForeground: TokamakUnion {
    case all(Bool)
    case parts(ForegroundPresentation)
  }

  /// The parts of a notification that show while the app is on screen.
  struct ForegroundPresentation: Codable {
    let banner: Bool?
    let list: Bool?
    let sound: Bool?
    let badge: Bool?
  }

  /// The actions of the notifications that name the category.
  struct Category: Codable {
    let id: String
    let actions: [NotificationAction]
    /// Replaces the body while previews are hidden.
    let hiddenPreviewsBodyPlaceholder: String?
    let hiddenPreviewsShowTitle: Bool?
    let hiddenPreviewsShowSubtitle: Bool?
    /// Summarises a stack of the category's notifications, such as "%u more messages".
    let summaryFormat: String?
    /// Reports dismissals of the category's notifications.
    let customDismissAction: Bool?
    /// iOS only: shows the category's notifications in CarPlay.
    let allowInCarPlay: Bool?
  }

  struct NotificationAction: Codable {
    let id: String
    let title: String
    /// A text field, whose text arrives as the opened notification's `input`.
    let input: ActionInput?
    /// Opens the app; otherwise the action reaches listeners without opening it.
    let foreground: Bool?
    let destructive: Bool?
    /// Requires the device to be unlocked.
    let authenticationRequired: Bool?
    /// An SF Symbol shown with the action.
    let systemImage: String?
  }

  struct ActionInput: Codable {
    /// The send button's title; the action's title by default.
    let button: String?
    let placeholder: String?
  }
}
