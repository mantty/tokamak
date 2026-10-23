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
            Visibility.`public` -> Notification.VISIBILITY_PUBLIC
            Visibility.`private` -> Notification.VISIBILITY_PRIVATE
            Visibility.secret -> Notification.VISIBILITY_SECRET
        }

/** The `Notification.CATEGORY_*` of the notification's `androidCategory`. */
internal val NotificationContent.systemCategory: String?
    get() =
        when (androidCategory) {
            null -> null
            AndroidCategory.alarm -> Notification.CATEGORY_ALARM
            AndroidCategory.call -> Notification.CATEGORY_CALL
            AndroidCategory.email -> Notification.CATEGORY_EMAIL
            AndroidCategory.error -> Notification.CATEGORY_ERROR
            AndroidCategory.event -> Notification.CATEGORY_EVENT
            AndroidCategory.locationSharing -> Notification.CATEGORY_LOCATION_SHARING
            AndroidCategory.message -> Notification.CATEGORY_MESSAGE
            AndroidCategory.missedCall -> Notification.CATEGORY_MISSED_CALL
            AndroidCategory.navigation -> Notification.CATEGORY_NAVIGATION
            AndroidCategory.progress -> Notification.CATEGORY_PROGRESS
            AndroidCategory.promo -> Notification.CATEGORY_PROMO
            AndroidCategory.recommendation -> Notification.CATEGORY_RECOMMENDATION
            AndroidCategory.reminder -> Notification.CATEGORY_REMINDER
            AndroidCategory.service -> Notification.CATEGORY_SERVICE
            AndroidCategory.social -> Notification.CATEGORY_SOCIAL
            AndroidCategory.status -> Notification.CATEGORY_STATUS
            AndroidCategory.stopwatch -> Notification.CATEGORY_STOPWATCH
            AndroidCategory.system -> Notification.CATEGORY_SYSTEM
            AndroidCategory.transport -> Notification.CATEGORY_TRANSPORT
            AndroidCategory.workout -> Notification.CATEGORY_WORKOUT
        }
