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
    plugins: Map<String, TokamakPlugin>,
) : WebViewCompat.WebMessageListener {
    private data class RequestKey(val session: String, val id: Int)

    private val requests = TokamakPluginRequests<RequestKey>(plugins, ::send)
    private var activeSession: String? = null
    private var replyProxy: JavaScriptReplyProxy? = null

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
        replyProxy = null
        requests.cancelAll()
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
        val type = request.optString("type")
        if (type == "reset") {
            close()
            activeSession = session
            this.replyProxy = replyProxy
            return
        }
        val id = request.optInt("id", -1)
        if (id < 0) return
        val key = RequestKey(session, id)
        when (type) {
            "cancel" -> requests.cancel(key)
            "call", "subscribe" ->
                requests.run(
                    key,
                    type == "subscribe",
                    request.optString("plugin"),
                    request.optString("method"),
                    request.opt("arguments"),
                )
            else -> requests.fail(key, TokamakPluginError.notSupported("Plugin operation is not supported"))
        }
    }

    /** Passes [result] to the page that made the request [key], if it is still loaded. */
    private fun send(key: RequestKey, result: JSONObject) =
        activity.runOnUiThread {
            if (activeSession != key.session) return@runOnUiThread
            replyProxy?.postMessage(result.put("session", key.session).put("id", key.id).toString())
        }
}
