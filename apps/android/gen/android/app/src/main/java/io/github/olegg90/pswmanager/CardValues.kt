package io.github.olegg90.pswmanager

/**
 * A card's values as a form's fields take them (#412): the number as digits
 * only, and the expiry (kept as text, `MM/YY` or `MM/YYYY`) split into its
 * month and year. Plain Kotlin, so it is tested without a phone.
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

  private val EXPIRY = Regex("""(\d{1,2})\s*[/\-.]\s*(\d{2}|\d{4})""")
}
