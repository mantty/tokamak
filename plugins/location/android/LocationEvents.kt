package com.tokamak.plugins.location

import android.util.Log
import com.tokamak.runtime.TokamakHost
import com.tokamak.runtime.TokamakPluginError
import com.tokamak.runtime.TokamakPluginReply
import org.json.JSONArray
import org.json.JSONObject

/** How long the Worker's listeners have for an event, within the time Android gives a broadcast. */
private const val EVENT_DEADLINE_MILLIS = 8_000L

/** The listener methods, which receive the Worker's events. */
private val LISTENERS = setOf("onLocationUpdate", "onRegionTransition", "onVisit", "onLocationError")

/** Delivers the plugin's events to the Worker's listeners and the page's. Use it on the main thread. */
internal class LocationEvents(private val host: TokamakHost) {
    private val listeners = mutableMapOf<String, MutableMap<Int, TokamakPluginReply>>()
    private var nextListener = 0

    /** Positions that arrived while the Worker handled the previous update, and the broadcasts to finish once it has them. */
    private val positions = mutableListOf<JSONObject>()
    private val finishes = mutableListOf<() -> Unit>()
    private var updating = false

    /** The time of the newest position delivered, as requests that run together receive the same positions. */
    private var latest = Long.MIN_VALUE

    /** Adds a page's or Worker's listener of [method], returning its removal, or null for another method. */
    fun listen(method: String, reply: TokamakPluginReply): (() -> Unit)? {
        if (method !in LISTENERS) return null
        val key = nextListener++
        listeners.getOrPut(method, ::mutableMapOf)[key] = reply
        return { listeners[method]?.remove(key) }
    }

    /**
     * Delivers the [received] positions newer than those delivered, then calls [finish]. Positions
     * that arrive while the Worker handles an update reach it together as the next.
     */
    fun update(received: List<JSONObject>, finish: () -> Unit) {
        val newer = received.filter { it.getLong("timestamp") > latest }
        if (newer.isEmpty()) return finish()
        latest = newer.maxOf { it.getLong("timestamp") }
        positions += newer
        finishes += finish
        if (!updating) sendUpdate()
    }

    fun transition(id: String, entered: Boolean, finish: () -> Unit) {
        val event =
            JSONObject()
                .put("id", id)
                .put("transition", if (entered) "enter" else "exit")
                .put("timestamp", System.currentTimeMillis())
        send("onRegionTransition", "region", event, finish)
    }

    /** Reports a background service's failure, then calls [finish]. */
    fun fail(error: TokamakPluginError, finish: () -> Unit = {}) {
        val event = JSONObject().put("name", error.errorName).put("message", error.message)
        send("onLocationError", "error", event, finish)
    }

    /** Sends the waiting positions as one update, then the positions that arrive meanwhile as the next. */
    private fun sendUpdate() {
        updating = positions.isNotEmpty()
        if (!updating) return
        val event = JSONObject().put("positions", JSONArray(positions.toList()))
        val delivered = finishes.toList()
        positions.clear()
        finishes.clear()
        send("onLocationUpdate", "update", event) {
            delivered.forEach { it() }
            sendUpdate()
        }
    }

    /** Delivers [event] to the page's [method] listeners and the Worker's [name] listeners, then calls [finish]. */
    private fun send(method: String, name: String, event: JSONObject, finish: () -> Unit) {
        listeners[method].orEmpty().values.toList().forEach { it(Result.success(event)) }
        Thread {
            runCatching { host.emit(name, event.toString(), EVENT_DEADLINE_MILLIS) }
                .onFailure { Log.w("tokamak", "location $name failed: ${it.message}") }
            host.context.mainExecutor.execute(finish)
        }.start()
    }
}
