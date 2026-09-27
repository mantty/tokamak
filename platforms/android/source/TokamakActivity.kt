package com.tokamak.runtime

import android.app.Activity
import android.content.pm.PackageManager
import android.os.Bundle
import android.util.Log
import android.view.ViewGroup
import android.view.WindowInsets
import android.webkit.WebView
import android.widget.FrameLayout
import androidx.webkit.WebViewFeature
import java.io.File

private const val TAG = "tokamak"
private const val HOST_METADATA = "tokamak.host"
private const val DEV_ENDPOINT_METADATA = "tokamak.dev.endpoint"
private const val DEV_SESSION_TOKEN_METADATA = "tokamak.dev.session-token"
private const val STARTING = "<!doctype html><title>Starting</title>"
private const val FAILED =
    "<!doctype html><title>tokamak</title><h1>App failed to start</h1><p>See logcat for details.</p>"
private const val UNSUPPORTED =
    "<!doctype html><title>tokamak</title><h1>Unsupported WebView</h1>" +
        "<p>This device's WebView cannot serve the app's secure origin.</p>"

/** The tokamak application: owns the window, WebView, and runtime lifecycle. */
class TokamakActivity : Activity() {
    private lateinit var webView: WebView
    private lateinit var pluginBridge: TokamakPluginBridge
    private lateinit var chromeClient: TokamakWebChromeClient
    private var runtime: TokamakRuntime? = null
    private var proxyPort: Int? = null
    private var restoreGeneration = 0L
    private var destroyed = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        webView = WebView(this).apply {
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = true
            settings.setSupportMultipleWindows(false)
        }
        chromeClient = TokamakWebChromeClient(this, appHost())
        webView.webChromeClient = chromeClient
        pluginBridge = TokamakPluginBridge(this, appHost(), tokamakPlugins(this))
        if (WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER)) {
            pluginBridge.install(webView)
        }
        setContentView(webViewContainer())
        if (!proxyIsSupported()) {
            show(UNSUPPORTED)
            return
        }
        show(STARTING)
        Thread(::startRuntime, "tokamak-startup").start()
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
        TokamakProxy.release(this)
        pluginBridge.close()
        webView.stopLoading()
        webView.destroy()
        runtime?.stop()
        runtime = null
        super.onDestroy()
    }

    override fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray,
    ) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        chromeClient.onRequestPermissionsResult(requestCode)
        pluginBridge.onRequestPermissionsResult(requestCode, permissions, grantResults)
    }

    private fun startRuntime() {
        val started = runCatching { startConfiguredRuntime() }
        runOnUiThread { finishStart(started) }
    }

    private fun startConfiguredRuntime(): TokamakRuntime {
        val metadata = packageManager
            .getApplicationInfo(packageName, PackageManager.GET_META_DATA)
            .metaData
        val endpoint = metadata?.getString(DEV_ENDPOINT_METADATA)
        val sessionToken = metadata?.getString(DEV_SESSION_TOKEN_METADATA)
        return if (!endpoint.isNullOrEmpty() && !sessionToken.isNullOrEmpty()) {
            TokamakRuntime.startDevelopment(stateDir(), appHost(), endpoint, sessionToken)
        } else {
            TokamakRuntime.start(unpackApp(), stateDir(), appHost())
        }
    }

    private fun finishStart(started: Result<TokamakRuntime>) {
        if (destroyed) {
            started.getOrNull()?.stop()
            return
        }
        val runtime = started.getOrElse { error ->
            Log.e(TAG, "tokamak failed to start", error)
            show(FAILED)
            return
        }
        this.runtime = runtime
        proxyPort = runtime.port
        TokamakProxy.acquire(this, appHost(), runtime.port)
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
                        if (port != runtime.port) return@onSuccess
                        if (port != proxyPort) {
                            proxyPort = port
                            TokamakProxy.acquire(this, appHost(), port)
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
        webView.webViewClient = TokamakWebViewClient(this, appHost(), runtime, pluginBridge)
        webView.loadUrl("https://${appHost()}/")
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

    /** Copy the packaged app out of the APK so the runtime can read it as files. */
    private fun unpackApp(): File {
        val app = File(filesDir, "tokamak/app")
        app.deleteRecursively()
        copyAsset("app", app)
        return app
    }

    private fun copyAsset(source: String, destination: File) {
        val entries = assets.list(source) ?: emptyArray()
        if (entries.isEmpty()) {
            destination.parentFile?.mkdirs()
            assets.open(source).use { input ->
                destination.outputStream().use(input::copyTo)
            }
            return
        }
        destination.mkdirs()
        for (entry in entries) copyAsset("$source/$entry", File(destination, entry))
    }

    private fun stateDir(): File = File(filesDir, "tokamak/state").apply { mkdirs() }

    private fun appHost(): String {
        val info = packageManager.getApplicationInfo(packageName, PackageManager.GET_META_DATA)
        return requireNotNull(info.metaData?.getString(HOST_METADATA)) {
            "$HOST_METADATA is required"
        }
    }

}
