package io.github.olegg90.pswmanager

import java.util.UUID

/**
 * What was typed in to save (a login, a card), held in memory for the save
 * screen rather than in its intent: the system keeps an activity's extras and
 * gives them back to a process started again, so the secret would outlive it
 * there. Only the latest, under a token of its own.
 */
open class Typed<T : Any>(private val newToken: () -> String = { UUID.randomUUID().toString() }) {
  private var typed: Pair<String, T>? = null

  /** Keeps `value` in place of any kept before; the token takes it. */
  @Synchronized
  fun keep(value: T): String {
    val token = newToken()
    typed = token to value
    return token
  }

  /** What was kept under `token`, taken: none once taken, or after the process started again. */
  @Synchronized
  fun take(token: String): T? {
    val (_, value) = typed?.takeIf { it.first == token } ?: return null
    typed = null
    return value
  }
}

/** The login typed in, for [AutofillSaveActivity]: its user name and password. */
class TypedLogins(newToken: () -> String = { UUID.randomUUID().toString() }) : Typed<Pair<String, String>>(newToken) {
  /** Keeps a user name and password in place of any kept before; the token takes them. */
  fun keep(username: String, password: String): String = keep(username to password)
}
