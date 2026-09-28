package com.tokamak.plugins.notifications

import com.tokamak.runtime.TokamakPluginError
import org.json.JSONObject

/** A notification the page asked to show or schedule. */
internal class Content(
    val id: String,
    val title: String,
    val body: String?,
    val data: JSONObject,
    /** Milliseconds since the epoch; null to show now. */
    val at: Long?,
) {
    fun toJson(): JSONObject =
        JSONObject()
            .put("id", id)
            .put("title", title)
            .put("data", data)
            .apply {
                body?.let { put("body", it) }
                at?.let { put("at", it) }
            }

    companion object {
        fun parse(arguments: Any?, scheduled: Boolean): Content {
            val json = arguments as? JSONObject ?: JSONObject()
            val id = json.opt("id") as? String
            if (id.isNullOrEmpty()) throw typeError("id must be a non-empty string")
            val title = json.opt("title") as? String ?: throw typeError("title must be a string")
            val at = if (scheduled) scheduledTime(json) else null
            return Content(id, title, json.opt("body") as? String, json.optJSONObject("data") ?: JSONObject(), at)
        }

        fun fromJson(json: JSONObject): Content = parse(json, json.has("at"))

        private fun scheduledTime(json: JSONObject): Long {
            val at = (json.opt("at") as? Number)?.toDouble()
            if (at == null || !at.isFinite()) {
                throw typeError("at must be a number of milliseconds since the epoch")
            }
            return at.toLong()
        }

        private fun typeError(message: String) = TokamakPluginError("TypeError", message)
    }
}
