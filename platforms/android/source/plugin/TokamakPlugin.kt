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

    fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray,
    ) = Unit

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
     * Runs the Worker's [event] handler with a JSON [payload], including work it passes to
     * `ctx.waitUntil`, and returns the handler's JSON result; null when the Worker has no
     * handler for [event]. Starts the runtime when it is not running. Blocks, so call it
     * off the main thread; throws when the handler fails.
     */
    fun dispatch(event: String, payload: String): String?

    /** The plugin with [id], or null when the app does not include it. */
    fun plugin(id: String): TokamakPlugin?
}

/** The host of the app this context belongs to. */
val Context.tokamakHost: TokamakHost
    get() = applicationContext as TokamakHost
