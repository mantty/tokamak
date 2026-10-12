package com.tokamak.plugins.notifications

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Person
import android.app.RemoteInput
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.drawable.Icon
import android.os.Bundle
import android.service.notification.StatusBarNotification
import com.tokamak.runtime.TokamakPluginError
import com.tokamak.runtime.tokamakPlugin
import org.json.JSONArray
import org.json.JSONObject

internal const val CHANNEL = "tokamak.notifications"
internal const val OPENED_EXTRA = "com.tokamak.notifications.opened"
internal const val DISMISSED_EXTRA = "com.tokamak.notifications.dismissed"
internal const val GROUP_EXTRA = "com.tokamak.notifications.group"
private const val CONTENT_EXTRA = "com.tokamak.notifications.content"
private const val INPUT_KEY = "com.tokamak.notifications.input"

/** The plugin and FCM post every notification with this ID, telling them apart by tag. */
private const val NOTIFICATION_ID = 0

/** The plugin posts each group's summary with this ID, tagged with the group. */
private const val SUMMARY_ID = 1

/** The group of silent notifications that name none. */
private const val SILENT_GROUP = "tokamak.silent"

/** Posts, lists and removes the app's notifications, and keeps its channels as `setup` describes. */
internal class Notifier(private val context: Context) {
    private val manager = context.getSystemService(NotificationManager::class.java)

    val enabled: Boolean
        get() = manager.areNotificationsEnabled()

    /** Creates the default channel, named after the app, unless it exists. */
    fun ensureDefaultChannel() {
        if (manager.getNotificationChannel(CHANNEL) == null) manager.createNotificationChannel(appChannel())
    }

    /**
     * Makes the channel groups and channels [setup] describes, removing those of [previous] it
     * leaves out, and returns the settings of existing channels Android kept.
     */
    fun configure(setup: Setup, previous: Setup?): JSONArray {
        val groups = setup.channelGroups.map { it.id }.toSet()
        previous?.channelGroups?.map { it.id }?.filterNot(groups::contains)?.forEach(manager::deleteNotificationChannelGroup)
        manager.createNotificationChannelGroups(setup.channelGroups)
        val channels = setup.channels.map { it.id }.toSet()
        previous?.channels?.map { it.id }?.filterNot(channels::contains)?.forEach(manager::deleteNotificationChannel)
        if (setup.defaultChannel == null) manager.createNotificationChannel(appChannel())
        val unapplied = JSONArray()
        for (spec in listOfNotNull(setup.defaultChannel) + setup.channels) {
            manager.createNotificationChannel(spec.channel(context))
            val kept = spec.unapplied(manager.getNotificationChannel(spec.id), context)
            if (kept.isNotEmpty()) unapplied.put(JSONObject().put("channel", spec.id).put("fields", JSONArray(kept)))
        }
        return unapplied
    }

    /** Throws `TypeError` unless [content] names a channel that exists. */
    fun requireChannel(content: Content) {
        val channel = content.channel ?: return
        if (manager.getNotificationChannel(channel) == null) {
            throw TokamakPluginError.typeError("channel $channel is not a setup channel")
        }
    }

    /**
     * Shows [content] with [images], the bitmaps of its [imageSources] that loaded, replacing a
     * notification with the same ID. Opening it launches the app. A channel `setup` has since
     * removed falls back to the default.
     */
    fun post(content: Content, source: String, images: Map<String, Bitmap>) {
        ensureDefaultChannel()
        val channel = content.channel?.takeIf { manager.getNotificationChannel(it) != null } ?: CHANNEL
        val described = described(content, source)
        val opened = Intent().putExtra(OPENED_EXTRA, JSONObject(described.toString()).put("action", JSONObject.NULL).toString())
        val dismissed = Intent().putExtra(DISMISSED_EXTRA, described.toString())
        content.group?.let {
            // Opening one removes it only with `autoCancel`.
            if (content.autoCancel) opened.putExtra(GROUP_EXTRA, it)
            dismissed.putExtra(GROUP_EXTRA, it)
        }
        val builder =
            Notification.Builder(context, channel)
                .setSmallIcon(icon())
                .setContentTitle(content.title)
                .setContentText(content.body)
                .setContentIntent(launch("opened\n${content.id}", opened, mutable = false))
                .setDeleteIntent(respond("dismissed\n${content.id}", dismissed))
                .addExtras(Bundle().apply { putString(CONTENT_EXTRA, content.json.toString()) })
                .setFields(content, channel, images)
        for (action in actions(content)) builder.addAction(action(content, source, action, images[action.icon]))
        val previousGroup = shown(content.id)?.notification?.group
        manager.notify(content.id, NOTIFICATION_ID, builder.build())
        content.group?.let { postSummary(it, channel) }
        if (previousGroup != null && previousGroup != content.group) removeSummaryIfEmpty(previousGroup, content.id)
    }

    /** The images [content] and its category's actions show: its `image` only with the big picture style. */
    fun imageSources(content: Content): List<String> {
        val image = content.image.takeIf { content.style == null || content.style is Style.BigPicture }
        return (listOfNotNull(image, content.icon) + actions(content).mapNotNull { it.icon }).distinct()
    }

    private fun actions(content: Content): List<Action> =
        content.category?.let { Setup.stored(context)?.categories?.get(it) }.orEmpty()

    /** Sets what [content] says beyond its text, actions and intents. */
    private fun Notification.Builder.setFields(
        content: Content,
        channel: String,
        images: Map<String, Bitmap>,
    ): Notification.Builder {
        setAutoCancel(content.autoCancel)
        setOngoing(content.ongoing)
        setOnlyAlertOnce(content.onlyAlertOnce)
        content.subText?.let(::setSubText)
        content.color?.let(::setColor)
        content.badge?.let(::setNumber)
        content.timestamp?.let { setWhen(it).setShowWhen(true) }
        if (content.chronometer) setUsesChronometer(true).setChronometerCountDown(content.chronometerCountsDown)
        content.timeoutAfter?.let(::setTimeoutAfter)
        content.sortKey?.let(::setSortKey)
        content.visibility?.let(::setVisibility)
        content.androidCategory?.let(::setCategory)
        content.progress?.let { setProgress(it.max, it.current, it.indeterminate) }
        content.publicVersion?.let { (title, body) ->
            setPublicVersion(Notification.Builder(context, channel).setSmallIcon(icon()).setContentTitle(title).setContentText(body).build())
        }
        content.icon?.let(images::get)?.let(::setLargeIcon)
        style(content.style, content.image?.let(images::get))?.let(::setStyle)
        // Silent as AndroidX's setSilent makes a notification: one that leaves alerting to its
        // group's summary does not alert, and the plugin's summaries never alert.
        if (content.silent) {
            setGroup(content.group ?: SILENT_GROUP).setGroupAlertBehavior(Notification.GROUP_ALERT_SUMMARY)
        } else {
            content.group?.let(::setGroup)
        }
        return this
    }

    /** Posts the summary Android shows [group]'s notifications under, unless it has one; it never alerts. */
    private fun postSummary(group: String, channel: String) {
        if (manager.activeNotifications.any { it.id == SUMMARY_ID && it.tag == group }) return
        val summary =
            Notification.Builder(context, channel)
                .setSmallIcon(icon())
                .setGroup(group)
                .setGroupSummary(true)
                .setGroupAlertBehavior(Notification.GROUP_ALERT_CHILDREN)
                .build()
        manager.notify(group, SUMMARY_ID, summary)
    }

    /**
     * Removes [group]'s summary once [removed] was its last notification. Android may still list
     * [removed] while it cancels it, so it does not count.
     */
    fun removeSummaryIfEmpty(group: String, removed: String) {
        val others =
            manager.activeNotifications.any {
                it.id == NOTIFICATION_ID && it.tag != removed && it.notification.group == group
            }
        if (!others) manager.cancel(group, SUMMARY_ID)
    }

    /** The style Android shows: [given], or by default the big picture style when there is an [image]. */
    private fun style(given: Style?, image: Bitmap?): Notification.Style? =
        when (val style = given ?: Style.BigPicture(title = null, summary = null, showWhenCollapsed = false)) {
            is Style.BigPicture ->
                image?.let {
                    Notification.BigPictureStyle()
                        .bigPicture(it)
                        .setBigContentTitle(style.title)
                        .showBigPictureWhenCollapsed(style.showWhenCollapsed)
                        // Even a null summary replaces the expanded notification's text.
                        .apply { style.summary?.let(::setSummaryText) }
                }
            is Style.BigText ->
                Notification.BigTextStyle().bigText(style.text).setBigContentTitle(style.title).setSummaryText(style.summary)
            is Style.Inbox ->
                Notification.InboxStyle()
                    .setBigContentTitle(style.title)
                    .setSummaryText(style.summary)
                    .apply { style.lines.forEach(::addLine) }
            is Style.Messaging ->
                Notification.MessagingStyle(person(style.user)).apply {
                    style.conversationTitle?.let {
                        setConversationTitle(it)
                        setGroupConversation(true)
                    }
                    for (message in style.messages) {
                        addMessage(Notification.MessagingStyle.Message(message.text, message.at, message.sender?.let(::person)))
                    }
                }
        }

    private fun person(name: String): Person = Person.Builder().setName(name).build()

    /** The app's shown notifications, identified by tag, as the page sees them. */
    fun delivered(): List<JSONObject> =
        manager.activeNotifications.mapNotNull { shown ->
            val id = shown.tag?.takeIf { shown.id == NOTIFICATION_ID } ?: return@mapNotNull null
            val extras = shown.notification.extras
            // FCM posts notification messages itself, without the content the plugin keeps.
            extras.getString(CONTENT_EXTRA)?.let { JSONObject(it).apply { remove("at") } }
                ?: JSONObject()
                    .put("id", id)
                    .put("title", extras.getCharSequence(Notification.EXTRA_TITLE)?.toString() ?: "")
                    .put("body", extras.getCharSequence(Notification.EXTRA_TEXT)?.toString())
                    .put("data", JSONObject())
                    .put("channel", shown.notification.channelId.takeIf { it != CHANNEL })
        }

    fun remove(id: String) {
        val group = shown(id)?.notification?.group
        manager.cancel(id, NOTIFICATION_ID)
        group?.let { removeSummaryIfEmpty(it, id) }
    }

    private fun shown(id: String): StatusBarNotification? =
        manager.activeNotifications.firstOrNull { it.id == NOTIFICATION_ID && it.tag == id }

    /**
     * An action that launches the app when it has `foreground`, or otherwise reaches the plugin
     * without opening it. An action with a text field passes the text it receives; wearables show
     * [icon].
     */
    private fun action(content: Content, source: String, action: Action, icon: Bitmap?): Notification.Action {
        val opened = Intent().putExtra(OPENED_EXTRA, described(content, source).put("action", action.id).toString())
        val identifier = "action\n${content.id}\n${action.id}"
        val mutable = action.inputLabel != null
        val intent = if (action.foreground) launch(identifier, opened, mutable) else respond(identifier, opened, mutable)
        val builder = Notification.Action.Builder(icon?.let(Icon::createWithBitmap), action.title, intent)
        action.inputLabel?.let { builder.addRemoteInput(RemoteInput.Builder(INPUT_KEY).setLabel(it).build()) }
        return builder.build()
    }

    /** The app's monochrome `drawable/tokamak_notification`, or its launcher icon. */
    private fun icon(): Int =
        context.resources
            .getIdentifier("tokamak_notification", "drawable", context.packageName)
            .takeIf { it != 0 }
            ?: context.applicationInfo.icon.takeIf { it != 0 }
            ?: android.R.drawable.sym_def_app_icon

    /** Launches the app with [response]'s extras. */
    private fun launch(identifier: String, response: Intent, mutable: Boolean): PendingIntent {
        val intent =
            requireNotNull(context.packageManager.getLaunchIntentForPackage(context.packageName))
                .setIdentifier(identifier)
                .putExtras(response)
        return PendingIntent.getActivity(context, 0, intent, flags(mutable))
    }

    /** Delivers [response] to [TokamakResponseReceiver] without opening the app. */
    private fun respond(identifier: String, response: Intent, mutable: Boolean = false): PendingIntent {
        val intent =
            Intent(context, TokamakResponseReceiver::class.java)
                .setIdentifier(identifier)
                .putExtras(response)
        return PendingIntent.getBroadcast(context, 0, intent, flags(mutable))
    }

    /** Text fields fill in their intent, which therefore must be mutable. */
    private fun flags(mutable: Boolean): Int =
        PendingIntent.FLAG_UPDATE_CURRENT or if (mutable) PendingIntent.FLAG_MUTABLE else PendingIntent.FLAG_IMMUTABLE

    /** The default channel, named after the app. */
    private fun appChannel(): NotificationChannel =
        NotificationChannel(
            CHANNEL,
            context.applicationInfo.loadLabel(context.packageManager),
            NotificationManager.IMPORTANCE_DEFAULT,
        )

    /** [content] as listeners receive it, before the plugin adds the chosen action and typed text. */
    private fun described(content: Content, source: String): JSONObject =
        JSONObject()
            .put("id", content.id)
            .put("title", content.title)
            .put("body", content.body ?: JSONObject.NULL)
            .put("data", content.data)
            .put("source", source)

    companion object {
        /** The text typed into the action that sent [intent], if any. */
        fun input(intent: Intent): String? =
            RemoteInput.getResultsFromIntent(intent)?.getCharSequence(INPUT_KEY)?.toString()
    }
}

/** Receives actions that do not open the app, and dismissed notifications. */
class TokamakResponseReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val plugin = context.tokamakPlugin("notifications") as? TokamakNotificationsPlugin ?: return
        val pending = goAsync()
        plugin.respond(intent, pending::finish)
    }
}
