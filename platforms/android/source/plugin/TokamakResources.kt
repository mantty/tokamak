package com.tokamak.runtime

import java.util.UUID

/** Native objects that plugins share by ID. Each belongs to a scope and is kept until it is released or its scope ends. */
class TokamakResources {
    private val objects = HashMap<String, Pair<Any, String>>()
    private val scopes = HashMap<String, MutableSet<String>>()

    /** Starts a scope and returns its ID. */
    fun open(): String {
        val scope = UUID.randomUUID().toString()
        synchronized(this) { scopes[scope] = HashSet() }
        return scope
    }

    /** Keeps [value] in [scope] and returns its ID. An ended scope keeps nothing, so the ID finds no object. */
    fun retain(value: Any, scope: String): String {
        val id = UUID.randomUUID().toString()
        synchronized(this) {
            if (scopes[scope]?.add(id) == true) objects[id] = value to scope
        }
        return id
    }

    /** Keeps [value] in a scope of its own, replies with its ID, and returns the release to return from `subscribe`. */
    fun retain(value: Any, reply: TokamakPluginReply): () -> Unit {
        val scope = open()
        reply(Result.success(retain(value, scope)))
        return { end(scope) }
    }

    /** Releases the object retained as [id], if it is still kept. */
    fun release(id: Any?) {
        synchronized(this) {
            val (_, scope) = objects.remove(id as? String) ?: return
            scopes[scope]?.remove(id)
        }
    }

    /** Releases every object [scope] keeps, and keeps nothing in it later. */
    fun end(scope: String) {
        synchronized(this) { scopes.remove(scope)?.forEach { objects.remove(it) } }
    }

    /** The object retained as [id]. */
    fun <T : Any> get(id: Any?, type: Class<T>): T {
        val value = synchronized(this) { objects[id as? String]?.first }
            ?: throw TokamakPluginError("InvalidStateError", "The native reference is closed")
        if (!type.isInstance(value)) throw TokamakPluginError.typeError("The native reference is not ${type.simpleName}")
        return type.cast(value)!!
    }
}
