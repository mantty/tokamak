package com.tokamak.plugins.notifications

import com.tokamak.runtime.TokamakExporter
import com.tokamak.runtime.TokamakExports

object NotificationsExports : TokamakExports<TokamakNotificationsPlugin> {
    override fun exports(e: TokamakExporter<TokamakNotificationsPlugin>) {
        e.call(TokamakNotificationsPlugin::setup, doc = "Applies the configuration of every platform, replacing the previous one.")
        e.call(TokamakNotificationsPlugin::permission, doc = "Whether the app may show notifications.")
        e.call(TokamakNotificationsPlugin::requestPermission, doc = "Shows the system's permission prompt where Android still shows it.")
        e.call(TokamakNotificationsPlugin::settings, doc = "The user's notification settings.")
        e.call(TokamakNotificationsPlugin::openSettings, doc = "Opens the app's notification settings, a channel's, or a special access page.")
        e.call(TokamakNotificationsPlugin::show, doc = "Shows a notification now.")
        e.call(TokamakNotificationsPlugin::schedule, doc = "Shows a notification when its trigger fires.")
        e.call(TokamakNotificationsPlugin::getScheduled, doc = "Scheduled notifications not yet shown.")
        e.call(TokamakNotificationsPlugin::getDelivered, doc = "Shown notifications still in the notification drawer.")
        e.call(TokamakNotificationsPlugin::remove, doc = "Removes a scheduled or shown notification.")
        e.call(TokamakNotificationsPlugin::subscribeToPush, doc = "Registers for push messages, resolving to the subscription.")
        e.call(TokamakNotificationsPlugin::getSubscription, doc = "The push subscription, or null.")
        e.call(TokamakNotificationsPlugin::unsubscribe, doc = "Stops push delivery.")
        e.call(TokamakNotificationsPlugin::subscribeToTopic, doc = "Receives the push messages the app's server sends to an FCM topic.")
        e.call(TokamakNotificationsPlugin::unsubscribeFromTopic)
        e.stream(TokamakNotificationsPlugin::onMessage, doc = "Push messages that arrive while the listener runs.")
        e.stream(TokamakNotificationsPlugin::onNotificationOpened, doc = "Opened notifications, including the one that opened the app.")
        e.stream(TokamakNotificationsPlugin::onNotificationDismissed, doc = "Dismissed notifications.")
        e.stream(TokamakNotificationsPlugin::onOpenSettings, doc = "The user's requests, from the system's settings, for the app's own.")
        e.stream(TokamakNotificationsPlugin::onSubscriptionChange, doc = "The replacement when FCM changes the Firebase Installation ID.")
        e.event(
            "push",
            Message::class,
            reply = NotificationContent::class,
            doc = "Receives each data-only push message, and shows the notification a listener returns.",
        )
        e.event("opened", OpenedNotification::class, doc = "Receives each notification the user opens, including one that opens the app.")
        e.event("dismissed", DismissedNotification::class, doc = "Receives each notification the user dismisses.")
    }
}
