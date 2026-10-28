package com.tokamak.runtime

import android.content.Context
import androidx.webkit.ProxyConfig
import androidx.webkit.ProxyController

/**
 * Routes the app host, in every WebView of the process, through the gateway's port. The gateway
 * can lose its port and bind another, so the route follows whichever port it reports. Without a
 * route the WebView would connect to the app host itself, where another app or device could
 * answer. Main thread only.
 */
internal object TokamakProxy {
    private var routed: Int? = null
    private var requested: Int? = null
    private val waiting = mutableListOf<() -> Unit>()

    /**
     * Routes [host] through the gateway's [port], then runs [ready] once the route is in place. A
     * call for another port while a route is pending replaces it, and every waiting [ready] runs
     * once the latest route is in place.
     */
    fun route(context: Context, host: String, port: Int, ready: () -> Unit) {
        if (routed == port && requested == port) {
            ready()
            return
        }
        waiting += ready
        if (requested == port) return
        requested = port
        val config = ProxyConfig.Builder()
            .addProxyRule("http://127.0.0.1:$port")
            .addBypassRule(host)
            .setReverseBypassEnabled(true)
            .build()
        ProxyController.getInstance().setProxyOverride(config, context.mainExecutor) {
            routed = port
            if (requested != port) return@setProxyOverride
            waiting.forEach { it() }
            waiting.clear()
        }
    }
}
