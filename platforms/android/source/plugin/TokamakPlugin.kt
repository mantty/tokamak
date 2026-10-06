package com.tokamak.runtime

import android.app.Activity
import android.content.Context
import android.content.Intent

/** A failure a plugin reports to the page as a `DOMException` named [errorName]. */
class TokamakPluginError(
    val errorName: String,
    message: String,
) : Exception(message) {
    companion object {
        fun notSupported(message: String) = TokamakPluginError("NotSupportedError", message)
    }
}

typealias TokamakPluginReply = (Result<Any?>) -> Unit

/** A native plugin, created once per process with the app's [TokamakHost]. */
interface TokamakPlugin {
    val id: String

    fun call(method: String, arguments: Any?, reply: TokamakPluginReply) {
        reply(Result.failure(TokamakPluginError.notSupported("$id.$method is not supported")))
    }

    fun subscribe(
        method: String,
        arguments: Any?,
        reply: TokamakPluginReply,
    ): () -> Unit {
        reply(Result.failure(TokamakPluginError.notSupported("$id.$method is not supported")))
        return {}
    }

    /** Receives the intent that started the app's activity, and each intent it receives later. */
    fun onIntent(intent: Intent) = Unit
}

/** The app as a plugin sees it: its context, its activity, and its Worker. */
interface TokamakHost {
    /** The application context. */
    val context: Context

    /** The app's activity, or null while it has none. */
    val activity: Activity?

    /**
     * Asks the user for [permissions] in the app's activity once earlier requests finish, then
     * passes [callback] whether each is granted. Without an activity, answers at once. Call it on
     * the main thread; [callback] runs on the main thread.
     */
    fun requestPermissions(permissions: Set<String>, callback: (Map<String, Boolean>) -> Unit)

    /**
     * Posts the JSON [body] to the Worker's `/tokamak/<name>` endpoint and returns the
     * response body. Starts the runtime when it is not running. Blocks, so call it off the main
     * thread; throws unless the Worker responds 200 within [timeoutMillis].
     */
    fun call(name: String, body: String, timeoutMillis: Long): String

    /** The plugin with [id], or null when the app does not include it. */
    fun plugin(id: String): TokamakPlugin?
}

/** The host of the app this context belongs to. */
val Context.tokamakHost: TokamakHost
    get() = applicationContext as TokamakHost
