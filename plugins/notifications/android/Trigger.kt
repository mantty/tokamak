package com.tokamak.plugins.notifications

import com.tokamak.runtime.TokamakPluginError
import java.time.Instant
import java.time.LocalDate
import java.time.LocalTime
import java.time.ZoneId
import java.time.ZonedDateTime

/** The shortest repeating interval, as on Apple platforms. */
private const val MIN_INTERVAL_MILLIS = 60_000L

/**
 * How far past its first possible day a date match looks: long enough for February 29 on a
 * given weekday, which can take 40 years across a century that is not a leap year.
 */
private const val SEARCH_DAYS = 366L * 40

/** Whether the trigger fires again after it fires. */
internal val Trigger.repeating: Boolean
    get() =
        when (this) {
            is Trigger.AtTrigger -> false
            is Trigger.EveryTrigger -> true
            is Trigger.DateTrigger -> repeats == true
        }

/**
 * The next time the trigger fires after [after], in milliseconds since the epoch, counting from
 * [anchor], when it was scheduled; null when it never fires again. A date match without `repeats`
 * fires at its first match after it was scheduled, so a match missed while the device was off
 * fires once it can. Throws `TypeError` for an interval or date part it cannot fire at.
 */
internal fun Trigger.next(after: Long, anchor: Long): Long? =
    when (this) {
        is Trigger.AtTrigger -> at
        is Trigger.EveryTrigger -> {
            val interval = intervalMillis(every)
            anchor + ((after - anchor) / interval + 1) * interval
        }
        is Trigger.DateTrigger -> Dates(on).next(if (repeats == true) after else anchor)
    }

/** The interval in milliseconds, which throws `TypeError` when it is shorter than 60 seconds. */
private fun intervalMillis(every: Interval): Long {
    val parts = listOf(every.seconds to 1_000.0, every.minutes to 60_000.0, every.hours to 3_600_000.0, every.days to 86_400_000.0)
    val millis = parts.sumOf { (value, unit) -> (value ?: 0.0) * unit }.toLong()
    if (millis < MIN_INTERVAL_MILLIS) throw TokamakPluginError.typeError("every must be at least 60 seconds")
    return millis
}

/**
 * The times [match] matches. As on Apple platforms, parts finer than the finest given one take
 * their least value, so `{ hour: 9 }` matches 9:00:00, and coarser ones match any value. Throws
 * `TypeError` for a part out of its range.
 */
private class Dates(match: DateMatch) {
    private val year = match.year.within("year", 1..9999)
    private val weekday = match.weekday.within("weekday", 1..7)

    /** The finest part given: 0 for the year to 5 for the second. */
    private val finest =
        listOf(match.year, match.month, match.day ?: match.weekday, match.hour, match.minute, match.second).indexOfLast { it != null }
    private val month = match.month.within("month", 1..12) ?: least(1, 1)
    private val day = match.day.within("day", 1..31) ?: least(2, 1)
    private val hours = values(match.hour.within("hour", 0..23), 3, 0..23)
    private val minutes = values(match.minute.within("minute", 0..59), 4, 0..59)
    private val seconds = values(match.second.within("second", 0..59), 5, 0..59)

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
}

/** The part, which throws `TypeError` unless it is in [range]. */
private fun Int?.within(part: String, range: IntRange): Int? =
    also { if (it != null && it !in range) throw TokamakPluginError.typeError("on.$part must be from ${range.first} to ${range.last}") }
