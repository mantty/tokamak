package com.tokamak.plugins.notifications

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.SystemClock
import android.util.Log
import com.google.firebase.FirebaseApp
import com.google.firebase.messaging.FirebaseMessaging
import com.tokamak.runtime.TokamakHost
import com.tokamak.runtime.TokamakPlugin
import com.tokamak.runtime.TokamakPluginError
import com.tokamak.runtime.TokamakPluginReply
import org.json.JSONArray
import org.json.JSONObject
import org.json.JSONTokener

private const val PREFERENCES = "tokamak.notifications"
private const val ASKED = "asked"
private const val SUBSCRIBED = "subscribed"
private const val TOKEN = "token"
private const val SHOW_IN_FOREGROUND = "show-in-foreground"
private const val PERMISSION_REQUEST = 0x4E07

/** FCM allows about 10 seconds for a message, including starting the app. */
private const val PUSH_DEADLINE_MILLIS = 8_000L
private const val PUSH_ATTEMPTS = 3
private const val PUSH_ATTEMPT_TIMEOUT_MILLIS = 2_000L
private const val PUSH_RETRY_DELAY_MILLIS = 1_000L
private const val FCM_MESSAGE_ID_EXTRA = "google.message_id"
private val LISTENERS = setOf("onMessage", "onNotificationOpened", "onSubscriptionChange")

class TokamakNotificationsPlugin(
    private val host: TokamakHost,
) : TokamakPlugin {
    override val id = "notifications"

    private val context = host.context
    private val preferences = context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
    private val notifier = Notifier(context)
    private val schedule = Schedule(context)
    private val listeners = mutableMapOf<String, MutableMap<Int, TokamakPluginReply>>()
    private var nextListener = 0

    /** Notifications opened while no page listened for them. */
    private val heldOpened = mutableListOf<JSONObject>()
    private val permissionReplies = mutableListOf<TokamakPluginReply>()

    init {
        notifier.createChannel()
    }

    override fun call(method: String, arguments: Any?, reply: TokamakPluginReply) {
        runCatching {
            when (method) {
                "permission" -> reply(Result.success(permission()))
                "requestPermission" -> requestPermission(reply)
                "show" -> show(Content.parse(arguments, scheduled = false), reply)
                "schedule" -> schedule(Content.parse(arguments, scheduled = true), reply)
                "getScheduled" -> reply(Result.success(JSONArray(schedule.all().map(Content::toJson))))
                "getDelivered" -> reply(Result.success(JSONArray(notifier.delivered())))
                "remove" -> remove(identifier(arguments), reply)
                "subscribe" -> subscribe(arguments, reply)
                "getSubscription" -> reply(Result.success(subscription()))
                "unsubscribe" -> unsubscribe(reply)
                else -> super.call(method, arguments, reply)
            }
        }.onFailure { reply(Result.failure(pluginError(it))) }
    }

    override fun subscribe(
        method: String,
        arguments: Any?,
        reply: TokamakPluginReply,
    ): () -> Unit {
        if (method !in LISTENERS) return super.subscribe(method, arguments, reply)
        val key = nextListener++
        listeners.getOrPut(method, ::mutableMapOf)[key] = reply
        if (method == "onNotificationOpened") {
            heldOpened.forEach { reply(Result.success(it)) }
            heldOpened.clear()
        }
        return { listeners[method]?.remove(key) }
    }

    override fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray,
    ) {
        if (requestCode != PERMISSION_REQUEST) return
        val permission = permission()
        permissionReplies.forEach { it(Result.success(permission)) }
        permissionReplies.clear()
    }

    /** Delivers a notification the user opened, local or from FCM, that started the activity. */
    override fun onIntent(intent: Intent) {
        val opened =
            intent.getStringExtra(OPENED_EXTRA)?.let(::JSONObject)
                ?: intent.getStringExtra(FCM_MESSAGE_ID_EXTRA)?.let { openedPush(intent, it) }
                ?: return
        intent.removeExtra(OPENED_EXTRA)
        intent.removeExtra(FCM_MESSAGE_ID_EXTRA)
        emit("onNotificationOpened", opened)
    }

    /** Reports a replaced token; `subscribe` reports the first. */
    internal fun onNewToken(token: String) {
        val previous = preferences.getString(TOKEN, null) ?: return
        if (!preferences.getBoolean(SUBSCRIBED, false) || token == previous) return
        preferences.edit().putString(TOKEN, token).apply()
        context.mainExecutor.execute { emit("onSubscriptionChange", fcmSubscription(token)) }
    }

    /**
     * Delivers an FCM message to the page. Posts a data-only message to the Worker's push
     * endpoint and shows the notification it returns. FCM calls this off the main thread.
     */
    internal fun onMessageReceived(message: JSONObject, visible: Boolean) {
        context.mainExecutor.execute { emit("onMessage", message) }
        if (visible) {
            if (preferences.getBoolean(SHOW_IN_FOREGROUND, false)) notifier.post(content(message), "push")
            return
        }
        val deadline = SystemClock.elapsedRealtime() + PUSH_DEADLINE_MILLIS
        runCatching { deliverPush(message.toString(), attempt = 1, deadline = deadline) }
            .onSuccess(::showReturned)
    }

    /**
     * Posts [body] to `/tokamak/push`, logging each failed attempt and retrying while another
     * attempt can finish before [deadline].
     */
    private fun deliverPush(
        body: String,
        attempt: Int,
        deadline: Long,
    ): String =
        try {
            host.call("push", body, PUSH_ATTEMPT_TIMEOUT_MILLIS)
        } catch (error: Exception) {
            Log.w("tokamak", "push notification failed: ${error.message}")
            val retryEnds = SystemClock.elapsedRealtime() + PUSH_RETRY_DELAY_MILLIS + PUSH_ATTEMPT_TIMEOUT_MILLIS
            if (attempt == PUSH_ATTEMPTS || retryEnds > deadline) throw error
            Thread.sleep(PUSH_RETRY_DELAY_MILLIS)
            deliverPush(body, attempt + 1, deadline)
        }

    /** Shows the notification a push response returns, if any. */
    private fun showReturned(response: String) {
        if (response.isEmpty()) return
        runCatching {
            val returned = JSONTokener(response).nextValue()
            if (returned == JSONObject.NULL) return
            notifier.post(Content.parse(returned as JSONObject, scheduled = false), "local")
        }.onFailure { Log.w("tokamak", "the push response is not a notification", it) }
    }

    private fun emit(method: String, value: JSONObject) {
        val current = listeners[method].orEmpty().values.toList()
        if (method == "onNotificationOpened" && current.isEmpty()) heldOpened += value
        current.forEach { it(Result.success(value)) }
    }

    private fun permission(): String =
        when {
            notifier.enabled -> "granted"
            canPrompt() -> "prompt"
            else -> "denied"
        }

    /** Whether asking shows the system prompt, which Android 13 and later show until the user refuses twice. */
    private fun canPrompt(): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return false
        val permission = Manifest.permission.POST_NOTIFICATIONS
        if (context.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED) return false
        return !preferences.getBoolean(ASKED, false) ||
            host.activity?.shouldShowRequestPermissionRationale(permission) == true
    }

    private fun requestPermission(reply: TokamakPluginReply) {
        val activity = host.activity
        if (
            Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
            activity == null ||
            permission() != "prompt"
        ) {
            reply(Result.success(permission()))
            return
        }
        preferences.edit().putBoolean(ASKED, true).apply()
        permissionReplies += reply
        if (permissionReplies.size == 1) {
            activity.requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), PERMISSION_REQUEST)
        }
    }

    private fun show(content: Content, reply: TokamakPluginReply) {
        requireEnabled()
        notifier.post(content, "local")
        reply(Result.success(null))
    }

    private fun schedule(content: Content, reply: TokamakPluginReply) {
        requireEnabled()
        if ((content.at ?: 0) <= System.currentTimeMillis()) {
            notifier.post(content, "local")
        } else {
            schedule.add(content)
        }
        reply(Result.success(null))
    }

    private fun remove(id: String, reply: TokamakPluginReply) {
        schedule.remove(id)
        notifier.remove(id)
        reply(Result.success(null))
    }

    private fun subscribe(arguments: Any?, reply: TokamakPluginReply) {
        val messaging = messaging() ?: throw invalidState(
            "Push requires the app's Firebase project values in its Android manifest",
        )
        val showInForeground = (arguments as? JSONObject)?.optBoolean("showInForeground") ?: false
        preferences.edit()
            .putBoolean(SUBSCRIBED, true)
            .putBoolean(SHOW_IN_FOREGROUND, showInForeground)
            .apply()
        messaging.isAutoInitEnabled = true
        messaging.token.addOnCompleteListener { task ->
            when {
                !preferences.getBoolean(SUBSCRIBED, false) ->
                    reply(Result.failure(TokamakPluginError("AbortError", "unsubscribe was called")))
                task.isSuccessful -> {
                    preferences.edit().putString(TOKEN, task.result).apply()
                    reply(Result.success(fcmSubscription(task.result)))
                }
                else -> reply(Result.failure(operationError(task.exception)))
            }
        }
    }

    private fun subscription(): JSONObject? {
        if (!preferences.getBoolean(SUBSCRIBED, false)) return null
        return preferences.getString(TOKEN, null)?.let(::fcmSubscription)
    }

    private fun unsubscribe(reply: TokamakPluginReply) {
        preferences.edit().remove(SUBSCRIBED).remove(TOKEN).remove(SHOW_IN_FOREGROUND).apply()
        val messaging = messaging() ?: return reply(Result.success(null))
        messaging.isAutoInitEnabled = false
        messaging.deleteToken().addOnCompleteListener { task ->
            reply(if (task.isSuccessful) Result.success(null) else Result.failure(operationError(task.exception)))
        }
    }

    private fun messaging(): FirebaseMessaging? =
        if (FirebaseApp.getApps(context).isEmpty()) null else FirebaseMessaging.getInstance()

    /** A received push message's notification, shown as FCM would show it. */
    private fun content(message: JSONObject): Content =
        Content(
            message.getString("id"),
            message.opt("title") as? String ?: "",
            message.opt("body") as? String,
            message.getJSONObject("data"),
            null,
        )

    private fun fcmSubscription(token: String): JSONObject =
        JSONObject().put("service", "fcm").put("token", token)

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
    }

    private fun requireEnabled() {
        if (!notifier.enabled) {
            throw TokamakPluginError("NotAllowedError", "Notification permission has not been granted")
        }
    }

    private fun identifier(arguments: Any?): String {
        val id = (arguments as? JSONObject)?.opt("id") as? String
        if (id.isNullOrEmpty()) throw TokamakPluginError("TypeError", "id must be a non-empty string")
        return id
    }

    private companion object {
        /** Extras FCM adds to the launch intent of an opened notification. */
        val FCM_KEYS = setOf("from", "collapse_key")

        fun invalidState(message: String) = TokamakPluginError("InvalidStateError", message)

        fun operationError(error: Throwable?) =
            TokamakPluginError("OperationError", error?.message ?: "The push service failed")

        fun pluginError(error: Throwable): TokamakPluginError =
            error as? TokamakPluginError ?: operationError(error)
    }
}
