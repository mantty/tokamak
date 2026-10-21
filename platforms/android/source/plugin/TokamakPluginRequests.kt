package com.tokamak.runtime

import java.util.concurrent.Executor

/** What the Worker asked a plugin for. */
enum class TokamakPluginRequest { CALL, CALL_WITH_BODY, SUBSCRIBE }

/**
 * Runs the Worker's requests on the executor each plugin chooses, and passes replies straight to the
 * runtime with [send]; [pull] asks the Worker for the next chunk of a call's body.
 */
class TokamakPluginRequests<K>(
    private val plugins: Map<String, TokamakPlugin>,
    private val pull: (K) -> Unit = {},
    private val send: (K, TokamakPluginResult) -> Unit,
) {
    /** A pending call or subscription. Only its serial executor touches [cancellation], so a cancellation runs after the operation started. */
    private class Request(val executor: Executor) {
        var body: TokamakPluginBody? = null
        var cancellation: (() -> Unit)? = null
    }

    private val pending = mutableMapOf<K, Request>()

    fun run(key: K, kind: TokamakPluginRequest, plugin: String, method: String, payload: ByteArray) {
        val target = plugins[plugin]
        if (target == null || method.isEmpty()) {
            send(key, failure(TokamakPluginError.notSupported("Plugin is not supported")))
            return
        }
        val request = Request(target.executor(method))
        if (kind == TokamakPluginRequest.CALL_WITH_BODY) request.body = TokamakPluginBody(request.executor) { pull(key) }
        synchronized(pending) { pending[key] = request }
        val subscribe = kind == TokamakPluginRequest.SUBSCRIBE
        val reply: TokamakPluginBytesReply = { result ->
            val last = !subscribe || result.done
            if (if (last) remove(key, request) else isPending(key, request)) {
                send(key, result)
                if (subscribe && result.done) cancel(request)
            }
        }
        request.executor.execute {
            if (!isPending(key, request)) return@execute
            try {
                val body = request.body
                request.cancellation = when {
                    subscribe -> target.subscribeBytes(method, payload, reply)
                    body != null -> target.callBytes(method, payload, body, reply)
                    else -> target.callBytes(method, payload, reply)
                }
            } catch (error: Exception) {
                if (remove(key, request)) send(key, failure(error))
            }
        }
    }

    /** Answers the pending read of the call [key]'s body with the chunk the Worker sent. */
    fun chunk(key: K, chunk: TokamakPluginResult) {
        synchronized(pending) { pending[key]?.body }?.receive(chunk)
    }

    fun cancel(key: K) {
        synchronized(pending) { pending.remove(key) }?.let(::cancel)
    }

    fun cancelAll() {
        val requests = synchronized(pending) { pending.values.toList().also { pending.clear() } }
        requests.forEach(::cancel)
    }

    /** Runs the operation's cancellation on its executor. */
    private fun cancel(request: Request) {
        request.executor.execute {
            request.cancellation?.invoke()
            request.cancellation = null
        }
    }

    private fun isPending(key: K, request: Request) = synchronized(pending) { pending[key] === request }

    /** Removes [request], returning whether it was pending. */
    private fun remove(key: K, request: Request) = synchronized(pending) {
        (pending[key] === request).also { if (it) pending.remove(key) }
    }

    private fun failure(error: Throwable) = TokamakPluginResult.encoded(Result.failure(error), done = true)
}
