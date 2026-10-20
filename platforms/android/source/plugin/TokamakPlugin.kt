package com.tokamak.runtime

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.os.Handler
import android.os.Looper
import java.util.concurrent.Executor
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
) : Exception(message) {
    companion object {
        fun notSupported(message: String) = TokamakPluginError("NotSupportedError", message)

        fun typeError(message: String) = TokamakPluginError("TypeError", message)

        /** UI was needed while the app is in the background, where it cannot show any. */
        fun needsUI() = TokamakPluginError("NeedsUIError", "The app cannot show UI while it is in the background")
    }
}

typealias TokamakPluginReply = (Result<Any?>) -> Unit
typealias TokamakPluginBytesReply = (TokamakPluginResult) -> Unit

data class TokamakPluginResult(val payload: ByteArray, val failed: Boolean, val done: Boolean) {
    companion object {
        fun encoded(result: Result<ByteArray>, done: Boolean): TokamakPluginResult {
            val error = result.exceptionOrNull()
                ?: return TokamakPluginResult(result.getOrThrow(), false, done)
            val name = (error as? TokamakPluginError)?.errorName ?: "OperationError"
            val payload = JSONObject().put("name", name).put("message", error.message ?: error.javaClass.name)
            return TokamakPluginResult(payload.toString().toByteArray(Charsets.UTF_8), true, done)
        }
    }
}

private fun decodePluginValue(payload: ByteArray): Any? = JSONTokener(payload.toString(Charsets.UTF_8)).nextValue()

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
private fun encodePluginValue(value: Any?): ByteArray {
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
 * A native plugin, created once per process with the app's [TokamakHost]. A [call] or [subscribe]
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
    ): (() -> Unit)? =
        call(method, decodePluginValue(payload), body) {
            reply(TokamakPluginResult.encoded(it.mapCatching(::encodePluginValue), true))
        }

    fun subscribeBytes(method: String, payload: ByteArray, reply: TokamakPluginBytesReply): () -> Unit =
        subscribe(method, decodePluginValue(payload)) {
            reply(TokamakPluginResult.encoded(it.mapCatching(::encodePluginValue), false))
        }

    /** Returns cancellation for pending work when the platform supports it. */
    fun call(method: String, arguments: Any?, reply: TokamakPluginReply): (() -> Unit)? {
        reply(Result.failure(TokamakPluginError.notSupported("$id.$method is not supported")))
        return null
    }

    /** A call whose [body] of bytes the Worker streams, which the plugin reads one chunk at a time. */
    fun call(method: String, arguments: Any?, body: TokamakPluginBody, reply: TokamakPluginReply): (() -> Unit)? {
        reply(Result.failure(TokamakPluginError.notSupported("$id.$method does not take a body")))
        return null
    }

    fun subscribe(
        method: String,
        arguments: Any?,
        reply: TokamakPluginReply,
    ): () -> Unit {
        throw TokamakPluginError.notSupported("$id.$method is not supported")
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
     * passes [callback] whether each is granted. Answers at once, without UI, when none would
     * prompt; throws `NeedsUIError` when one would while the app is in the background. Call it on
     * the main thread; [callback] runs on the main thread.
     */
    fun requestPermissions(permissions: Set<String>, callback: (Map<String, Boolean>) -> Unit)

    /**
     * Delivers the plugin's event [name], which the Worker's listeners receive as
     * `<plugin>.<name>`, with its encoded [payload], and returns their encoded reply, which is
     * empty when none replied. Starts the runtime when it is not running. Blocks, so call it off
     * the main thread; throws unless the listeners return within [timeoutMillis], which includes
     * runtime startup.
     */
    fun emitBytes(name: String, payload: ByteArray, timeoutMillis: Long): ByteArray

    /** [emitBytes] with the JSON [event], returning their reply as JSON, `null` when none replied. */
    fun emit(name: String, event: String, timeoutMillis: Long): String =
        emitBytes(name, event.toByteArray(Charsets.UTF_8), timeoutMillis).toString(Charsets.UTF_8).ifEmpty { "null" }

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
