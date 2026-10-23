package com.tokamak.plugins.notifications

import android.Manifest
import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.SystemClock
import android.provider.Settings
import android.util.Log
import com.google.android.gms.tasks.Task
import com.google.android.gms.tasks.Tasks
import com.google.firebase.FirebaseApp
import com.google.firebase.installations.FirebaseInstallations
import com.google.firebase.messaging.FirebaseMessaging
import com.tokamak.runtime.TokamakHost
import com.tokamak.runtime.TokamakListeners
import com.tokamak.runtime.TokamakPlugin
import com.tokamak.runtime.TokamakPluginError
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.suspendCancellableCoroutine
import org.json.JSONObject
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.TimeoutException
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

private const val PREFERENCES = "tokamak.notifications"
private const val SUBSCRIBED = "subscribed"
/** The Firebase Installation ID (FID) FCM sends to. */
private const val FID = "fid"

/** FCM allows about 10 seconds for a message, including starting the app. */
private const val PUSH_DEADLINE_MILLIS = 8_000L

/** How long a notification's images may take to load, within the 10 seconds Android allows a broadcast receiver. */
private const val IMAGE_TIMEOUT_MILLIS = 8_000L

/** How long posting a notification may take once its images load. */
private const val POST_TIMEOUT_MILLIS = 1_000L

/** How long `subscribeToPush`, `unsubscribe` and topic changes wait for FCM. */
private const val REGISTER_TIMEOUT_SECONDS = 30L

/**
 * How long the Worker's listeners have for an opened or dismissed notification, within the 10
 * seconds Android allows a broadcast receiver.
 */
private const val RESPONSE_DEADLINE_MILLIS = 8_000L
private const val FCM_MESSAGE_ID_EXTRA = "google.message_id"

/** The names the page gives channel importances. */
private val IMPORTANCES =
    mapOf(
        NotificationManager.IMPORTANCE_NONE to Importance.none,
        NotificationManager.IMPORTANCE_MIN to Importance.min,
        NotificationManager.IMPORTANCE_LOW to Importance.low,
        NotificationManager.IMPORTANCE_DEFAULT to Importance.default,
        NotificationManager.IMPORTANCE_HIGH to Importance.high,
        NotificationManager.IMPORTANCE_MAX to Importance.high,
    )

class TokamakNotificationsPlugin(
    private val host: TokamakHost,
) : TokamakPlugin {
    override val id = "notifications"

    private val context = host.context
    private val preferences = context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
    private val notifier = Notifier(host)
    private val schedule = Schedule(host)
    private val images = Images(host)

    /** The latest presentation of each notification whose images are loading, which [remove] cancels. */
    private val presenting = mutableMapOf<String, Any>()
    private val messages = TokamakListeners<Message>()
    private val opened = TokamakListeners<OpenedNotification>(holding = true)
    private val dismissed = TokamakListeners<DismissedNotification>(holding = true)
    private val settingsRequests = TokamakListeners<OpenSettingsEvent>(holding = true)
    private val subscriptionChanges = TokamakListeners<Subscription>()

    init {
        notifier.ensureDefaultChannel()
        // Force-stopping the app, or revoking exact alarms, clears its alarms.
        armAll()
    }

    /**
     * Applies [config], all of which it checks first, and stores it for alarms and pushes that
     * arrive before the Worker runs.
     */
    fun setup(config: SetupConfig): SetupResult {
        val setup = Setup(config)
        setup.requireSounds(context)
        val unapplied = notifier.configure(setup, Setup.stored(host))
        setup.store(host)
        return SetupResult(unapplied)
    }

    fun permission(): Permission =
        when {
            notifier.enabled -> Permission.granted
            canPrompt() -> Permission.prompt
            else -> Permission.denied
        }

    /** Asks for permission when that shows the system prompt, which needs the app in the foreground. */
    suspend fun requestPermission(): Permission {
        if (canPrompt()) {
            suspendCancellableCoroutine { asked ->
                host.requestPermissions(setOf(Manifest.permission.POST_NOTIFICATIONS)) { asked.resume(Unit) }
            }
        }
        return permission()
    }

    /** The user's notification settings: the permission, channels and special access. */
    fun settings(): NotificationSettings {
        val manager = context.getSystemService(NotificationManager::class.java)
        return NotificationSettings(
            permission = permission(),
            channels = manager.notificationChannels.map { ChannelImportance(it.id, IMPORTANCES[it.importance] ?: Importance.default) },
            blockedChannelGroups = manager.notificationChannelGroups.filter { it.isBlocked }.map { it.id },
            doNotDisturbAccess = manager.isNotificationPolicyAccessGranted,
            exactAlarms = schedule.canScheduleExact,
            fullScreenIntent = Build.VERSION.SDK_INT < Build.VERSION_CODES.UPSIDE_DOWN_CAKE || manager.canUseFullScreenIntent(),
        )
    }

    /** Opens the app's notification settings, a channel's, or the special access [options] names. */
    fun openSettings(options: OpenSettingsOptions?) {
        val app = Uri.fromParts("package", context.packageName, null)
        val channel = options?.channel
        val intent =
            when {
                options?.exactAlarms == true -> Intent(Settings.ACTION_REQUEST_SCHEDULE_EXACT_ALARM, app)
                options?.fullScreenIntent == true ->
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                        Intent(Settings.ACTION_MANAGE_APP_USE_FULL_SCREEN_INTENT, app)
                    } else {
                        throw TokamakPluginError.notSupported("Full-screen notification settings need Android 14")
                    }
                options?.doNotDisturbAccess == true -> Intent(Settings.ACTION_NOTIFICATION_POLICY_ACCESS_SETTINGS)
                channel != null -> {
                    notifier.requireChannel(channel)
                    Intent(Settings.ACTION_CHANNEL_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_CHANNEL_ID, channel)
                }
                else -> Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
            }
        host.requireForegroundActivity().startActivity(intent.putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName))
    }

    suspend fun show(notification: NotificationContent) {
        notification.check()
        show(notification, Source.local)
    }

    /** Shows a notification `at` a time now when the time has passed, as Apple platforms do. */
    suspend fun schedule(notification: NotificationContent, trigger: Trigger) {
        notification.check()
        val now = System.currentTimeMillis()
        // Finding the first time checks the trigger's fields.
        val next = trigger.next(now, now)
        if (trigger is Trigger.AtTrigger && trigger.at <= now) return show(notification, Source.local)
        requireEnabled()
        notifier.requireChannel(notification)
        if (trigger.exact == true && !schedule.canScheduleExact) {
            throw TokamakPluginError(
                "NotAllowedError",
                "Exact alarms need the user's permission, which openSettings({ exactAlarms: true }) opens",
            )
        }
        if (next == null) throw TokamakPluginError.typeError("on matches no time from now")
        schedule.add(ScheduledNotification(notification, trigger))
    }

    /** Scheduled notifications not yet shown, as they were scheduled. */
    fun getScheduled(): List<ScheduledNotification> = schedule.all()

    /** Shown notifications still in the notification drawer. */
    fun getDelivered(): List<NotificationContent> = notifier.delivered()

    /** Removes a scheduled or shown notification. */
    fun remove(id: String) {
        if (id.isEmpty()) throw TokamakPluginError.typeError("id must be a non-empty string")
        presenting.remove(id)
        schedule.remove(id)
        notifier.remove(id)
    }

    /** Subscribes to FCM, which sends to the Firebase Installation ID. */
    suspend fun subscribeToPush(): Subscription {
        val messaging = messaging() ?: throw invalidState("Push requires the app's Firebase project values in its Android manifest")
        preferences.edit().putBoolean(SUBSCRIBED, true).apply()
        messaging.isAutoInitEnabled = true
        val installations = FirebaseInstallations.getInstance()
        // getId rejects malformed Firebase values at once; register() never completes on them.
        // Registering can replace the FID, so the FID is read again afterwards.
        installations.id.result()
        messaging.register().result()
        val fid = installations.id.result()
        if (!preferences.getBoolean(SUBSCRIBED, false)) throw TokamakPluginError("AbortError", "unsubscribe was called")
        preferences.edit().putString(FID, fid).apply()
        return Subscription(PushService.fcm, fid)
    }

    fun getSubscription(): Subscription? {
        if (!preferences.getBoolean(SUBSCRIBED, false)) return null
        return preferences.getString(FID, null)?.let { Subscription(PushService.fcm, it) }
    }

    suspend fun unsubscribe() {
        preferences.edit().remove(SUBSCRIBED).remove(FID).apply()
        val messaging = messaging() ?: return
        messaging.isAutoInitEnabled = false
        messaging.unregister().result()
    }

    /** Receives the push messages the app's server sends to [topic] through FCM. */
    suspend fun subscribeToTopic(topic: String) {
        topicMessaging().subscribeToTopic(topic).result()
    }

    suspend fun unsubscribeFromTopic(topic: String) {
        topicMessaging().unsubscribeFromTopic(topic).result()
    }

    fun onMessage(): Flow<Message> = messages.stream()

    fun onNotificationOpened(): Flow<OpenedNotification> = opened.stream()

    fun onNotificationDismissed(): Flow<DismissedNotification> = dismissed.stream()

    fun onOpenSettings(): Flow<OpenSettingsEvent> = settingsRequests.stream()

    fun onSubscriptionChange(): Flow<Subscription> = subscriptionChanges.stream()

    /**
     * Delivers the notification or `foreground` action that started the activity, or the system
     * settings' request to open the app's own notification settings.
     */
    override fun onIntent(intent: Intent) {
        if (intent.hasCategory(Notification.INTENT_CATEGORY_NOTIFICATION_PREFERENCES)) {
            intent.removeCategory(Notification.INTENT_CATEGORY_NOTIFICATION_PREFERENCES)
            settingsRequests.send(
                OpenSettingsEvent(intent.getStringExtra(Notification.EXTRA_CHANNEL_ID), Notifier.settingsNotification(intent)),
            )
            return
        }
        val opened =
            intent.getStringExtra(OPENED_EXTRA)?.let { opened(intent, it) }
                ?: intent.getStringExtra(FCM_MESSAGE_ID_EXTRA)?.let { openedPush(intent, it) }
                ?: return
        removeSummaryIfEmpty(intent, opened.id)
        intent.removeExtra(OPENED_EXTRA)
        intent.removeExtra(FCM_MESSAGE_ID_EXTRA)
        intent.removeExtra(GROUP_EXTRA)
        deliver(this.opened, "opened", opened) {}
    }

    /** Delivers an action without `foreground`, or a dismissed notification. Call it on the main thread. */
    internal fun respond(intent: Intent, finish: () -> Unit) {
        val dismissed = intent.getStringExtra(DISMISSED_EXTRA)?.let { host.fromJson(JSONObject(it), DismissedNotification::class) }
        val opened = intent.getStringExtra(OPENED_EXTRA)?.let { opened(intent, it) }
        dismissed?.let { removeSummaryIfEmpty(intent, it.id) }
        when {
            dismissed != null -> deliver(this.dismissed, "dismissed", dismissed, finish)
            opened != null -> deliver(this.opened, "opened", opened, finish)
            else -> finish()
        }
    }

    /** Removes the summary of the group of the notification [id], which Android removed, if it was the group's last. */
    private fun removeSummaryIfEmpty(intent: Intent, id: String) {
        intent.getStringExtra(GROUP_EXTRA)?.let { notifier.removeSummaryIfEmpty(it, id) }
    }

    /** An opened notification with the typed text, removing the notification after an action, as Apple does. */
    private fun opened(intent: Intent, json: String): OpenedNotification {
        val opened = host.fromJson(JSONObject(json), OpenedNotification::class).copy(input = Notifier.input(intent))
        if (opened.action != null) notifier.remove(opened.id)
        return opened
    }

    /** Delivers [value] to the page's [listeners] and the Worker's [event] listeners, then calls [finish]. */
    private fun <T : Any> deliver(listeners: TokamakListeners<T>, event: String, value: T, finish: () -> Unit) {
        listeners.send(value)
        Thread {
            runCatching { host.emit(event, value, RESPONSE_DEADLINE_MILLIS) }
                .onFailure { Log.w("tokamak", "$event notification failed: ${it.message}") }
            finish()
        }.start()
    }

    /** Reports a changed registration; `subscribeToPush` reports the first. FCM calls this off the main thread. */
    internal fun onRegistered(fid: String) {
        val previous = preferences.getString(FID, null) ?: return
        if (!preferences.getBoolean(SUBSCRIBED, false) || fid == previous) return
        preferences.edit().putString(FID, fid).apply()
        subscriptionChanges.send(Subscription(PushService.fcm, fid))
    }

    /**
     * Delivers an FCM message to the page, and a data-only message to the Worker's `onPush`
     * listeners, showing the notification they reply with. FCM calls this off the main thread.
     */
    internal fun onMessageReceived(received: Message, notification: NotificationContent?) {
        val deadline = SystemClock.elapsedRealtime() + PUSH_DEADLINE_MILLIS
        messages.send(received)
        if (notification != null) {
            presentBeforeReturning(notification, Source.push, deadline)
            return
        }
        runCatching { host.emit("push", received, NotificationContent::class, PUSH_DEADLINE_MILLIS) }
            .onSuccess { showReply(it, deadline) }
            .onFailure { Log.w("tokamak", "push notification failed: ${it.message}") }
    }

    /** Shows the notification `onPush` listeners reply with, if any, with the images that load by [deadline]. */
    private fun showReply(reply: NotificationContent?, deadline: Long) {
        if (reply == null) return
        runCatching {
            reply.check()
            presentBeforeReturning(reply, Source.local, deadline)
        }.onFailure { Log.w("tokamak", "the push reply is not a notification", it) }
    }

    /** Presents [content] from an FCM thread, waiting until it shows: FCM may stop the process once the thread returns. */
    private fun presentBeforeReturning(content: NotificationContent, source: Source, deadline: Long) {
        val shown = CountDownLatch(1)
        context.mainExecutor.execute { present(content, source, deadline) { shown.countDown() } }
        shown.await(deadline + POST_TIMEOUT_MILLIS - SystemClock.elapsedRealtime(), TimeUnit.MILLISECONDS)
    }

    /** Whether asking shows the system prompt, which Android 13 and later show until the user refuses twice. */
    private fun canPrompt(): Boolean =
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            host.wouldPrompt(Manifest.permission.POST_NOTIFICATIONS)

    private suspend fun show(content: NotificationContent, source: Source) {
        requireEnabled()
        notifier.requireChannel(content)
        suspendCancellableCoroutine { shown -> present(content, source) { shown.resumeWith(it) } }
    }

    /** Sets an alarm for every scheduled notification, as a restart or a clock change clears them. */
    internal fun armAll() = schedule.armAll()

    /** Shows the scheduled notification [id], whose alarm fired, then calls [done] on the main thread. */
    internal fun fire(id: String, done: () -> Unit) {
        val scheduled = schedule.fire(id) ?: return done()
        present(scheduled.content, Source.local) { done() }
    }

    /**
     * Shows a notification with the images that load by [deadline], in [SystemClock.elapsedRealtime]
     * milliseconds, then calls [done] on the main thread with whether posting succeeded. Shows
     * nothing when [remove] or a later presentation of the notification comes first, or when the
     * app is on screen by then and `setup` hides notifications there. Call it on the main thread.
     */
    internal fun present(
        content: NotificationContent,
        source: Source,
        deadline: Long = SystemClock.elapsedRealtime() + IMAGE_TIMEOUT_MILLIS,
        done: (Result<Unit>) -> Unit,
    ) {
        val presentation = Any()
        presenting[content.id] = presentation
        images.load(notifier.imageSources(content), deadline) { loaded ->
            if (presenting[content.id] !== presentation) return@load done(Result.success(Unit))
            presenting.remove(content.id)
            if (host.isInForeground && !showInForeground()) return@load done(Result.success(Unit))
            val posted = runCatching { notifier.post(content, source, loaded) }
            posted.onFailure { Log.w("tokamak", "notification ${content.id} failed", it) }
            done(posted)
        }
    }

    private fun showInForeground(): Boolean = Setup.stored(host)?.showInForeground ?: true

    private fun messaging(): FirebaseMessaging? =
        if (FirebaseApp.getApps(context).isEmpty()) null else FirebaseMessaging.getInstance()

    private fun topicMessaging(): FirebaseMessaging =
        messaging() ?: throw invalidState("Topics require the app's Firebase project values in its Android manifest")

    /** What the task gives, waiting at most [REGISTER_TIMEOUT_SECONDS], or its failure as an `OperationError`. */
    private suspend fun <T> Task<T>.result(): T =
        suspendCancellableCoroutine { continuation ->
            Tasks.withTimeout(this, REGISTER_TIMEOUT_SECONDS, TimeUnit.SECONDS).addOnCompleteListener { task ->
                if (task.isSuccessful) continuation.resume(task.result) else continuation.resumeWithException(operationError(task.exception))
            }
        }

    /** An opened FCM notification: FCM passes its data map, but not its title or body. */
    private fun openedPush(intent: Intent, messageId: String): OpenedNotification {
        val data = JSONObject()
        intent.extras?.keySet().orEmpty()
            .filterNot { it.startsWith("google.") || it.startsWith("gcm.") || it in FCM_KEYS }
            .forEach { key -> intent.getStringExtra(key)?.let { data.put(key, it) } }
        return OpenedNotification(messageId, null, null, data, Source.push)
    }

    private fun requireEnabled() {
        if (!notifier.enabled) {
            throw TokamakPluginError("NotAllowedError", "Notification permission has not been granted")
        }
    }

    private companion object {
        /** Extras FCM adds to the launch intent of an opened notification. */
        val FCM_KEYS = setOf("from", "collapse_key")

        fun invalidState(message: String) = TokamakPluginError("InvalidStateError", message)

        fun operationError(error: Exception?) =
            TokamakPluginError(
                "OperationError",
                if (error is TimeoutException) {
                    "FCM did not respond within $REGISTER_TIMEOUT_SECONDS seconds"
                } else {
                    error?.message ?: "The push service failed"
                },
            )
    }
}
