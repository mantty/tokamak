package com.tokamak.runtime

import java.util.Base64
import org.json.JSONArray
import org.json.JSONObject

/**
 * One call or subscription of a generated platform binding. It decodes the Worker's JSON into
 * native values and encodes native values back, keeping the native objects it returns in the
 * receiving invocation's scope.
 *
 * JSON objects tagged `#` carry what JSON lacks: native objects, bytes, errors, non-finite
 * numbers, and records whose keys include `#`.
 */
class TokamakBinding(payload: ByteArray, private val resources: TokamakResources, private val reply: TokamakPluginBytesReply) {
    companion object {
        /** The rejection of a member the running API level lacks. */
        val unavailable get() = TokamakPluginError.notSupported("The API is unavailable on this API level")
    }

    private val request = decodePluginValue(payload) as? JSONObject
        ?: throw TokamakPluginError.typeError("A binding request is an object")

    /** Argument [index] as JSON. */
    operator fun get(index: Int): Any? = request.optJSONArray("arguments")?.opt(index) ?: JSONObject.NULL

    /** The object the call is made on. */
    fun <T : Any> target(type: Class<T>): T = obj(request.opt("target"), type)

    /** The handlers an implementation calls. */
    val methods: Set<String> by lazy {
        val methods = request.optJSONArray("methods") ?: return@lazy emptySet()
        (0 until methods.length()).map(methods::getString).toSet()
    }

    private val scope: String get() = request.optString("scope")

    fun boolean(value: Any?): Boolean = value as? Boolean ?: throw mismatch(value, "boolean")

    fun byte(value: Any?): Byte = long(value).let { if (it in Byte.MIN_VALUE..Byte.MAX_VALUE) it.toByte() else throw mismatch(value, "byte") }

    fun short(value: Any?): Short = long(value).let { if (it in Short.MIN_VALUE..Short.MAX_VALUE) it.toShort() else throw mismatch(value, "short") }

    fun int(value: Any?): Int = long(value).let { if (it in Int.MIN_VALUE..Int.MAX_VALUE) it.toInt() else throw mismatch(value, "int") }

    fun long(value: Any?): Long {
        val number = double(value)
        val integer = number.toLong()
        if (integer.toDouble() != number) throw mismatch(value, "long")
        return integer
    }

    fun float(value: Any?): Float = double(value).toFloat()

    fun double(value: Any?): Double =
        (value as? Number)?.toDouble() ?: string(tagged(value, "number")).toDoubleOrNull() ?: throw mismatch(value, "double")

    fun char(value: Any?): Char = string(value).singleOrNull() ?: throw mismatch(value, "char")

    fun string(value: Any?): String = value as? String ?: throw mismatch(value, "String")

    fun bytes(value: Any?): ByteArray = Base64.getDecoder().decode(string(tagged(value, "bytes")))

    /** The native object [value] refers to. */
    fun <T : Any> obj(value: Any?, type: Class<T>): T = resources.get(tagged(value, "object", "id"), type)

    /** A value of an erased or `Object` type: JSON as it is, or the native value a tag carries. */
    fun any(value: Any?): Any? =
        when (value) {
            null, JSONObject.NULL -> null
            is JSONArray -> list(value, ::any)
            is JSONObject ->
                when (value.optString("#")) {
                    "object" -> obj(value, Any::class.java)
                    "bytes" -> bytes(value)
                    "number" -> double(value)
                    else -> map(value, ::any)
                }
            else -> value
        }

    fun <T> optional(value: Any?, decode: (Any?) -> T): T? = if (value == null || value == JSONObject.NULL) null else decode(value)

    fun <T> list(value: Any?, decode: (Any?) -> T): List<T> {
        val values = value as? JSONArray ?: throw mismatch(value, "List")
        return (0 until values.length()).map { decode(values.get(it)) }
    }

    fun <T> map(value: Any?, decode: (Any?) -> T): Map<String, T> {
        val fields = (value as? JSONObject)?.let { if (it.optString("#") == "record") it.getJSONObject("value") else it }
            ?: throw mismatch(value, "Map")
        return fields.keys().asSequence().associateWith { decode(fields.get(it)) }
    }

    /**
     * [value] as JSON: values by their runtime type, and native objects as references kept in the
     * invocation's scope, with the names of their class and superclasses.
     */
    fun encode(value: Any?): Any =
        when (value) {
            null -> JSONObject.NULL
            is Boolean, is String, is Int, is Long, is Short, is Byte -> value
            is Double -> if (value.isFinite()) value else JSONObject().put("#", "number").put("value", value.toString())
            is Float -> encode(value.toDouble())
            is Char, is CharSequence -> value.toString()
            is ByteArray -> JSONObject().put("#", "bytes").put("value", Base64.getEncoder().encodeToString(value))
            is IntArray -> JSONArray(value.map(::encode))
            is LongArray -> JSONArray(value.map(::encode))
            is FloatArray -> JSONArray(value.map(::encode))
            is DoubleArray -> JSONArray(value.map(::encode))
            is BooleanArray -> JSONArray(value.map(::encode))
            is Array<*> -> JSONArray(value.map(::encode))
            is Iterable<*> -> JSONArray(value.map(::encode))
            is Map<*, *> -> record(value)
            is Enum<*> -> value.name
            is Throwable -> error(value)
            else -> JSONObject().put("#", "object").put("id", resources.retain(value, scope)).put("types", JSONArray(types(value)))
        }

    /** Replies with the JSON [body] returns, or the exception it throws. */
    fun run(body: () -> Any?): (() -> Unit)? {
        reply(TokamakPluginResult.encoded(runCatching { encodePluginValue(body()) }, true))
        return null
    }

    /** Keeps the implementation [create] makes while the subscription lasts, and replies with its ID. */
    fun implement(create: (TokamakBinding) -> Any): () -> Unit {
        val scope = resources.open()
        send(resources.retain(create(this), scope))
        return { resources.end(scope) }
    }

    /**
     * Calls the handler [method], when the implementation has one, with the arguments it encodes,
     * and returns whether it had one.
     */
    fun call(method: String, arguments: (TokamakBinding) -> List<Any?>): Boolean {
        if (method !in methods) return false
        send(JSONObject().put("method", method).put("arguments", JSONArray(arguments(this))))
        return true
    }

    private fun send(value: Any?) {
        reply(TokamakPluginResult.encoded(runCatching { encodePluginValue(value) }, false))
    }

    private fun record(fields: Map<*, *>): JSONObject {
        val encoded = JSONObject()
        fields.forEach { (key, value) -> encoded.put(key.toString(), encode(value)) }
        return if (encoded.has("#")) JSONObject().put("#", "record").put("value", encoded) else encoded
    }

    private fun error(error: Throwable): JSONObject = failure(error).put("#", "error")

    /** The field [key] of [value] tagged [tag]. */
    private fun tagged(value: Any?, tag: String, key: String = "value"): Any? {
        val fields = value as? JSONObject
        if (fields?.optString("#") != tag || !fields.has(key)) throw TokamakPluginError.typeError("Expected a tagged $tag")
        return fields.get(key)
    }

    private fun mismatch(value: Any?, type: String) = TokamakPluginError.typeError("${value?.javaClass?.simpleName} is not $type")

    /** The names of [value]'s class and superclasses, then of the interfaces they implement. */
    private fun types(value: Any): List<String> {
        val classes = generateSequence<Class<*>>(value.javaClass) { it.superclass }.toList()
        return (classes + classes.flatMap(::interfaces)).distinct().map { it.name }
    }

    /** The interfaces [type] implements, each followed by those it extends. */
    private fun interfaces(type: Class<*>): List<Class<*>> = type.interfaces.flatMap { listOf(it) + interfaces(it) }
}
