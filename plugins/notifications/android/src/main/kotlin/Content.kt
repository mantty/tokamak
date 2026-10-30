package com.tokamak.plugins.notifications

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
