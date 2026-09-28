package com.tokamak.runtime

import androidx.webkit.ProxyConfig
import androidx.webkit.ProxyController

internal object TokamakProxy {
    private var active: Request? = null
    private var desired: Request? = null
    private var changing = false

    fun acquire(activity: TokamakActivity, host: String, port: Int) {
        desired = Request(activity, host, port)
        reconcile()
    }

    fun release(activity: TokamakActivity) {
        if (desired?.activity === activity) desired = null
        reconcile()
    }

    private fun reconcile() {
        if (changing || active === desired) return
        val current = active
        if (current != null) {
            clear(current)
            return
        }
        desired?.let(::set)
    }

    private fun clear(request: Request) {
        changing = true
        ProxyController.getInstance().clearProxyOverride(request.activity.mainExecutor) {
            active = null
            changing = false
            reconcile()
        }
    }

    private fun set(request: Request) {
        changing = true
        ProxyController.getInstance().setProxyOverride(
            proxyConfig(request),
            request.activity.mainExecutor,
        ) {
            active = request
            changing = false
            if (desired === request) request.activity.proxyReady()
            reconcile()
        }
    }

    private fun proxyConfig(request: Request): ProxyConfig =
        ProxyConfig.Builder()
            .addProxyRule("http://127.0.0.1:${request.port}")
            .addBypassRule(request.host)
            .setReverseBypassEnabled(true)
            .build()

    private class Request(
        val activity: TokamakActivity,
        val host: String,
        val port: Int,
    )
}
