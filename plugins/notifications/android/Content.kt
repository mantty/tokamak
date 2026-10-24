package com.tokamak.plugins.notifications

import android.app.Notification
import android.graphics.Color
import com.tokamak.runtime.TokamakPluginError

/**
 * Throws `TypeError` unless Android can show the notification: its id is not empty and its colour
 * is one Android reads. A Worker's `onPush` reply is checked too, when it arrives.
 */
internal fun NotificationContent.check() {
    if (id.isEmpty()) throw TokamakPluginError.typeError("id must be a non-empty string")
    accentColour
}

/** The accent colour as Android reads it. */
internal val NotificationContent.accentColour: Int?
    get() =
        color?.let {
            runCatching { Color.parseColor(it) }.getOrElse {
                throw TokamakPluginError.typeError("color must be a colour such as #00ff00")
            }
        }

internal val NotificationContent.lockScreenVisibility: Int?
    get() =
        when (visibility) {
            null -> null
            Visibility.Public -> Notification.VISIBILITY_PUBLIC
            Visibility.Private -> Notification.VISIBILITY_PRIVATE
            Visibility.Secret -> Notification.VISIBILITY_SECRET
        }

/** The `Notification.CATEGORY_*` of the notification's `androidCategory`. */
internal val NotificationContent.systemCategory: String?
    get() =
        when (androidCategory) {
            null -> null
            AndroidCategory.Alarm -> Notification.CATEGORY_ALARM
            AndroidCategory.Call -> Notification.CATEGORY_CALL
            AndroidCategory.Email -> Notification.CATEGORY_EMAIL
            AndroidCategory.Error -> Notification.CATEGORY_ERROR
            AndroidCategory.Event -> Notification.CATEGORY_EVENT
            AndroidCategory.LocationSharing -> Notification.CATEGORY_LOCATION_SHARING
            AndroidCategory.Message -> Notification.CATEGORY_MESSAGE
            AndroidCategory.MissedCall -> Notification.CATEGORY_MISSED_CALL
            AndroidCategory.Navigation -> Notification.CATEGORY_NAVIGATION
            AndroidCategory.Progress -> Notification.CATEGORY_PROGRESS
            AndroidCategory.Promo -> Notification.CATEGORY_PROMO
            AndroidCategory.Recommendation -> Notification.CATEGORY_RECOMMENDATION
            AndroidCategory.Reminder -> Notification.CATEGORY_REMINDER
            AndroidCategory.Service -> Notification.CATEGORY_SERVICE
            AndroidCategory.Social -> Notification.CATEGORY_SOCIAL
            AndroidCategory.Status -> Notification.CATEGORY_STATUS
            AndroidCategory.Stopwatch -> Notification.CATEGORY_STOPWATCH
            AndroidCategory.System -> Notification.CATEGORY_SYSTEM
            AndroidCategory.Transport -> Notification.CATEGORY_TRANSPORT
            AndroidCategory.Workout -> Notification.CATEGORY_WORKOUT
        }
