package com.tokamak.plugins.notifications

import com.tokamak.runtime.TokamakPluginError
import java.time.Instant
import java.time.LocalDate
import java.time.LocalTime
import java.time.ZoneId
import java.time.ZonedDateTime
import org.json.JSONObject

/** The shortest repeating interval, as on Apple platforms. */
private const val MIN_INTERVAL_MILLIS = 60_000L

/**
 * How far past its first possible day a date match looks: long enough for February 29 on a
 * given weekday, which can take 40 years across a century that is not a leap year.
 */
private const val SEARCH_DAYS = 366L * 40

/** When a scheduled notification shows. */
internal sealed interface Trigger {
    /** Whether the alarm is exact, which needs the exact alarm permission. */
    val exact: Boolean

    /** Whether the trigger fires again after it fires. */
    val repeats: Boolean

    /**
     * The next time the trigger fires after [after], in milliseconds since the epoch, counting
     * from [anchor], when it was scheduled; null when it never fires again.
     */
    fun next(after: Long, anchor: Long): Long?

    class At(val at: Long, override val exact: Boolean) : Trigger {
        override val repeats = false

        override fun next(after: Long, anchor: Long): Long = at
    }

    /** Fires each [interval] milliseconds from when it was scheduled. */
    class Every(private val interval: Long, override val exact: Boolean) : Trigger {
        override val repeats = true

        override fun next(after: Long, anchor: Long): Long = anchor + ((after - anchor) / interval + 1) * interval
    }

    /**
     * Fires at the local times that match; the parts left out match any value. Without [repeats],
     * it fires at the first match after it was scheduled, so a match missed while the device was
     * off fires once it can.
     */
    class On(private val match: DateMatch, override val repeats: Boolean, override val exact: Boolean) : Trigger {
        override fun next(after: Long, anchor: Long): Long? = match.next(if (repeats) after else anchor)
    }

    companion object {
        /** The trigger of the scheduled notification [json]: `at`, `every` or `on`. */
        fun parse(json: JSONObject): Trigger {
            val exact = optionalBoolean(json, "exact") ?: false
            val repeats = optionalBoolean(json, "repeats") ?: false
            optionalNumber(json, "at")?.let {
                if (!it.toDouble().isFinite()) throw TokamakPluginError.typeError("at must be a number of milliseconds since the epoch")
                return At(it.toLong(), exact)
            }
            optionalObject(json, "every")?.let { return Every(intervalMillis(it), exact) }
            optionalObject(json, "on")?.let { return On(DateMatch.parse(it), repeats, exact) }
            if (json.has("region")) throw TokamakPluginError.notSupported("Region triggers are iOS only")
            throw TokamakPluginError.typeError("a scheduled notification must have one of at, every, on or region")
        }

        private fun intervalMillis(every: JSONObject): Long {
            val units = mapOf("seconds" to 1_000L, "minutes" to 60_000L, "hours" to 3_600_000L, "days" to 86_400_000L)
            val millis = units.entries.sumOf { (unit, millis) -> (optionalNumber(every, unit)?.toLong() ?: 0) * millis }
            if (millis < MIN_INTERVAL_MILLIS) throw TokamakPluginError.typeError("every must be at least 60 seconds")
            return millis
        }
    }
}

/**
 * Parts of a local date and time. As on Apple platforms, parts finer than the finest given one
 * take their least value, so `{ hour: 9 }` matches 9:00:00, and coarser ones match any value.
 */
internal class DateMatch(
    private val year: Int?,
    month: Int?,
    day: Int?,
    /** 1 for Sunday to 7 for Saturday. */
    private val weekday: Int?,
    hour: Int?,
    minute: Int?,
    second: Int?,
) {
    /** The finest part given: 0 for the year to 5 for the second. */
    private val finest = listOf(year, month, day ?: weekday, hour, minute, second).indexOfLast { it != null }
    private val month = month ?: least(1, 1)
    private val day = day ?: least(2, 1)
    private val hours = values(hour, 3, 0..23)
    private val minutes = values(minute, 4, 0..59)
    private val seconds = values(second, 5, 0..59)

    /** [value] when the part at [level] is finer than the finest given, or null to match any. */
    private fun least(level: Int, value: Int): Int? = if (level > finest) value else null

    private fun values(given: Int?, level: Int, all: IntRange): IntRange =
        given?.let { it..it } ?: least(level, all.first)?.let { it..it } ?: all

    /** The first matching time after [after], in milliseconds since the epoch, or null. */
    fun next(after: Long): Long? {
        val zone = ZoneId.systemDefault()
        val start = Instant.ofEpochMilli(after).atZone(zone)
        var date = start.toLocalDate()
        if (year != null && year > date.year) date = LocalDate.of(year, 1, 1)
        repeat(SEARCH_DAYS.toInt()) {
            if (year != null && date.year > year) return null
            if (matches(date)) {
                times().map { ZonedDateTime.of(date, it, zone) }.firstOrNull { it.isAfter(start) }?.let {
                    return it.toInstant().toEpochMilli()
                }
            }
            date = date.plusDays(1)
        }
        return null
    }

    private fun matches(date: LocalDate): Boolean =
        (year == null || date.year == year) &&
            (month == null || date.monthValue == month) &&
            (day == null || date.dayOfMonth == day) &&
            // java.time numbers Monday 1 to Sunday 7.
            (weekday == null || date.dayOfWeek.value % 7 + 1 == weekday)

    private fun times(): Sequence<LocalTime> =
        sequence {
            for (h in hours) for (m in minutes) for (s in seconds) yield(LocalTime.of(h, m, s))
        }

    companion object {
        fun parse(json: JSONObject): DateMatch {
            fun part(key: String, range: IntRange): Int? =
                optionalNumber(json, key)?.toInt()?.also {
                    if (it !in range) throw TokamakPluginError.typeError("on.$key must be from ${range.first} to ${range.last}")
                }
            return DateMatch(
                part("year", 1..9999),
                part("month", 1..12),
                part("day", 1..31),
                part("weekday", 1..7),
                part("hour", 0..23),
                part("minute", 0..59),
                part("second", 0..59),
            )
        }
    }
}
