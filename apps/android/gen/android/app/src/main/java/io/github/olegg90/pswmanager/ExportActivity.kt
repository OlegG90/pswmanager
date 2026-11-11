package io.github.olegg90.pswmanager

import android.app.Activity
import android.os.Bundle
import androidx.credentials.providerevents.IntentHandler
import androidx.credentials.providerevents.exception.ImportCredentialsNoExportOptionException
import androidx.credentials.providerevents.transfer.ImportCredentialsResponse
import androidx.credentials.providerevents.transfer.ProviderImportCredentialsRequest

/**
 * Another password manager imports from PswManager through Android's
 * Credential Transfer (#153): the system starts this activity with the
 * request. It is checked (Google Play services asks, with PswManager's own
 * secret id), the user sees where the entries go and confirms with the
 * fingerprint or the master password, every time, unlocked or not; Rust then
 * opens the database with that key and hands over its entries in use as CXF
 * JSON, which goes to the system's content URI.
 */
class ExportActivity : PasswordScreen() {
  private lateinit var request: ProviderImportCredentialsRequest
  /** The app the entries go to, and the database they come from, by name. */
  private lateinit var destination: String
  private lateinit var database: String

  override val heading get() = "Export to $destination"
  override val action = "Export"
  override val working = "Exporting…"
  override val promptTitle = "Export from PswManager"
  override val invalidated = "A new fingerprint or face was added: confirm with the master password"

  override fun body(fingerprint: Boolean) =
    "Every entry in use in “$database” goes to $destination: passwords, passkeys, TOTP secrets, notes and " +
      "fields. History, icons and files stay here; nothing here changes. Confirm with the master password" +
      (if (fingerprint) " or the fingerprint." else ".")

  override fun attempt(typed: String?, sealed: String?) =
    Answer.of(ProviderBridge.export(applicationContext, ProviderBridge.state(this), typed, sealed))

  /** The export goes to the system's content URI; the importer reads it from there. */
  override fun succeeded(answer: Answer.Ok) {
    try {
      IntentHandler.setImportCredentialsResponse(this, request.uri, intent, ImportCredentialsResponse(answer.text.orEmpty()))
    } catch (e: Exception) {
      return show(e.message ?: e.toString())
    }
    setResult(Activity.RESULT_OK, intent)
    finish()
  }

  override fun cancelled() {
    setResult(Activity.RESULT_CANCELED)
    finish()
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    request = IntentHandler.retrieveProviderImportCredentialsRequest(intent) ?: return cancelled()
    if (callingPackage != PLAY_SERVICES || !ExportRegistration.matches(this, request.credId)) {
      return refuse("PswManager exports only through Android's transfer")
    }
    destination = appLabel(request.callingAppInfo.packageName)
    database = ExportRegistration.name(this) ?: "PswManager"
    showScreen()
    askFirst(savedInstanceState)
  }

  /** Refused, with why, for the importing app (it is read from the result on RESULT_OK). */
  private fun refuse(why: String) {
    IntentHandler.setImportCredentialsException(intent, ImportCredentialsNoExportOptionException(why))
    setResult(Activity.RESULT_OK, intent)
    finish()
  }

  companion object {
    /** Who starts the export: Android's Credential Transfer, in Google Play services. */
    private const val PLAY_SERVICES = "com.google.android.gms"
  }
}
