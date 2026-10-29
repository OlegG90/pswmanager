package io.github.olegg90.pswmanager

import java.util.UUID

/**
 * The login typed in, held in memory for [AutofillSaveActivity] rather than
 * in its intent: the system keeps an activity's extras and gives them back to
 * a process started again, so the password would outlive it there. Only the
 * latest, under a token of its own.
 */
class TypedLogins(private val newToken: () -> String = { UUID.randomUUID().toString() }) {
  private var typed: Triple<String, String, String>? = null

  /** Keeps a user name and password in place of any kept before; the token takes them. */
  @Synchronized
  fun keep(username: String, password: String): String {
    val token = newToken()
    typed = Triple(token, username, password)
    return token
  }

  /** The user name and password kept under `token`, taken: none once taken, or after the process started again. */
  @Synchronized
  fun take(token: String): Pair<String, String>? {
    val (_, username, password) = typed?.takeIf { it.first == token } ?: return null
    typed = null
    return username to password
  }
}
