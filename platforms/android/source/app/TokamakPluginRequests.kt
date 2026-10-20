package com.tokamak.runtime

import android.os.Handler
import android.os.Looper
import org.json.JSONObject

/**
 * Runs one caller's calls and subscriptions, the page's or the Worker's, on the app's plugins,
 * keyed by [K]. [send] receives each result on the main thread as the JSON
 * object the page's native transport receives without its session and ID. Use it on the main
 * thread.
 */
internal class TokamakPluginRequests<K>(
    private val plugins: Map<String, TokamakPlugin>,
    private val send: (K, JSONObject) -> Unit,
) {
    private val cancellations = mutableMapOf<K, () -> Unit>()
    private val main = Handler(Looper.getMainLooper())

    /** Calls [method] of [plugin], or subscribes to it when [subscribe]. */
    fun run(key: K, subscribe: Boolean, plugin: String, method: String, arguments: Any?) {
        val target = plugins[plugin]
        if (target == null || method.isEmpty()) {
            fail(key, TokamakPluginError.notSupported("Plugin is not supported"))
            return
        }
        cancellations[key] = {}
        val reply: TokamakPluginReply = { result ->
            main.post {
                if (!cancellations.containsKey(key)) return@post
                if (!subscribe) cancellations.remove(key)
                send(key, result(result, done = !subscribe))
            }
        }
        try {
            if (subscribe) {
                cancellations[key] = target.subscribe(method, arguments, reply)
            } else {
                cancellations[key] = target.call(method, arguments, reply) ?: {}
            }
        } catch (error: Exception) {
            fail(key, error)
        }
    }

    /** Ends the request [key] with [error]. */
    fun fail(key: K, error: Throwable) {
        cancel(key)
        send(key, result(Result.failure(error), done = true))
    }

    /** Cancels the pending call or subscription [key]. */
    fun cancel(key: K) {
        cancellations.remove(key)?.invoke()
    }

    /** Cancels every pending call and subscription. */
    fun cancelAll() {
        val cancelAll = cancellations.values.toList()
        cancellations.clear()
        cancelAll.forEach { it() }
    }

    private fun result(result: Result<Any?>, done: Boolean): JSONObject {
        val json = JSONObject().put("done", done)
        val error = result.exceptionOrNull() ?: return json.put("value", JSONObject.wrap(result.getOrNull()))
        val name = (error as? TokamakPluginError)?.errorName ?: "OperationError"
        return json.put("error", JSONObject().put("name", name).put("message", error.message ?: error.javaClass.name))
    }
}
