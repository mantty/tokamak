import TokamakPlugin

extension TokamakNotificationsPlugin: TokamakExports {
  public static func exports(_ e: inout TokamakExporter<TokamakNotificationsPlugin>) {
    e.call(setup, doc: "Applies the configuration of every platform, replacing the previous one.")
    e.call(permission, doc: "Whether the app may show notifications.")
    e.call(requestPermission, doc: "Asks for what `options` name where the system has not asked.")
    e.call(settings, doc: "The user's notification settings.")
    e.call(setBadge, doc: "Sets the app's badge count, or removes the badge with 0.")
    e.call(openSettings, doc: "Opens the app's page of the system's notification settings.")
    e.call(show, doc: "Shows a notification now.")
    e.call(schedule, doc: "Shows a notification when its trigger fires.")
    e.call(getScheduled, doc: "Scheduled notifications not yet shown.")
    e.call(getDelivered, doc: "Shown notifications still in Notification Center.")
    e.call(remove, doc: "Removes a scheduled or shown notification.")
    e.call(subscribeToPush, doc: "Registers for push messages, resolving to the subscription.")
    e.call(getSubscription, doc: "The push subscription, or null.")
    e.call(unsubscribe, doc: "Stops push delivery.")
    e.stream(onMessage, doc: "Push messages that arrive while the listener runs.")
    e.stream(
      onNotificationOpened, doc: "Opened notifications, including the one that opened the app.")
    e.stream(
      onNotificationDismissed, doc: "Dismissed notifications of categories that report them.")
    e.stream(
      onOpenSettings, doc: "The user's requests, from the system's settings, for the app's own.")
    e.stream(onSubscriptionChange, doc: "The replacement when APNs changes the subscription.")
    e.event(
      "push", Message.self, reply: NotificationContent.self,
      doc:
        "Receives each data-only push message, and shows the notification a listener returns."
    )
    e.event(
      "opened", OpenedNotification.self,
      doc: "Receives each notification the user opens, including one that opens the app.")
    e.event(
      "dismissed", DismissedNotification.self,
      doc:
        "Receives each notification the user dismisses, of categories with `customDismissAction`.")
  }
}
