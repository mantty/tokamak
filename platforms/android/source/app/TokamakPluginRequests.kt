package com.tokamak.runtime

import android.os.Handler
import android.os.Looper

/** Runs the Tokamak worker's byte requests on the main thread. */
internal class TokamakPluginRequests<K>(
    private val plugins: Map<String, TokamakPlugin>,
    private val send: (K, TokamakPluginResult) -> Unit,
) {
    private val cancellations = mutableMapOf<K, () -> Unit>()
    private val main = Handler(Looper.getMainLooper())

    /** Calls [method] of [plugin], or subscribes to it when [subscribe]. */
    fun run(key: K, subscribe: Boolean, plugin: String, method: String, payload: ByteArray) {
        val target = plugins[plugin]
        if (target == null || method.isEmpty()) {
            fail(key, TokamakPluginError.notSupported("Plugin is not supported"))
            return
        }
        cancellations[key] = {}
        val reply: TokamakPluginBytesReply = { result ->
            val snapshot = result.copy(payload = result.payload.copyOf())
            main.post {
                if (!cancellations.containsKey(key)) return@post
                if (!subscribe || snapshot.done) cancellations.remove(key)
                send(key, snapshot)
            }
        }
        try {
            if (subscribe) {
                cancellations[key] = target.subscribeBytes(method, payload, reply)
            } else {
                cancellations[key] = target.callBytes(method, payload, reply) ?: {}
            }
        } catch (error: Exception) {
            fail(key, error)
        }
    }

    /** Ends the request [key] with [error]. */
    fun fail(key: K, error: Throwable) {
        cancel(key)
        send(key, TokamakPluginResult.encoded(Result.failure(error), done = true))
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
}
