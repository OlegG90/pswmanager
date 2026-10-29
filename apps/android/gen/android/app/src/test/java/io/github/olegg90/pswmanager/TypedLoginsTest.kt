package io.github.olegg90.pswmanager

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Test

class TypedLoginsTest {
  @Test
  fun aLoginIsTakenOnceByItsToken() {
    val typed = TypedLogins()
    val token = typed.keep("alice", "secret")
    assertNull("another token takes nothing", typed.take("other"))
    assertEquals("alice" to "secret", typed.take(token))
    assertNull("taken already", typed.take(token))
  }

  @Test
  fun onlyTheLatestIsKept() {
    val typed = TypedLogins()
    val first = typed.keep("alice", "one")
    val second = typed.keep("bob", "two")
    assertNotEquals(first, second)
    assertNull("replaced by the next one typed", typed.take(first))
    assertEquals("bob" to "two", typed.take(second))
  }

  @Test
  fun aTokenFromBeforeTakesNothingOfANewHolder() {
    // As after the process started again: the holder is new and empty.
    val token = TypedLogins { "same" }.keep("alice", "secret")
    assertNull(TypedLogins { "same" }.take(token))
  }
}
