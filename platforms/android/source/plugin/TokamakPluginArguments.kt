package com.tokamak.runtime

import org.json.JSONObject

/** [arguments] as an object; throws a `TypeError` otherwise. */
fun requireObject(arguments: Any?): JSONObject =
    arguments as? JSONObject ?: throw TokamakPluginError.typeError("arguments must be an object")

/** The string [name]; throws a `TypeError` otherwise. */
fun JSONObject.requireString(name: String): String =
    optionalString(name) ?: throw TokamakPluginError.typeError("$name must be a string")

/** The string [name], or null when it is absent or null; throws a `TypeError` otherwise. */
fun JSONObject.optionalString(name: String): String? =
    when (val value = opt(name)) {
        null, JSONObject.NULL -> null
        is String -> value
        else -> throw TokamakPluginError.typeError("$name must be a string")
    }

/** The boolean [name]; throws a `TypeError` otherwise. */
fun JSONObject.requireBoolean(name: String): Boolean =
    opt(name) as? Boolean ?: throw TokamakPluginError.typeError("$name must be a boolean")
