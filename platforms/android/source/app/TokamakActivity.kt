package com.tokamak.runtime

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.util.Log
import android.view.ViewGroup
import android.view.WindowInsets
import android.webkit.WebView
import android.widget.FrameLayout
import androidx.webkit.WebViewFeature

private const val TAG = "tokamak"
private const val FAILED =
    "<!doctype html><title>tokamak</title><h1>App failed to start</h1><p>See logcat for details.</p>"
private const val UNSUPPORTED =
    "<!doctype html><title>tokamak</title><h1>Unsupported WebView</h1>" +
        "<p>This device's WebView cannot serve the app's secure origin.</p>"

/** The tokamak window: shows the app's WebView over the process's runtime. */
class TokamakActivity : Activity() {
    private val tokamak: TokamakApplication
        get() = application as TokamakApplication
    internal lateinit var webView: WebView
        private set
    private var destroyed = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        tokamak.activity = this
        webView = WebView(this).apply {
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = true
            settings.setSupportMultipleWindows(false)
        }
        webView.webChromeClient = TokamakWebChromeClient(tokamak, tokamak.appHost)
        setContentView(webViewContainer())
        // A recreated activity or one reopened from Recents carries an intent plugins have had.
        val launchedFromHistory = intent.flags and Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY != 0
        if (savedInstanceState == null && !launchedFromHistory) deliver(intent)
        if (!proxyIsSupported()) {
            show(UNSUPPORTED)
            return
        }
        Thread(::startRuntime, "tokamak-startup").start()
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        deliver(intent)
    }

    override fun onDestroy() {
        destroyed = true
        if (tokamak.activity === this) tokamak.activity = null
        if (isFinishing) tokamak.permissionRequests.cancel()
        webView.stopLoading()
        webView.destroy()
        super.onDestroy()
    }

    override fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray,
    ) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        tokamak.permissionRequests.onRequestPermissionsResult(requestCode)
    }

    private fun deliver(intent: Intent) {
        tokamak.plugins.values.forEach { it.onIntent(intent) }
    }

    private fun startRuntime() {
        val started = runCatching { tokamak.runtime }
        runOnUiThread { finishStart(started) }
    }

    private fun finishStart(started: Result<TokamakRuntime>) {
        if (destroyed) return
        val runtime = started.getOrElse { error ->
            Log.e(TAG, "tokamak failed to start", error)
            show(FAILED)
            return
        }
        TokamakProxy.route(this, tokamak.appHost, runtime.port) { load(runtime) }
    }

    private fun load(runtime: TokamakRuntime) {
        if (destroyed) return
        webView.webViewClient =
            TokamakWebViewClient(this, tokamak.appHost, runtime)
        webView.loadUrl("https://${tokamak.appHost}/")
    }

    private fun proxyIsSupported(): Boolean =
        WebViewFeature.isFeatureSupported(WebViewFeature.PROXY_OVERRIDE) &&
            WebViewFeature.isFeatureSupported(WebViewFeature.PROXY_OVERRIDE_REVERSE_BYPASS)

    private fun show(html: String) = webView.loadData(html, "text/html", "utf-8")

    private fun webViewContainer(): FrameLayout =
        FrameLayout(this).apply {
            addView(
                webView,
                FrameLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.MATCH_PARENT,
                ),
            )
            setOnApplyWindowInsetsListener { view, insets ->
                val bars = insets.getInsets(WindowInsets.Type.systemBars())
                view.setPadding(bars.left, bars.top, bars.right, bars.bottom)
                insets
            }
        }
}
