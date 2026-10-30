package com.tokamak.runtime

import java.util.UUID

/** Native objects that plugins share by ID. Each is kept until the subscription that retained it ends. */
class TokamakResources {
    private val objects = HashMap<String, Any>()

    /** Keeps [value], replies with its ID, and returns the release to return from `subscribe`. */
    fun retain(value: Any, reply: TokamakPluginBytesReply): () -> Unit {
        val id = UUID.randomUUID().toString()
        synchronized(this) { objects[id] = value }
        reply(TokamakPluginResult(encodePluginValue(id), false, false))
        return { synchronized(this) { objects.remove(id) } }
    }

    /** The object retained as [id]. */
    fun <T : Any> get(id: Any?, type: Class<T>): T {
        val value = synchronized(this) { objects[id as? String] }
            ?: throw TokamakPluginError("InvalidStateError", "The native reference is closed")
        if (!type.isInstance(value)) throw TokamakPluginError.typeError("The native reference is not ${type.simpleName}")
        return type.cast(value)!!
    }
}
