package com.tokamak.runtime

import java.util.UUID
import java.util.concurrent.ConcurrentHashMap

/** Native objects that plugins share by ID, each kept while the subscription that retained it is open. */
class TokamakResources {
    private val objects = ConcurrentHashMap<String, Any>()

    /** Keeps [value], replies with its ID, and returns the release to return from `subscribe`. */
    fun retain(value: Any, reply: TokamakPluginReply): () -> Unit {
        val id = UUID.randomUUID().toString()
        objects[id] = value
        reply(Result.success(id))
        return { objects.remove(id) }
    }

    /** The object retained as [id]. */
    fun <T : Any> get(id: Any?, type: Class<T>): T {
        val value = (id as? String)?.let(objects::get)
            ?: throw TokamakPluginError("InvalidStateError", "The native reference is closed")
        if (!type.isInstance(value)) throw TokamakPluginError.typeError("The native reference is not ${type.simpleName}")
        return type.cast(value)!!
    }
}
