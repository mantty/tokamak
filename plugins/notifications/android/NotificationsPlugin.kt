package com.tokamak.plugins.notifications

import android.Manifest
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.SystemClock
import android.util.Log
import com.google.android.gms.tasks.Tasks
import com.google.firebase.FirebaseApp
import com.google.firebase.installations.FirebaseInstallations
import com.google.firebase.messaging.FirebaseMessaging
import com.tokamak.runtime.TokamakHost
import com.tokamak.runtime.TokamakPlugin
import com.tokamak.runtime.TokamakPluginError
import com.tokamak.runtime.TokamakPluginReply
import org.json.JSONArray
import org.json.JSONObject
import org.json.JSONTokener
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.TimeoutException

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

/** How long `subscribe` and `unsubscribe` wait for FCM. */
private const val REGISTER_TIMEOUT_SECONDS = 30L

/**
 * How long the Worker's listeners have for an opened or dismissed notification, within the 10
 * seconds Android allows a broadcast receiver.
 */
private const val RESPONSE_DEADLINE_MILLIS = 8_000L
private const val FCM_MESSAGE_ID_EXTRA = "google.message_id"
private val LISTENERS = setOf("onMessage", "onNotificationOpened", "onNotificationDismissed", "onSubscriptionChange")

/** Listener methods that receive what happened before a page listened. */
private val HELD = setOf("onNotificationOpened", "onNotificationDismissed")

class TokamakNotificationsPlugin(
    private val host: TokamakHost,
) : TokamakPlugin {
    override val id = "notifications"

    private val context = host.context
    private val preferences = context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
    private val notifier = Notifier(context)
    private val schedule = Schedule(context)
    private val images = Images(host)

    /** The latest presentation of each notification whose images are loading, which [remove] cancels. */
    private val presenting = mutableMapOf<String, Any>()
    private val listeners = mutableMapOf<String, MutableMap<Int, TokamakPluginReply>>()
    private var nextListener = 0

    /** Notifications opened or dismissed while no page listened for them, by listener method. */
    private val held = mutableMapOf<String, MutableList<JSONObject>>()

    init {
        notifier.ensureDefaultChannel()
    }

    override fun call(method: String, arguments: Any?, reply: TokamakPluginReply) {
        when (method) {
            "setup" -> setup(arguments, reply)
            "permission" -> reply(Result.success(permission()))
            "requestPermission" -> requestPermission(reply)
            "show" -> show(Content.parse(arguments, scheduled = false), reply)
            "schedule" -> schedule(Content.parse(arguments, scheduled = true), reply)
            "getScheduled" -> reply(Result.success(JSONArray(schedule.all().map { it.json })))
            "getDelivered" -> reply(Result.success(JSONArray(notifier.delivered())))
            "remove" -> remove(identifier(arguments), reply)
            "subscribe" -> subscribe(reply)
            "getSubscription" -> reply(Result.success(subscription()))
            "unsubscribe" -> unsubscribe(reply)
            else -> super.call(method, arguments, reply)
        }
    }

    override fun subscribe(
        method: String,
        arguments: Any?,
        reply: TokamakPluginReply,
    ): () -> Unit {
        if (method !in LISTENERS) return super.subscribe(method, arguments, reply)
        val key = nextListener++
        listeners.getOrPut(method, ::mutableMapOf)[key] = reply
        held.remove(method)?.forEach { reply(Result.success(it)) }
        return { listeners[method]?.remove(key) }
    }

    /** Delivers a notification, or an action with `foreground`, that started the activity. */
    override fun onIntent(intent: Intent) {
        val opened =
            intent.getStringExtra(OPENED_EXTRA)?.let { opened(intent, it) }
                ?: intent.getStringExtra(FCM_MESSAGE_ID_EXTRA)?.let { openedPush(intent, it) }
                ?: return
        removeSummaryIfEmpty(intent, opened)
        intent.removeExtra(OPENED_EXTRA)
        intent.removeExtra(FCM_MESSAGE_ID_EXTRA)
        intent.removeExtra(GROUP_EXTRA)
        deliver("onNotificationOpened", "opened", opened) {}
    }

    /** Delivers an action without `foreground`, or a dismissed notification. Call it on the main thread. */
    internal fun respond(intent: Intent, finish: () -> Unit) {
        val dismissed = intent.getStringExtra(DISMISSED_EXTRA)?.let(::JSONObject)
        val opened = intent.getStringExtra(OPENED_EXTRA)?.let { opened(intent, it) }
        dismissed?.let { removeSummaryIfEmpty(intent, it) }
        when {
            dismissed != null -> deliver("onNotificationDismissed", "dismissed", dismissed, finish)
            opened != null -> deliver("onNotificationOpened", "opened", opened, finish)
            else -> finish()
        }
    }

    /** Removes the summary of the group of [notification], which Android removed, if it was the group's last. */
    private fun removeSummaryIfEmpty(intent: Intent, notification: JSONObject) {
        intent.getStringExtra(GROUP_EXTRA)?.let { notifier.removeSummaryIfEmpty(it, notification.getString("id")) }
    }

    /** An opened notification with the typed text, removing the notification after an action, as Apple does. */
    private fun opened(intent: Intent, json: String): JSONObject {
        val opened = JSONObject(json).put("input", Notifier.input(intent) ?: JSONObject.NULL)
        if (!opened.isNull("action")) notifier.remove(opened.getString("id"))
        return opened
    }

    /** Delivers [value] to the page's [method] listeners and the Worker's [event] listeners, then calls [finish]. */
    private fun deliver(method: String, event: String, value: JSONObject, finish: () -> Unit) {
        notify(method, value)
        Thread {
            runCatching { host.emit(event, value.toString(), RESPONSE_DEADLINE_MILLIS) }
                .onFailure { Log.w("tokamak", "$event notification failed: ${it.message}") }
            finish()
        }.start()
    }

    /** Reports a changed registration; `subscribe` reports the first. FCM calls this off the main thread. */
    internal fun onRegistered(fid: String) {
        val previous = preferences.getString(FID, null) ?: return
        if (!preferences.getBoolean(SUBSCRIBED, false) || fid == previous) return
        preferences.edit().putString(FID, fid).apply()
        context.mainExecutor.execute { notify("onSubscriptionChange", fcmSubscription(fid)) }
    }

    /**
     * Delivers an FCM message to the page, and a data-only message to the Worker's `onPush`
     * listeners, showing the notification they reply with. FCM calls this off the main thread.
     */
    internal fun onMessageReceived(message: JSONObject, notification: Content?) {
        val deadline = SystemClock.elapsedRealtime() + PUSH_DEADLINE_MILLIS
        context.mainExecutor.execute { notify("onMessage", message) }
        if (notification != null) {
            presentBeforeReturning(notification, "push", deadline)
            return
        }
        runCatching { host.emit("push", message.toString(), PUSH_DEADLINE_MILLIS) }
            .onSuccess { showReply(it, deadline) }
            .onFailure { Log.w("tokamak", "push notification failed: ${it.message}") }
    }

    /** Shows the notification `onPush` listeners reply with, if any, with the images that load by [deadline]. */
    private fun showReply(reply: String, deadline: Long) {
        runCatching {
            val notification = JSONTokener(reply).nextValue()
            if (notification == JSONObject.NULL) return
            presentBeforeReturning(Content.parse(notification as JSONObject, scheduled = false), "local", deadline)
        }.onFailure { Log.w("tokamak", "the push reply is not a notification", it) }
    }

    /** Presents [content] from an FCM thread, waiting until it shows: FCM may stop the process once the thread returns. */
    private fun presentBeforeReturning(content: Content, source: String, deadline: Long) {
        val shown = CountDownLatch(1)
        context.mainExecutor.execute { present(content, source, deadline) { shown.countDown() } }
        shown.await(deadline + POST_TIMEOUT_MILLIS - SystemClock.elapsedRealtime(), TimeUnit.MILLISECONDS)
    }

    private fun notify(method: String, value: JSONObject) {
        val current = listeners[method].orEmpty().values.toList()
        if (method in HELD && current.isEmpty()) held.getOrPut(method, ::mutableListOf) += value
        current.forEach { it(Result.success(value)) }
    }

    private fun permission(): String =
        when {
            notifier.enabled -> "granted"
            canPrompt() -> "prompt"
            else -> "denied"
        }

    /** Whether asking shows the system prompt, which Android 13 and later show until the user refuses twice. */
    private fun canPrompt(): Boolean =
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            host.wouldPrompt(Manifest.permission.POST_NOTIFICATIONS)

    /** Asks for permission when that shows the system prompt, which needs the app in the foreground. */
    private fun requestPermission(reply: TokamakPluginReply) {
        if (!canPrompt()) {
            reply(Result.success(permission()))
            return
        }
        host.requestPermissions(setOf(Manifest.permission.POST_NOTIFICATIONS)) {
            reply(Result.success(permission()))
        }
    }

    /**
     * Applies [arguments], all of which it parses first, and stores them for alarms and pushes
     * that arrive before the Worker runs.
     */
    private fun setup(arguments: Any?, reply: TokamakPluginReply) {
        val setup = Setup.parse(arguments)
        val unapplied = notifier.configure(setup, Setup.stored(context))
        setup.store(context)
        reply(Result.success(JSONObject().put("unapplied", unapplied)))
    }

    private fun show(content: Content, reply: TokamakPluginReply) {
        requireEnabled()
        notifier.requireChannel(content)
        present(content, "local") { reply(it.map { null }) }
    }

    private fun schedule(content: Content, reply: TokamakPluginReply) {
        if ((content.at ?: 0) <= System.currentTimeMillis()) return show(content, reply)
        requireEnabled()
        notifier.requireChannel(content)
        schedule.add(content)
        reply(Result.success(null))
    }

    /**
     * Shows a notification with the images that load by [deadline], in [SystemClock.elapsedRealtime]
     * milliseconds, then calls [done] on the main thread with whether posting succeeded. Shows
     * nothing when [remove] or a later presentation of the notification comes first, or when the
     * app is on screen by then and `setup` hides notifications there. Call it on the main thread.
     */
    internal fun present(
        content: Content,
        source: String,
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

    private fun showInForeground(): Boolean = Setup.stored(context)?.showInForeground ?: true

    private fun remove(id: String, reply: TokamakPluginReply) {
        presenting.remove(id)
        schedule.remove(id)
        notifier.remove(id)
        reply(Result.success(null))
    }

    private fun subscribe(reply: TokamakPluginReply) {
        val messaging = messaging() ?: throw invalidState(
            "Push requires the app's Firebase project values in its Android manifest",
        )
        preferences.edit().putBoolean(SUBSCRIBED, true).apply()
        messaging.isAutoInitEnabled = true
        val installations = FirebaseInstallations.getInstance()
        // getId rejects malformed Firebase values at once; register() never completes on them.
        // Registering can replace the FID, so the FID is read again afterwards.
        installations.id
            .onSuccessTask { Tasks.withTimeout(messaging.register(), REGISTER_TIMEOUT_SECONDS, TimeUnit.SECONDS) }
            .onSuccessTask { installations.id }
            .addOnCompleteListener { task ->
                when {
                    !preferences.getBoolean(SUBSCRIBED, false) ->
                        reply(Result.failure(TokamakPluginError("AbortError", "unsubscribe was called")))
                    task.isSuccessful -> {
                        preferences.edit().putString(FID, task.result).apply()
                        reply(Result.success(fcmSubscription(task.result)))
                    }
                    else -> reply(Result.failure(operationError(task.exception)))
                }
            }
    }

    private fun subscription(): JSONObject? {
        if (!preferences.getBoolean(SUBSCRIBED, false)) return null
        return preferences.getString(FID, null)?.let(::fcmSubscription)
    }

    private fun unsubscribe(reply: TokamakPluginReply) {
        preferences.edit().remove(SUBSCRIBED).remove(FID).apply()
        val messaging = messaging() ?: return reply(Result.success(null))
        messaging.isAutoInitEnabled = false
        Tasks.withTimeout(messaging.unregister(), REGISTER_TIMEOUT_SECONDS, TimeUnit.SECONDS).addOnCompleteListener { task ->
            reply(if (task.isSuccessful) Result.success(null) else Result.failure(operationError(task.exception)))
        }
    }

    private fun messaging(): FirebaseMessaging? =
        if (FirebaseApp.getApps(context).isEmpty()) null else FirebaseMessaging.getInstance()

    private fun fcmSubscription(fid: String): JSONObject = JSONObject().put("service", "fcm").put("fid", fid)

    /** An opened FCM notification: FCM passes its data map, but not its title or body. */
    private fun openedPush(intent: Intent, messageId: String): JSONObject {
        val data = JSONObject()
        intent.extras?.keySet().orEmpty()
            .filterNot { it.startsWith("google.") || it.startsWith("gcm.") || it in FCM_KEYS }
            .forEach { key -> intent.getStringExtra(key)?.let { data.put(key, it) } }
        return JSONObject()
            .put("id", messageId)
            .put("title", JSONObject.NULL)
            .put("body", JSONObject.NULL)
            .put("data", data)
            .put("source", "push")
            .put("action", JSONObject.NULL)
            .put("input", JSONObject.NULL)
    }

    private fun requireEnabled() {
        if (!notifier.enabled) {
            throw TokamakPluginError("NotAllowedError", "Notification permission has not been granted")
        }
    }

    private fun identifier(arguments: Any?): String {
        val id = (arguments as? JSONObject)?.opt("id") as? String
        if (id.isNullOrEmpty()) throw TokamakPluginError.typeError("id must be a non-empty string")
        return id
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
