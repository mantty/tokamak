package com.tokamak.plugins.location

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import com.tokamak.runtime.tokamakPlugin

/** Receives updates and region transitions, which Android sends even after the app's process ended. */
class TokamakLocationReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val plugin = context.tokamakPlugin("location") as? TokamakLocationPlugin ?: return
        val pending = goAsync()
        plugin.receive(intent) { pending.finish() }
    }
}

/** Requests updates and regions again after a restart or an app update, which remove them. */
class TokamakLocationBootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action !in ACTIONS) return
        (context.tokamakPlugin("location") as? TokamakLocationPlugin)?.restore(always = true)
    }

    private companion object {
        val ACTIONS = setOf(Intent.ACTION_BOOT_COMPLETED, Intent.ACTION_MY_PACKAGE_REPLACED)
    }
}
