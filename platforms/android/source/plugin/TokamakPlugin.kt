package com.tokamak.runtime

import android.app.Activity
import android.content.Context
import android.content.Intent

/**
 * A failure a plugin reports to the page as a `DOMException` named [errorName]. Other exceptions
 * reach the page as an `OperationError`.
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

/**
 * A native plugin, created once per process with the app's [TokamakHost]. A [call] or [subscribe]
 * that throws replies with the exception.
 */
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

/** The app as one plugin sees it: its context, its activity, and its Worker. */
interface TokamakHost {
    /** The application context. */
    val context: Context

    /** The app's activity, or null while it has none. */
    val activity: Activity?

    /** Whether the app is in the foreground, where it can show UI. Read it on the main thread. */
    val isInForeground: Boolean

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
     * `<plugin>.<name>`, with the JSON [event], and returns their reply as JSON, `null` when none
     * replies. Starts the runtime when it is not running. Blocks, so call it off the main thread;
     * throws unless the listeners return within [timeoutMillis], which includes runtime startup.
     */
    fun emit(name: String, event: String, timeoutMillis: Long): String
}

/** The app's plugins. The app's [android.app.Application] is one. */
interface TokamakPlugins {
    /** The plugin with [id], or null when the app does not include it. */
    fun plugin(id: String): TokamakPlugin?
}

/** The plugin with [id] in the app this context belongs to, or null when the app does not include it. */
fun Context.tokamakPlugin(id: String): TokamakPlugin? = (applicationContext as TokamakPlugins).plugin(id)
