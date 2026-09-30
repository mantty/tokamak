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
    private lateinit var webView: WebView
    private lateinit var pluginBridge: TokamakPluginBridge
    private lateinit var chromeClient: TokamakWebChromeClient
    private var runtime: TokamakRuntime? = null
    private var proxyPort: Int? = null
    private var restoreGeneration = 0L
    private var destroyed = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        tokamak.activity = this
        webView = WebView(this).apply {
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = true
            settings.setSupportMultipleWindows(false)
        }
        chromeClient = TokamakWebChromeClient(this, tokamak.appHost)
        webView.webChromeClient = chromeClient
        pluginBridge = TokamakPluginBridge(this, tokamak.appHost, tokamak.plugins)
        if (WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER)) {
            pluginBridge.install(webView)
        }
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

    override fun onResume() {
        super.onResume()
        restoreGeneration += 1
        val generation = restoreGeneration
        runtime?.let { restoreGateway(it, generation) }
    }

    override fun onDestroy() {
        destroyed = true
        restoreGeneration += 1
        if (tokamak.activity === this) tokamak.activity = null
        TokamakProxy.release(this)
        pluginBridge.close()
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
        chromeClient.onRequestPermissionsResult(requestCode)
        tokamak.plugins.values.forEach {
            it.onRequestPermissionsResult(requestCode, permissions, grantResults)
        }
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
        this.runtime = runtime
        proxyPort = runtime.port
        TokamakProxy.acquire(this, tokamak.appHost, runtime.port)
    }

    private fun restoreGateway(runtime: TokamakRuntime, generation: Long) {
        Thread {
            val result = runCatching { runtime.restoreGateway() }
            runOnUiThread {
                if (
                    destroyed ||
                    this.runtime !== runtime ||
                    generation != restoreGeneration
                ) return@runOnUiThread
                result
                    .onSuccess { port ->
                        if (port == runtime.port && port != proxyPort) {
                            proxyPort = port
                            TokamakProxy.acquire(this, tokamak.appHost, port)
                        }
                    }
                    .onFailure { error ->
                        Log.w(TAG, "tokamak gateway could not recover", error)
                    }
            }
        }.start()
    }

    internal fun proxyReady() {
        if (destroyed) return
        val runtime = runtime ?: return
        webView.webViewClient =
            TokamakWebViewClient(this, tokamak.appHost, runtime, pluginBridge)
        webView.loadUrl("https://${tokamak.appHost}/")
    }

    private fun proxyIsSupported(): Boolean =
        WebViewFeature.isFeatureSupported(WebViewFeature.PROXY_OVERRIDE) &&
            WebViewFeature.isFeatureSupported(WebViewFeature.PROXY_OVERRIDE_REVERSE_BYPASS) &&
            WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER)

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
