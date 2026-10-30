package com.tokamak.runtime

import java.io.File
import kotlin.reflect.KClass
import kotlin.reflect.KParameter
import kotlin.reflect.KType
import kotlin.reflect.full.allSupertypes
import kotlin.reflect.full.findAnnotation
import kotlin.reflect.full.primaryConstructor
import kotlin.reflect.full.starProjectedType
import kotlinx.coroutines.flow.Flow
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Test

/** A failure to describe the plugin, which the message explains. */
class DescribeError(message: String) : Exception(message)

/**
 * Runs the plugin's registration, which the compiled classes in `tokamak.classes` include, and
 * describes the methods it exports and the events it emits into `tokamak.output`:
 * `description.json`, and the dispatch that serves them, `Exports.generated.kt`. A failure's
 * message goes to `error` there.
 */
class Describe {
    @Test
    @Suppress("UNCHECKED_CAST")
    fun describe() {
        val output = File(System.getProperty("tokamak.output"))
        try {
            val registration = registration(File(System.getProperty("tokamak.classes")))
            val plugin = registration::class.allSupertypes.first { it.classifier == TokamakExports::class }
                .arguments.single().type?.classifier as KClass<*>
            val exporter = TokamakExporter<TokamakPlugin>()
            (registration as TokamakExports<TokamakPlugin>).exports(exporter)
            val exports = exports(exporter, plugin)
            val events = exporter.events.map { Event(it.name, it.doc, it.type.starProjectedType, it.reply?.starProjectedType) }
            output.resolve("description.json").writeText(description(exports, events, Types()))
            output.resolve("Exports.generated.kt").writeText(dispatch(plugin, exports, events))
        } catch (error: DescribeError) {
            output.resolve("error").writeText(error.message ?: "")
        }
    }
}

/** An exported method as the registration lists it and Kotlin declares it. */
internal class Export(
    val kind: String,
    val name: String,
    val doc: String,
    val overflow: TokamakOverflow,
    val parameters: List<KParameter>,
    /** The call's result type, or the type of each value the stream sends. */
    val returns: KType,
    val suspends: Boolean,
)

/** An event as the registration lists it, with the types its listeners receive and reply with. */
internal class Event(val name: String, val doc: String, val type: KType, val reply: KType?)

/** The one object in [classes] that implements [TokamakExports]. */
private fun registration(classes: File): TokamakExports<*> {
    val loader = Describe::class.java.classLoader
    val registrations = classes.walk()
        .filter { it.extension == "class" }
        .map { it.relativeTo(classes).path.removeSuffix(".class").replace(File.separatorChar, '.') }
        .mapNotNull { name -> runCatching { Class.forName(name, false, loader) }.getOrNull() }
        .filter { TokamakExports::class.java.isAssignableFrom(it) && !it.isInterface }
        .toList()
    val registration = registrations.singleOrNull()
        ?: throw DescribeError("Exports.kt declares one object that implements TokamakExports, not ${registrations.size}")
    return registration.kotlin.objectInstance as? TokamakExports<*>
        ?: throw DescribeError("${registration.simpleName} is an object")
}

/** The methods [exporter] lists, which are [plugin]'s. */
private fun exports(exporter: TokamakExporter<TokamakPlugin>, plugin: KClass<*>): List<Export> =
    exporter.exports.map { export ->
        val method = export.method
        val receiver = method.parameters.firstOrNull()
        if (receiver?.kind != KParameter.Kind.INSTANCE || receiver.type.classifier != plugin) {
            throw DescribeError("${method.name} is not a method of ${plugin.simpleName}")
        }
        if (export.kind == "stream" && method.isSuspend) {
            throw DescribeError("${plugin.simpleName}.${method.name} returns its Flow without suspending")
        }
        val returns = if (export.kind == "stream") element(method.returnType, "${method.name}:") else method.returnType
        Export(export.kind, method.name, export.doc, export.overflow, method.parameters.drop(1), returns, method.isSuspend)
    }

/** The description of [exports] and [events], which records the data classes and enums they use in [types]. */
private fun description(exports: List<Export>, events: List<Event>, types: Types): String {
    val methods = exports.map { export ->
        val context = "${export.name}:"
        buildMap {
            put("name", export.name)
            put("kind", export.kind)
            if (export.doc.isNotEmpty()) put("doc", export.doc)
            if (export.parameters.isNotEmpty()) {
                put("parameters", export.parameters.map { mapOf("name" to it.name, "type" to types.describe(it.type, "$context ${it.name}")) })
            }
            when (export.kind) {
                "call" -> put("returns", types.describe(export.returns, "$context result"))
                "stream" -> put("returns", types.describe(export.returns, "$context values"))
            }
            if (export.overflow == TokamakOverflow.DROP_OLDEST) put("overflow", "dropOldest")
        }
    }
    val described = events.map { event ->
        val context = "event ${event.name}:"
        buildMap {
            put("name", event.name)
            if (event.doc.isNotEmpty()) put("doc", event.doc)
            put("type", types.describe(event.type, context))
            event.reply?.let { put("reply", types.describe(it, "$context reply")) }
        }
    }
    val description = buildMap {
        put("methods", methods)
        if (described.isNotEmpty()) put("events", described)
        put("types", types.described.values.toList())
    }
    return JSONObject(description).toString() + "\n"
}

/** The data classes and enums the description names. */
private class Types {
    private val classes = mutableMapOf<String, KClass<*>>()

    /** The descriptions of the named types, by name. */
    val described = sortedMapOf<String, Any>()

    /** The JSON description of [type], whose [context] errors name. Placement rules are tok's to check. */
    fun describe(type: KType, context: String): Any {
        val classifier = type.classifier as? KClass<*> ?: throw DescribeError("$context $type is not a type the Worker can pass")
        val argument = { index: Int -> type.arguments[index].type ?: throw DescribeError("$context $type has a star projection") }
        val value: Any = when (classifier) {
            String::class -> "string"
            Boolean::class -> "boolean"
            in NUMBERS -> "number"
            Unit::class -> "void"
            ByteArray::class -> "bytes"
            Any::class -> "json"
            JSONObject::class -> mapOf("record" to "json")
            JSONArray::class -> mapOf("array" to "json")
            TokamakPluginBody::class -> "body"
            TokamakReference::class -> "reference"
            List::class -> mapOf("array" to describe(argument(0), context))
            Map::class -> {
                if (argument(0).classifier != String::class) throw DescribeError("$context maps the Worker passes have String keys")
                mapOf("record" to describe(argument(1), context))
            }
            else -> mapOf("named" to named(classifier, context))
        }
        return if (type.isMarkedNullable) mapOf("optional" to value) else value
    }

    /** The name of [type], which the first use describes; recording it first ends recursion. */
    private fun named(type: KClass<*>, context: String): String {
        val name = type.simpleName ?: throw DescribeError("$context $type has no name")
        if (!type.java.isEnum && !type.isData && !type.isSealed) {
            throw DescribeError("$context ${type.qualifiedName} is not a data class, an enum class or a sealed type")
        }
        val existing = classes.putIfAbsent(name, type)
        if (existing == null) {
            described[name] = declaration(type, name)
        } else if (existing != type) {
            throw DescribeError("two types the Worker passes are named $name")
        }
        return name
    }

    private fun declaration(type: KClass<*>, name: String): Any =
        when {
            type.java.isEnum -> mapOf("name" to name, "values" to constants(type))
            type.isSealed -> buildMap {
                put("name", name)
                put("alternatives", forms(type).map { alternative(it, "$name.${it.name}:") })
                type.findAnnotation<TokamakTag>()?.let { put("tag", it.field) }
            }
            else -> mapOf("name" to name, "fields" to fields(type).map { mapOf("name" to it.name, "type" to describe(it.type, "$name.${it.name}:")) })
        }

    /** The [form]'s name, and the type of its value: a value class's value, or a data class. */
    private fun alternative(form: Form, context: String): Map<String, Any> {
        val value = form.value
        return when {
            form.isName -> mapOf("name" to form.name)
            value != null && (value.type.classifier as? KClass<*>)?.isData == true ->
                throw DescribeError("$context a data class is a form itself, not a value class's value")
            value != null -> mapOf("name" to form.name, "type" to describe(value.type, context))
            form.type.isData -> mapOf("name" to form.name, "type" to mapOf("named" to named(form.type, context)))
            else -> throw DescribeError("$context ${form.type.qualifiedName} is not an object, a data class or a value class")
        }
    }
}

/** A form of a sealed type: a subclass, named by [jsonName]. */
internal class Form(val name: String, val type: KClass<*>) {
    /** Whether the form is an object, whose JSON is its name. */
    val isName = type.objectInstance != null

    /** A value class's value, which is the form's JSON. */
    val value: KParameter? = if (type.isValue) fields(type).single() else null
}

/** The forms of the sealed type [type], by name, as reflection lists no declaration order. */
internal fun forms(type: KClass<*>): List<Form> =
    type.sealedSubclasses.map { subclass ->
        val name = subclass.simpleName ?: throw DescribeError("${type.simpleName} has a form without a name")
        Form(jsonName(name), subclass)
    }.sortedBy { it.name }

/** A data class's properties: its primary constructor's parameters. */
internal fun fields(type: KClass<*>): List<KParameter> = type.primaryConstructor?.parameters.orEmpty()

/** An enum class's constants, by [jsonName], read without running its initializer. */
internal fun constants(type: KClass<*>): List<String> =
    type.java.declaredFields.filter { it.isEnumConstant }.map { jsonName(it.name) }

private fun element(type: KType, context: String): KType {
    if (type.classifier != Flow::class) throw DescribeError("$context a stream returns a Flow")
    return type.arguments.single().type ?: throw DescribeError("$context $type has a star projection")
}
