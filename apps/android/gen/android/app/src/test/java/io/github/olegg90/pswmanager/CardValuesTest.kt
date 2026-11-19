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
}
