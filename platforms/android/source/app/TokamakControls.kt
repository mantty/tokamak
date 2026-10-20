package com.tokamak.runtime

import android.net.Uri
import java.net.URI
import java.util.concurrent.Executor
import java.util.concurrent.Executors
import org.json.JSONObject

/** Core device operations use the same endpoint and ownership as plugins. */
internal class TokamakCorePlugin(private val app: TokamakApplication) : TokamakPlugin {
    override val id = "tokamak"
    private val executor = Executors.newSingleThreadExecutor()

    /** WebView operations run on the main thread, which WebView requires. */
    override fun executor(method: String): Executor =
        if (method.startsWith("webview.")) super.executor(method) else executor

    override fun call(method: String, arguments: Any?, reply: TokamakPluginReply): (() -> Unit)? {
        if (method == "resource.read") {
            reply(Result.success(app.resources.get(arguments, ByteArray::class.java)))
            return null
        }
        val view = (app.activity as? TokamakActivity)?.webView
        val page = view?.url?.let(::URI)
        if (view == null || page == null) {
            throw TokamakPluginError("InvalidStateError", "No page is loaded")
        }
        when (method) {
            "webview.getCurrentPage" -> reply(Result.success(page.toString()))
            "webview.navigate" -> {
                val url = page.resolve(arguments as? String
                    ?: throw TokamakPluginError.typeError("Navigation requires an app URL"))
                if (!Uri.parse(url.toString()).isAppOrigin(app.appHost)) {
                    throw TokamakPluginError.typeError("Navigation requires an app URL")
                }
                view.loadUrl(url.toString())
                reply(Result.success(null))
            }
            "webview.postMessage" -> {
                val data = JSONObject().put("data", arguments ?: JSONObject.NULL).toString()
                view.evaluateJavascript(
                    "(() => { const value = JSON.parse(${JSONObject.quote(data)});" +
                        "value.origin = location.origin; window.dispatchEvent(new MessageEvent('message', value)); return true; })()",
                ) { result ->
                    if (result == "true") reply(Result.success(null))
                    else reply(Result.failure(TokamakPluginError("OperationError", "Page message delivery failed")))
                }
            }
            else -> throw TokamakPluginError.notSupported("$id.$method is not supported")
        }
        return null
    }
}
