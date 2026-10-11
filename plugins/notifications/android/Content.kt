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
    /** A `setup` category, whose actions the notification shows. */
    val category: String? = null,
    /** A `setup` channel; null for the default channel. */
    val channel: String? = null,
) {
    fun toJson(): JSONObject =
        JSONObject()
            .put("id", id)
            .put("title", title)
            .put("data", data)
            .apply {
                body?.let { put("body", it) }
                at?.let { put("at", it) }
                category?.let { put("category", it) }
                channel?.let { put("channel", it) }
            }

    companion object {
        fun parse(arguments: Any?, scheduled: Boolean): Content {
            val json = arguments as? JSONObject ?: JSONObject()
            val id = json.opt("id") as? String
            if (id.isNullOrEmpty()) throw TokamakPluginError.typeError("id must be a non-empty string")
            val title = requireString(json, "title")
            val at = if (scheduled) scheduledTime(json) else null
            return Content(
                id,
                title,
                json.opt("body") as? String,
                json.optJSONObject("data") ?: JSONObject(),
                at,
                optionalString(json, "category"),
                optionalString(json, "channel"),
            )
        }

        fun fromJson(json: JSONObject): Content = parse(json, json.has("at"))

        private fun scheduledTime(json: JSONObject): Long {
            val at = (json.opt("at") as? Number)?.toDouble()
            if (at == null || !at.isFinite()) {
                throw TokamakPluginError.typeError("at must be a number of milliseconds since the epoch")
            }
            return at.toLong()
        }
    }
}
