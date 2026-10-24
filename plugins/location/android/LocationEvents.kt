package com.tokamak.plugins.location

import android.util.Log
import com.tokamak.runtime.TokamakHost
import com.tokamak.runtime.TokamakListeners
import com.tokamak.runtime.TokamakPluginError

/** How long the Worker's listeners have for an event, within the time Android gives a broadcast. */
private const val EVENT_DEADLINE_MILLIS = 8_000L

/** Delivers the plugin's events to the Worker's listeners and the page's. Use it on the main thread. */
internal class LocationEvents(private val host: TokamakHost) {
    val updates = TokamakListeners<LocationUpdate>()
    val transitions = TokamakListeners<RegionTransition>()
    val errors = TokamakListeners<LocationErrorEvent>()

    /** Positions that arrived while the Worker handled the previous update, and the broadcasts to finish once it has them. */
    private val positions = mutableListOf<Position>()
    private val finishes = mutableListOf<() -> Unit>()
    private var updating = false

    /** The time of the newest position delivered, as requests that run together receive the same positions. */
    private var latest = Long.MIN_VALUE

    /**
     * Delivers the [received] positions newer than those delivered, then calls [finish]. Positions
     * that arrive while the Worker handles an update reach it together as the next.
     */
    fun update(received: List<Position>, finish: () -> Unit) {
        val newer = received.filter { it.timestamp > latest }
        if (newer.isEmpty()) return finish()
        latest = newer.maxOf { it.timestamp }
        positions += newer
        finishes += finish
        if (!updating) sendUpdate()
    }

    fun transition(id: String, entered: Boolean, finish: () -> Unit) {
        val transition = RegionTransition(id, if (entered) Transition.Enter else Transition.Exit, System.currentTimeMillis())
        transitions.send(transition)
        send("region", transition, finish)
    }

    /** Reports a background service's failure, then calls [finish]. */
    fun fail(error: TokamakPluginError, finish: () -> Unit = {}) {
        val event = LocationErrorEvent(error.errorName, error.message.orEmpty())
        errors.send(event)
        send("error", event, finish)
    }

    /** Sends the waiting positions as one update, then the positions that arrive meanwhile as the next. */
    private fun sendUpdate() {
        updating = positions.isNotEmpty()
        if (!updating) return
        val update = LocationUpdate(positions.toList())
        val delivered = finishes.toList()
        positions.clear()
        finishes.clear()
        updates.send(update)
        send("update", update) {
            delivered.forEach { it() }
            sendUpdate()
        }
    }

    /** Delivers [event] to the Worker's [name] listeners, then calls [finish]. */
    private fun send(name: String, event: Any, finish: () -> Unit) {
        Thread {
            runCatching { host.emit(name, event, EVENT_DEADLINE_MILLIS) }
                .onFailure { Log.w("tokamak", "location $name failed: ${it.message}") }
            host.context.mainExecutor.execute(finish)
        }.start()
    }
}
