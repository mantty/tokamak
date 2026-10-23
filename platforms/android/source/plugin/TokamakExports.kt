package com.tokamak.runtime

import kotlin.coroutines.cancellation.CancellationException
import kotlin.enums.EnumEntries
import kotlin.reflect.KClass
import kotlin.reflect.KFunction
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.onSubscription
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject

/**
 * The methods a plugin's `android/Exports.kt` lists for the Worker to call, and the events the Worker
 * receives. `tok plugin generate android` runs [exports] to write the plugin's dispatch and
 * TypeScript API; the app never does.
 */
interface TokamakExports<P : TokamakPlugin> {
    fun exports(e: TokamakExporter<P>)
}

/** What a stream does with values a slow reader has not taken. */
enum class TokamakOverflow {
    /** Ends the stream with `QuotaExceededError`. */
    ERROR,

    /** Drops the oldest values, for samples where only recent ones matter. */
    DROP_OLDEST,
}

/** Records the methods a plugin exports, with their options, and the events it emits. */
class TokamakExporter<P : TokamakPlugin> internal constructor() {
    internal class Export(val kind: String, val method: KFunction<*>, val doc: String, val overflow: TokamakOverflow)

    internal class Event(val name: String, val type: KClass<*>, val reply: KClass<*>?, val doc: String)

    internal val exports = mutableListOf<Export>()
    internal val events = mutableListOf<Event>()

    /** Lists the event [name], whose listeners receive a [type] and may reply with a [reply]. */
    fun event(name: String, type: KClass<*>, reply: KClass<*>? = null, doc: String = "") {
        events += Event(name, type, reply, doc)
    }

    /** Exports [method], which returns once. */
    fun call(method: KFunction<*>, doc: String = "") {
        exports += Export("call", method, doc, TokamakOverflow.ERROR)
    }

    /** Exports [method], whose values the Worker reads until it cancels. */
    fun stream(method: KFunction<Flow<*>>, overflow: TokamakOverflow = TokamakOverflow.ERROR, doc: String = "") {
        exports += Export("stream", method, doc, overflow)
    }

    /** Exports [method], which returns a native object the Worker holds as a `NativeReference`. */
    fun resource(method: KFunction<Any>, doc: String = "") {
        exports += Export("resource", method, doc, TokamakOverflow.ERROR)
    }
}

/** How many values listeners keep: those a slow reader has not taken, or those held for the first. */
private const val LISTENER_CAPACITY = 64

/**
 * The flows of an event's listeners, such as a page's, each of which receives every value [send]
 * delivers while it collects, keeping the newest 64 that a slow one has not taken. When [holding],
 * the newest 64 values sent while none listen wait for the first listener.
 */
class TokamakListeners<T>(private val holding: Boolean = false) {
    private val values = MutableSharedFlow<T>(extraBufferCapacity = LISTENER_CAPACITY, onBufferOverflow = BufferOverflow.DROP_OLDEST)
    private val held = mutableListOf<T>()

    /** A new listener's flow. */
    fun stream(): Flow<T> =
        values.onSubscription {
            synchronized(held) { held.toList().also { held.clear() } }.forEach { emit(it) }
        }

    fun send(value: T) {
        synchronized(held) {
            if (holding && values.subscriptionCount.value == 0) {
                held += value
                if (held.size > LISTENER_CAPACITY) held.removeAt(0)
                return
            }
        }
        values.tryEmit(value)
    }
}

/**
 * Converts the values a plugin's `android/Exports.kt` uses to and from JSON: its data classes and
 * sealed types, and its events and their replies. Each generated dispatch's companion object is
 * one, which the plugin's host uses, even while the plugin is being created.
 */
interface TokamakCodec {
    /** [value] as the JSON org.json writes. */
    fun encodeValue(value: Any): Any = throw TokamakPluginError.typeError("android/Exports.kt uses no ${value.javaClass.simpleName}")

    /** [json] as a [type]. */
    fun decodeValue(json: TokamakValue, type: KClass<*>): Any = throw TokamakPluginError.typeError("android/Exports.kt uses no ${type.simpleName}")

    /** The JSON [json], as org.json reads it, as a [type], which [decodeValue] decodes. */
    fun <T : Any> fromJson(json: Any?, type: KClass<T>): T {
        val value = decodeValue(TokamakValue(json, "json"), type)
        return type.castOrNull(value) ?: throw TokamakPluginError.typeError("json is ${value.javaClass.simpleName}, not ${type.simpleName}")
    }

    /** [event] as the JSON payload of the event [name], which `android/Exports.kt` lists. */
    fun eventPayload(name: String, event: Any): ByteArray = throw TokamakPluginError.unlistedEvent(name)

    /** The listeners' [reply] to the event [name], which `android/Exports.kt` lists with a reply, as its type. */
    fun eventReply(name: String, reply: TokamakValue): Any = throw TokamakPluginError.unlistedReply(name)

    /** The listeners' encoded [payload] in reply to the event [name] as a [type], or null when none replied. */
    fun <R : Any> reply(name: String, payload: ByteArray, type: KClass<R>): R? {
        if (payload.isEmpty()) return null
        val json = decodePluginValue(payload).takeUnless { it == JSONObject.NULL } ?: return null
        val value = eventReply(name, TokamakValue(json, "reply"))
        return type.castOrNull(value)
            ?: throw TokamakPluginError.typeError("event $name replies with ${value.javaClass.simpleName}, not ${type.simpleName}")
    }

    /**
     * [event] as the payload of the event [name], whose listeners receive a [type] as [encode]
     * encodes it, for the generated [eventPayload].
     */
    fun <T : Any> encodeEvent(name: String, event: Any, type: KClass<T>, encode: (T) -> Any?): ByteArray {
        val value = type.castOrNull(event)
            ?: throw TokamakPluginError.typeError("event $name takes ${type.simpleName}, not ${event.javaClass.simpleName}")
        return encodePluginValue(encode(value))
    }
}

/** A native object another plugin retained, which the Worker passes as a `NativeReference`. */
class TokamakReference<T : Any>(val value: T)

/**
 * Names the [field] that tags the forms of a sealed type the Worker passes: each form is an object
 * whose [field] is the form's name, with a data class form's fields.
 */
@Target(AnnotationTarget.CLASS)
annotation class TokamakTag(val field: String)

/** A JSON value in a call's arguments, at [path], which errors name. */
class TokamakValue(private val json: Any?, private val path: String) {
    /** The field [key] of this object. */
    operator fun get(key: String): TokamakValue {
        val json = json as? JSONObject ?: throw invalid("an object")
        return TokamakValue(json.opt(key), if (path.isEmpty()) key else "$path.$key")
    }

    fun string(): String = json as? String ?: throw invalid("a string")

    fun boolean(): Boolean = json as? Boolean ?: throw invalid("a boolean")

    fun double(): Double = (json as? Number)?.toDouble() ?: throw invalid("a number")

    fun float(): Float = double().toFloat()

    /** The whole number, which JSON from JavaScript carries exactly up to 2^53. */
    fun long(): Long {
        val number = json as? Number ?: throw invalid("a number")
        if (number is Long || number is Int) return number.toLong()
        // A Double from 2^63 up converts to Long.MAX_VALUE.
        val double = number.toDouble()
        if (double % 1.0 != 0.0) throw invalid("a whole number")
        if (double < Long.MIN_VALUE.toDouble() || double >= -Long.MIN_VALUE.toDouble()) throw invalid("a Long")
        return double.toLong()
    }

    fun int(): Int = long().takeIf { it in Int.MIN_VALUE..Int.MAX_VALUE }?.toInt() ?: throw invalid("an Int")

    fun short(): Short = long().takeIf { it in Short.MIN_VALUE..Short.MAX_VALUE }?.toShort() ?: throw invalid("a Short")

    fun byte(): Byte = long().takeIf { it in Byte.MIN_VALUE..Byte.MAX_VALUE }?.toByte() ?: throw invalid("a Byte")

    /** Any JSON value, as org.json reads it, with null for JSON null. */
    fun json(): Any? = json.takeUnless { it == JSONObject.NULL }

    /** A JSON value other than null, as org.json reads it. */
    fun any(): Any = json() ?: throw invalid("a value other than null")

    fun jsonObject(): JSONObject = json as? JSONObject ?: throw invalid("an object")

    fun jsonArray(): JSONArray = json as? JSONArray ?: throw invalid("an array")

    /** Null for JSON null or a missing field, and otherwise what [read] reads. */
    fun <T> nullable(read: (TokamakValue) -> T): T? = if (json == null || json == JSONObject.NULL) null else read(this)

    fun <T> list(read: (TokamakValue) -> T): List<T> {
        val array = json as? JSONArray ?: throw invalid("an array")
        return List(array.length()) { read(TokamakValue(array.opt(it), "$path[$it]")) }
    }

    fun <T> map(read: (TokamakValue) -> T): Map<String, T> {
        val json = json as? JSONObject ?: throw invalid("an object")
        return json.keys().asSequence().associateWith { read(get(it)) }
    }

    fun <E : Enum<E>> enum(entries: EnumEntries<E>): E {
        val name = string()
        return entries.firstOrNull { it.name == name } ?: throw invalid("one of ${entries.joinToString()}")
    }

    /** The error of a value that is none of the forms of the sealed type [name]. */
    fun mismatch(name: String): TokamakPluginError = invalid("one of $name's forms")

    /** The object of type [type] that [resources] retains as this ID. */
    fun <T : Any> reference(resources: TokamakResources, type: Class<T>): TokamakReference<T> =
        TokamakReference(resources.get(string(), type))

    private fun invalid(expected: String) = TokamakPluginError.typeError("${path.ifEmpty { "arguments" }} must be $expected")

    companion object {
        /** A call's arguments: a JSON object of them by parameter name. */
        fun arguments(payload: ByteArray): TokamakValue =
            TokamakValue(runCatching { decodePluginValue(payload) }.getOrNull(), "")
    }
}

/**
 * A plugin with the dispatch generated from its `android/Exports.kt`. The app's registry creates the
 * generated subclass in place of [plugin], which plugins find with `tokamakPlugin(id)`.
 */
abstract class TokamakDispatch<P : TokamakPlugin>(val plugin: P, protected val host: TokamakHost) : TokamakPlugin by plugin {
    /** Replies with what [method] returns, on the call's executor, with nothing to cancel. */
    protected fun respond(reply: TokamakPluginBytesReply, method: () -> Any?): (() -> Unit)? {
        reply(TokamakPluginResult.encoded(runCatching { encodePluginValue(method().unlessUnit()) }, true))
        return null
    }

    /**
     * Runs the suspending [block] in a coroutine on the executor of [method], replying with what it
     * returns. Cancelling the call cancels the coroutine.
     */
    protected fun launch(method: String, reply: TokamakPluginBytesReply, block: suspend () -> Any?): () -> Unit =
        start(method) {
            reply(TokamakPluginResult.encoded(catching { encodePluginValue(block().unlessUnit()) }, true))
        }

    /** Replies with each of [values], encoded by [encode], until they end or the Worker cancels. */
    protected fun <T> collect(method: String, reply: TokamakPluginBytesReply, values: Flow<T>, encode: (T) -> Any? = { it }): () -> Unit =
        start(method) {
            val ended = catching { values.collect { reply(TokamakPluginResult(encodePluginValue(encode(it)), false, false)) } }
            reply(TokamakPluginResult.encoded(ended.map { ByteArray(0) }, true))
        }

    /** Retains [value], replying with its ID, until the subscription ends. */
    protected fun retain(reply: TokamakPluginBytesReply, value: Any): () -> Unit =
        host.resources.retain(value) { reply(TokamakPluginResult.encoded(it.mapCatching(::encodePluginValue), false)) }

    /** Retains what the suspending [block] returns, replying with its ID, until the subscription ends. */
    protected fun retain(method: String, reply: TokamakPluginBytesReply, block: suspend () -> Any): () -> Unit {
        // The coroutine and its cancellation run on the method's serial executor.
        var release: (() -> Unit)? = null
        var cancelled = false
        val cancel = start(method) {
            catching { block() }.fold(
                // A method that returns after cancellation, ignoring it, leaves nothing to retain.
                { value -> if (!cancelled) release = retain(reply, value) },
                { reply(TokamakPluginResult.encoded(Result.failure(it), true)) },
            )
        }
        return {
            cancelled = true
            cancel()
            release?.invoke()
        }
    }

    private fun start(method: String, block: suspend () -> Unit): () -> Unit {
        val job: Job = CoroutineScope(executor(method).asCoroutineDispatcher()).launch(start = CoroutineStart.UNDISPATCHED) { block() }
        return { job.cancel() }
    }
}

private fun Any?.unlessUnit(): Any? = takeUnless { it == Unit }

/** [value] as a [T], or null when it is of another type. */
private fun <T : Any> KClass<T>.castOrNull(value: Any): T? = javaObjectType.takeIf { it.isInstance(value) }?.cast(value)

/** What [block] returns or throws, except cancellation, which ends the coroutine. */
private suspend fun <T> catching(block: suspend () -> T): Result<T> =
    try {
        Result.success(block())
    } catch (error: CancellationException) {
        throw error
    } catch (error: Throwable) {
        Result.failure(error)
    }
