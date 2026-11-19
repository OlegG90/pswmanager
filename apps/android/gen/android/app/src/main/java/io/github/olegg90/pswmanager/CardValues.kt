package io.github.olegg90.pswmanager

import java.time.LocalDate
import java.time.ZoneOffset

/**
 * A card's values as a form's fields take them (#412): the number as digits
 * only, and the expiry (kept as text, `MM/YY` or `MM/YYYY`) split into its
 * month and year, as text, as a list's option, or as a date. Plain Kotlin, so
 * it is tested without a phone.
 */
object CardValues {
  /** The number's digits, without the spaces or dashes it may be kept with. */
  fun digits(number: String): String = number.filter { it.isDigit() }

  /**
   * The month (two digits) and the year (four digits) of `expiry` (`12/30`,
   * `12/2030`, `12-30`, `1/30`); null when it is neither.
   */
  fun monthYear(expiry: String): Pair<String, String>? {
    val match = EXPIRY.matchEntire(expiry.trim()) ?: return null
    val month = match.groupValues[1].toInt()
    if (month !in 1..12) return null
    val year = match.groupValues[2].let { if (it.length == 2) "20$it" else it }
    return month.toString().padStart(2, '0') to year
  }

  /** The expiry for one field holding both: `MM/YY`; as kept when it cannot be read. */
  fun short(expiry: String): String = monthYear(expiry)?.let { (month, year) -> "$month/${year.takeLast(2)}" } ?: expiry

  /** The expiry as a date field takes it: the first day of its month, UTC, in milliseconds; null when it cannot be read. */
  fun dateMillis(expiry: String): Long? {
    val (month, year) = monthYear(expiry) ?: return null
    return LocalDate.of(year.toInt(), month.toInt(), 1).atStartOfDay(ZoneOffset.UTC).toInstant().toEpochMilli()
  }

  /** What a list asks for, by its options alone: months (`01`…`12`, `Jan`…) or years (`2030`…). */
  enum class ListOf { MONTH, YEAR }

  /** Whether `options` are months or years (a placeholder like `MM` aside); null when neither. */
  fun listKind(options: List<String>): ListOf? = when {
    options.count { monthOf(it) != null } >= 12 -> ListOf.MONTH
    options.count { YEAR.containsMatchIn(it) } >= 3 -> ListOf.YEAR
    else -> null
  }

  /**
   * The expiry a form had, to save: its own field's text, else its month and
   * year (`06` or `June`, `2027` or `27`) as `MM/YYYY`; empty when it had
   * neither, or only a placeholder (`MM`, `YYYY`).
   */
  fun typedExpiry(expiry: String?, month: String?, year: String?): String {
    expiry?.trim()?.takeIf { it.isNotEmpty() }?.let { return it }
    val number = month?.let(::monthOf) ?: return ""
    val full = year?.trim()?.takeIf { YEAR_ONLY.matches(it) }?.let { if (it.length == 2) "20$it" else it } ?: return ""
    return "${number.toString().padStart(2, '0')}/$full"
  }

  /** The option for `month` (`01`…`12`): by its number (`1`, `01`, `01 - January`) or its name; null for none. */
  fun monthIndex(options: List<String>, month: String): Int? {
    val wanted = month.toIntOrNull() ?: return null
    return options.indexOfFirst { monthOf(it) == wanted }.takeIf { it >= 0 }
  }

  /** The option for `year` (four digits): `2030`, or `30` alone; null for none. */
  fun yearIndex(options: List<String>, year: String): Int? =
    options.indexOfFirst { it.contains(year) }.takeIf { it >= 0 }
      ?: options.indexOfFirst { it.trim() == year.takeLast(2) }.takeIf { it >= 0 }

  /** The month an option names (1…12), by its leading number or its name; null for a placeholder or a year. */
  private fun monthOf(option: String): Int? {
    val text = option.trim().lowercase()
    LEADING.find(text)?.let { match -> return match.groupValues[1].toInt().takeIf { it in 1..12 } }
    return MONTHS.indexOfFirst { text.startsWith(it) }.takeIf { it >= 0 }?.plus(1)
  }

  private val EXPIRY = Regex("""(\d{1,2})\s*[/\-.]\s*(\d{2}|\d{4})""")
  private val LEADING = Regex("""^(\d{1,2})(?!\d)""")
  private val YEAR = Regex("""\b20\d{2}\b""")
  private val YEAR_ONLY = Regex("""\d{2}|\d{4}""")
  private val MONTHS = listOf("jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec")
}
