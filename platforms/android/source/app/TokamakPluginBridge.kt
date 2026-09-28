package com.tokamak.runtime

import android.app.Activity
import android.net.Uri
import android.webkit.WebView
import androidx.webkit.JavaScriptReplyProxy
import androidx.webkit.WebMessageCompat
import androidx.webkit.WebViewCompat
import org.json.JSONObject

/** Carries calls and subscriptions between the page and the app's plugins. */
internal class TokamakPluginBridge(
    private val activity: Activity,
    private val host: String,
    private val plugins: Map<String, TokamakPlugin>,
) : WebViewCompat.WebMessageListener {
    private data class RequestKey(val session: String, val id: Int)

    private val cancellations = mutableMapOf<RequestKey, () -> Unit>()
    private var activeSession: String? = null

    fun install(webView: WebView) {
        WebViewCompat.addWebMessageListener(
            webView,
            "__tokamakNative",
            setOf("https://$host"),
            this,
        )
    }

    fun close() {
        activeSession = null
        val cancelAll = cancellations.values.toList()
        cancellations.clear()
        cancelAll.forEach { it() }
    }

    override fun onPostMessage(
        view: WebView,
        message: WebMessageCompat,
        sourceOrigin: Uri,
        isMainFrame: Boolean,
        replyProxy: JavaScriptReplyProxy,
    ) {
        if (!isMainFrame || !sourceOrigin.isAppOrigin(host)) return
        val request = runCatching { JSONObject(message.data ?: return) }.getOrNull() ?: return
        val session = request.optString("session")
        if (session.isEmpty()) return
        if (request.optString("type") == "reset") {
            close()
            activeSession = session
            return
        }
        val id = request.optInt("id", -1)
        if (id < 0) return
        val key = RequestKey(session, id)
        when (request.optString("type")) {
            "cancel" -> cancellations.remove(key)?.invoke()
            "call", "subscribe" -> dispatch(request, key, replyProxy)
            else ->
                send(
                    replyProxy,
                    key,
                    Result.failure(
                        TokamakPluginError.notSupported("Plugin operation is not supported"),
                    ),
                    true,
                )
        }
    }

    private fun dispatch(
        request: JSONObject,
        key: RequestKey,
        replyProxy: JavaScriptReplyProxy,
    ) {
        val plugin = plugins[request.optString("plugin")]
        val method = request.optString("method")
        if (plugin == null || method.isEmpty()) {
            send(
                replyProxy,
                key,
                Result.failure(TokamakPluginError.notSupported("Plugin is not supported")),
                true,
            )
            return
        }
        val arguments = request.opt("arguments")
        val reply: TokamakPluginReply = { result ->
            send(replyProxy, key, result, request.optString("type") == "call")
        }
        if (request.optString("type") == "call") {
            plugin.call(method, arguments, reply)
        } else {
            val cancellation = plugin.subscribe(method, arguments, reply)
            cancellations[key] = cancellation
        }
    }

    private fun send(
        replyProxy: JavaScriptReplyProxy,
        key: RequestKey,
        result: Result<Any?>,
        done: Boolean,
    ) = activity.runOnUiThread { deliver(replyProxy, key, result, done) }

    private fun deliver(
        replyProxy: JavaScriptReplyProxy,
        key: RequestKey,
        result: Result<Any?>,
        done: Boolean,
    ) {
        if (activeSession != key.session) return
        val error = result.exceptionOrNull()
        if (done) cancellations.remove(key)?.invoke()
        val response =
            JSONObject()
                .put("session", key.session)
                .put("id", key.id)
                .put("done", done)
        if (error == null) {
            response.put("value", JSONObject.wrap(result.getOrNull()))
        } else {
            val name = (error as? TokamakPluginError)?.errorName ?: "UnknownError"
            response.put(
                "error",
                JSONObject().put("name", name).put("message", error.message ?: name),
            )
        }
        replyProxy.postMessage(response.toString())
    }
}
