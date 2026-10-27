package com.tokamak.runtime

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.os.Handler
import android.os.Looper
import java.util.concurrent.Executor
import kotlin.reflect.KClass
import org.json.JSONArray
import org.json.JSONObject
import org.json.JSONTokener

/**
 * A failure a plugin reports to the worker as a `DOMException` named [errorName]. Other exceptions
 * reach the worker as an `OperationError`.
 */
class TokamakPluginError(
    val errorName: String,
    message: String,
    /** JSON details the Worker receives as the error's `cause`. */
    val errorCause: Any? = null,
) : Exception(message) {
    companion object {
        fun notSupported(message: String) = TokamakPluginError("NotSupportedError", message)

        fun typeError(message: String) = TokamakPluginError("TypeError", message)

        /** UI was needed while the app is in the background, where it cannot show any. */
        fun needsUI() = TokamakPluginError("NeedsUIError", "The app cannot show UI while it is in the background")

        /** The plugin emitted an event its registration does not list. */
        fun unlistedEvent(name: String) = typeError("android/Exports.kt lists no event $name")

        /** The plugin asked for the reply to an event its registration lists without one. */
        fun unlistedReply(name: String) = typeError("android/Exports.kt lists no reply to event $name")
    }
}

typealias TokamakPluginReply = (Result<Any?>) -> Unit
typealias TokamakPluginBytesReply = (TokamakPluginResult) -> Unit

data class TokamakPluginResult(val payload: ByteArray, val failed: Boolean, val done: Boolean) {
    companion object {
        fun encoded(result: Result<ByteArray>, done: Boolean): TokamakPluginResult {
            val error = result.exceptionOrNull()
                ?: return TokamakPluginResult(result.getOrThrow(), false, done)
            return TokamakPluginResult(failure(error).toString().toByteArray(Charsets.UTF_8), true, done)
        }
    }
}

/** [error] for the Worker: its name, message and cause, or an `OperationError` caused by the exception's class. */
internal fun failure(error: Throwable): JSONObject {
    val failure = JSONObject().put("message", error.message ?: error.javaClass.name)
    return if (error is TokamakPluginError) {
        failure.put("name", error.errorName).putOpt("cause", JSONObject.wrap(error.errorCause))
    } else {
        failure.put("name", "OperationError").put("cause", JSONObject().put("exception", error.javaClass.name))
    }
}

internal fun decodePluginValue(payload: ByteArray): Any? = JSONTokener(payload.toString(Charsets.UTF_8)).nextValue()

/**
 * The bytes the Worker streams to a call, which the plugin reads one chunk at a time on the call's
 * [executor]; [pull] asks the Worker for the next.
 */
class TokamakPluginBody internal constructor(private val executor: Executor, private val pull: () -> Unit) {
    private var reading: ((Result<ByteArray?>) -> Unit)? = null

    /**
     * Reads the next chunk: its bytes, null after the last, or the Worker's error. [handler] runs on
     * the call's executor; read again only after it has.
     */
    fun read(handler: (Result<ByteArray?>) -> Unit) {
        synchronized(this) { reading = handler }
        pull()
    }

    /** Answers the pending read with the chunk the Worker sent. */
    internal fun receive(chunk: TokamakPluginResult) {
        val handler = synchronized(this) { reading.also { reading = null } } ?: return
        executor.execute {
            handler(
                when {
                    chunk.failed -> Result.failure(chunkError(chunk.payload))
                    chunk.done -> Result.success(null)
                    else -> Result.success(chunk.payload)
                },
            )
        }
    }

    private fun chunkError(payload: ByteArray): TokamakPluginError {
        val error = runCatching { JSONObject(payload.toString(Charsets.UTF_8)) }.getOrNull()
        return TokamakPluginError(error?.optString("name", "Error") ?: "Error", error?.optString("message", "") ?: "")
    }
}

/** [value] as a payload: a [ByteArray] as it is, and any other value as JSON. */
internal fun encodePluginValue(value: Any?): ByteArray {
    if (value is ByteArray) return value
    val wrapped = JSONObject.wrap(value)
    if (value != null && wrapped == null) throw TokamakPluginError("DataCloneError", "The plugin value cannot be encoded as JSON")
    val json = when (wrapped) {
        null, JSONObject.NULL -> "null"
        is String -> JSONObject.quote(wrapped)
        is Number -> JSONObject.numberToString(wrapped)
        is Boolean, is JSONObject, is JSONArray -> wrapped.toString()
        else -> throw TokamakPluginError("DataCloneError", "The plugin value cannot be encoded as JSON")
    }
    return json.toByteArray(Charsets.UTF_8)
}

private val uiExecutor by lazy {
    val main = Handler(Looper.getMainLooper())
    Executor { main.post(it) }
}

/**
 * A native plugin, created once per process with the app's [TokamakHost]. A call or subscription
 * that throws replies with the exception.
 */
interface TokamakPlugin {
    val id: String

    /** Serial executor for this operation and its cancellation. */
    fun executor(method: String): Executor = uiExecutor

    fun callBytes(method: String, payload: ByteArray, reply: TokamakPluginBytesReply): (() -> Unit)? =
        call(method, decodePluginValue(payload)) { reply(TokamakPluginResult.encoded(it.mapCatching(::encodePluginValue), true)) }

    fun callBytes(
        method: String,
        payload: ByteArray,
        body: TokamakPluginBody,
        reply: TokamakPluginBytesReply,
    ): (() -> Unit)? = throw TokamakPluginError.notSupported("$id.$method does not take a body")

    fun subscribeBytes(method: String, payload: ByteArray, reply: TokamakPluginBytesReply): () -> Unit =
        throw TokamakPluginError.notSupported("$id.$method is not supported")

    /** Returns cancellation for pending work when the platform supports it. */
    fun call(method: String, arguments: Any?, reply: TokamakPluginReply): (() -> Unit)? {
        reply(Result.failure(TokamakPluginError.notSupported("$id.$method is not supported")))
        return null
    }

    /** Receives the intent that started the app's activity, and each intent it receives later. */
    fun onIntent(intent: Intent) = Unit
}

/** The app as one plugin sees it: its context, its activity, and its Worker. */
interface TokamakHost {
    /** The application context. */
    val context: Context

    /** The app's activity, or null while it has none. */
    val activity: Activity?

    /** Whether the app is in the foreground, where it can show UI. Read it on the main thread. */
    val isInForeground: Boolean

    /** The native objects plugins share with each other by ID. */
    val resources: TokamakResources

    /**
     * The activity to show UI in. Throws `NeedsUIError` while the app is in the background, where
     * it cannot show UI. Call it on the main thread.
     */
    fun requireForegroundActivity(): Activity

    /**
     * Whether asking for [permission] shows the system prompt: the app does not hold it, and the
     * user has not refused it for good. Call it on the main thread.
     */
    fun wouldPrompt(permission: String): Boolean

    /**
     * Asks the user for [permissions] in the app's activity once earlier requests finish, then
     * returns whether each is granted. Answers at once, without UI, when none would prompt; throws
     * `NeedsUIError` when one would while the app is in the background. Call it on the main thread.
     */
    suspend fun requestPermissions(permissions: Set<String>): Map<String, Boolean>

    /**
     * Delivers the plugin's event [name], which the Worker's listeners receive as
     * `<plugin>.android.<name>`, with its encoded [payload], and returns their encoded reply, which
     * is empty when none replied. Starts the runtime when it is not running. Blocks, so call it off
     * the main thread; throws unless the listeners return within [timeoutMillis], which includes
     * runtime startup.
     */
    fun emitBytes(name: String, payload: ByteArray, timeoutMillis: Long): ByteArray

    /**
     * Delivers [event] to the Worker's listeners of the plugin's event [name], which
     * `android/Exports.kt` lists. Starts the runtime when it is not running. Blocks, so call it off
     * the main thread; throws unless the listeners return within [timeoutMillis], which includes
     * runtime startup.
     */
    fun emit(name: String, event: Any, timeoutMillis: Long)

    /** [emit] of an event whose listeners may reply, returning the reply, or null when none replied. */
    fun <R : Any> emit(name: String, event: Any, reply: KClass<R>, timeoutMillis: Long): R?

    /**
     * [value], of a data class or sealed type `android/Exports.kt`'s methods and events use, as
     * the JSON the Worker receives, which org.json writes, for a value the plugin stores.
     */
    fun json(value: Any): Any

    /**
     * The JSON [json], as org.json reads it, as a data class or sealed [type] `android/Exports.kt`'s
     * methods and events use, for a value the plugin stored with [json].
     */
    fun <T : Any> fromJson(json: Any?, type: KClass<T>): T

    /**
     * What the app serves at [path], as its WebView would load it: a file the app ships or the
     * Worker's response. Starts the runtime when it is not running. Blocks, so call it off the main
     * thread; throws unless it arrives within [timeoutMillis].
     */
    fun fetch(path: String, timeoutMillis: Long): ByteArray
}

/** The app's plugins. The app's [android.app.Application] is one. */
interface TokamakPlugins {
    /** The plugin with [id], or null when the app does not include it. */
    fun plugin(id: String): TokamakPlugin?
}

/** The plugin with [id] in the app this context belongs to, or null when the app does not include it. */
fun Context.tokamakPlugin(id: String): TokamakPlugin? = (applicationContext as TokamakPlugins).plugin(id)

/** The app's monochrome `drawable/tokamak_notification`, or its launcher icon, for a notification's small icon. */
fun Context.tokamakNotificationIcon(): Int =
    resources.getIdentifier("tokamak_notification", "drawable", packageName).takeIf { it != 0 }
        ?: applicationInfo.icon.takeIf { it != 0 }
        ?: android.R.drawable.sym_def_app_icon
