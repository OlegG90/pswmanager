package io.github.olegg90.pswmanager

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class CardValuesTest {
  @Test
  fun theNumberIsGivenAsDigits() {
    assertEquals("4111111111111111", CardValues.digits("4111 1111-1111 1111"))
  }

  @Test
  fun theExpiryIsSplitIntoMonthAndFullYear() {
    assertEquals("12" to "2030", CardValues.monthYear("12/30"))
    assertEquals("01" to "2030", CardValues.monthYear("1/2030"))
    assertEquals("07" to "2031", CardValues.monthYear(" 07 - 31 "))
    assertNull(CardValues.monthYear("13/30"))
    assertNull(CardValues.monthYear("soon"))
  }

  @Test
  fun oneFieldTakesTheShortForm() {
    assertEquals("12/30", CardValues.short("12/2030"))
    assertEquals("01/30", CardValues.short("1/30"))
    assertEquals("as kept", CardValues.short("as kept"))
  }

  @Test
  fun aDateFieldTakesTheFirstOfTheMonth() {
    assertEquals(1_922_313_600_000L, CardValues.dateMillis("12/30"))
    assertNull(CardValues.dateMillis("soon"))
  }

  @Test
  fun theExpiryToSaveIsTheFieldsElseTheMonthAndYear() {
    assertEquals("12/30", CardValues.typedExpiry(" 12/30 ", "01", "2031"))
    assertEquals("06/2027", CardValues.typedExpiry(null, "06", "2027"))
    assertEquals("06/2027", CardValues.typedExpiry("", "June", "27"))
    assertEquals("", CardValues.typedExpiry(null, "MM", "YYYY"))
    assertEquals("", CardValues.typedExpiry(null, "06", null))
  }

  @Test
  fun listsOfMonthsAndYearsAreToldApartByTheirOptions() {
    val months = listOf("MM") + (1..12).map { it.toString().padStart(2, '0') }
    val years = listOf("YYYY") + (2025..2038).map { it.toString() }
    assertEquals(CardValues.ListOf.MONTH, CardValues.listKind(months))
    assertEquals(CardValues.ListOf.YEAR, CardValues.listKind(years))
    assertNull(CardValues.listKind(listOf("Visa", "Mastercard")))
    assertEquals(CardValues.ListOf.MONTH, CardValues.listKind(listOf("January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December")))
  }

  @Test
  fun theOptionIsPickedByNumberOrName() {
    val months = listOf("MM") + (1..12).map { it.toString().padStart(2, '0') }
    assertEquals(12, CardValues.monthIndex(months, "12"))
    assertEquals(1, CardValues.monthIndex(listOf("Month", "1 - January", "2 - February"), "01"))
    assertEquals(2, CardValues.monthIndex(listOf("--", "Jan", "Feb"), "02"))
    val years = listOf("YYYY") + (2025..2038).map { it.toString() }
    assertEquals(6, CardValues.yearIndex(years, "2030"))
    assertEquals(2, CardValues.yearIndex(listOf("YY", "29", "30"), "2030"))
    assertNull(CardValues.yearIndex(years, "2050"))
  }
}
