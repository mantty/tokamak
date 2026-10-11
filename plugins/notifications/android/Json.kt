package com.tokamak.plugins.notifications

import com.tokamak.runtime.TokamakPluginError
import org.json.JSONArray
import org.json.JSONObject

// Readers of the page's JSON arguments, which throw `TypeError` on a value of the wrong type. An
// absent list reads as empty.

internal fun objects(json: JSONObject, key: String): List<JSONObject> =
    list(json, key) { array, index ->
        array.opt(index) as? JSONObject ?: throw TokamakPluginError.typeError("each of $key must be an object")
    }

internal fun strings(json: JSONObject, key: String): List<String> =
    list(json, key) { array, index ->
        array.opt(index) as? String ?: throw TokamakPluginError.typeError("each of $key must be a string")
    }

internal fun requireString(json: JSONObject, key: String): String =
    json.opt(key) as? String ?: throw TokamakPluginError.typeError("$key must be a string")

internal fun optionalString(json: JSONObject, key: String): String? =
    if (json.has(key)) requireString(json, key) else null

internal fun requireNumber(json: JSONObject, key: String): Number =
    json.opt(key) as? Number ?: throw TokamakPluginError.typeError("$key must be a number")

internal fun optionalNumber(json: JSONObject, key: String): Number? =
    if (json.has(key)) requireNumber(json, key) else null

internal fun optionalBoolean(json: JSONObject, key: String): Boolean? {
    if (!json.has(key)) return null
    return json.opt(key) as? Boolean ?: throw TokamakPluginError.typeError("$key must be a boolean")
}

internal fun optionalObject(json: JSONObject, key: String): JSONObject? {
    if (!json.has(key)) return null
    return json.opt(key) as? JSONObject ?: throw TokamakPluginError.typeError("$key must be an object")
}

private fun <T> list(json: JSONObject, key: String, entry: (JSONArray, Int) -> T): List<T> {
    if (!json.has(key)) return emptyList()
    val array = json.opt(key) as? JSONArray ?: throw TokamakPluginError.typeError("$key must be an array")
    return List(array.length()) { entry(array, it) }
}
