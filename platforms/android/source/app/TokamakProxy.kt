package com.tokamak.runtime

import android.content.Context
import androidx.webkit.ProxyConfig
import androidx.webkit.ProxyController

/**
 * Routes the app host, in every WebView of the process, through the gateway's port. The gateway
 * keeps one port for the life of the process, so the route is set once and never cleared: without
 * it the WebView would connect to the app host itself, where another app or device could answer.
 * Main thread only.
 */
internal object TokamakProxy {
    private var routed = false
    private val waiting = mutableListOf<() -> Unit>()

    /** Routes [host] through the gateway's [port], then runs [ready] once the route is in place. */
    fun route(context: Context, host: String, port: Int, ready: () -> Unit) {
        if (routed) {
            ready()
            return
        }
        waiting += ready
        // Only the first caller requests the route.
        if (waiting.size > 1) return
        val config = ProxyConfig.Builder()
            .addProxyRule("http://127.0.0.1:$port")
            .addBypassRule(host)
            .setReverseBypassEnabled(true)
            .build()
        ProxyController.getInstance().setProxyOverride(config, context.mainExecutor) {
            routed = true
            waiting.forEach { it() }
            waiting.clear()
        }
    }
}
