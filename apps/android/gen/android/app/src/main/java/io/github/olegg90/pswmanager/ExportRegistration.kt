package io.github.olegg90.pswmanager

import android.content.Context
import android.util.Base64
import java.security.MessageDigest
import java.security.SecureRandom

/**
 * What PswManager registers for export (#153): one entry for the database on
 * the phone, under a secret random id kept in the app's private storage.
 * Android's Credential Transfer hands the id back with a request, so a
 * request that did not come through it (another id) is refused.
 */
object ExportRegistration {
  private const val PREFS = "export"
  private const val ID = "id"
  private const val NAME = "name"

  private fun prefs(context: Context) = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

  /** The secret id, made the first time. */
  fun id(context: Context): String {
    prefs(context).getString(ID, null)?.let { return it }
    val bytes = ByteArray(32).also { SecureRandom().nextBytes(it) }
    val id = Base64.encodeToString(bytes, Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP)
    prefs(context).edit().putString(ID, id).apply()
    return id
  }

  /** Whether `credId` is the registered id (compared in constant time); false when none is. */
  fun matches(context: Context, credId: String): Boolean {
    val id = prefs(context).getString(ID, null) ?: return false
    return MessageDigest.isEqual(id.toByteArray(), credId.toByteArray())
  }

  /** The database registered, as the importing app shows it. */
  fun name(context: Context): String? = prefs(context).getString(NAME, null)

  fun registered(context: Context, name: String) = prefs(context).edit().putString(NAME, name).apply()

  /** Cleared: a new id next time, so a request for the old one is refused. */
  fun cleared(context: Context) = prefs(context).edit().clear().apply()
}
