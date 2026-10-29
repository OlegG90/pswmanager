package io.github.olegg90.pswmanager

import org.json.JSONObject

/**
 * What Rust answers through [ProviderBridge] (#315): one JSON object told apart
 * by its key, as `provider.rs` makes it, so a new state is a new case here
 * rather than a new prefix to strip at every call.
 */
sealed class Answer {
  /** Why it was not done, for the user: what Rust said, else that the answer is not one expected here. */
  open val why: String get() = "PswManager could not answer"

  /** Done; what was asked for is [json] or [text] (null when there is nothing to give). */
  class Ok(private val answer: JSONObject) : Answer() {
    val json: JSONObject get() = answer.getJSONObject("ok")
    val text: String? get() = if (answer.isNull("ok")) null else answer.getString("ok")
  }

  /** The login is for the app's package, but the user has not confirmed the app ([confirmApp]) with this certificate. */
  class Unconfirmed(val certificate: String) : Answer()

  /** Not done, and [why]; [stale] when the sealed key no longer opens the database. */
  class Failed(override val why: String, val stale: Boolean = false) : Answer()

  companion object {
    /** Reads `answer`; one that is missing or cannot be read failed. */
    fun of(answer: String?): Answer {
      val json = answer?.let { runCatching { JSONObject(it) }.getOrNull() } ?: return Failed("PswManager could not answer")
      return when {
        json.has("ok") -> Ok(json)
        json.has("unconfirmed") -> Unconfirmed(json.getString("unconfirmed"))
        else -> Failed(json.optString("failed", "PswManager could not answer"), json.optBoolean("stale"))
      }
    }
  }
}
