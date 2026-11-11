package io.github.olegg90.pswmanager

import android.content.Intent
import android.os.Bundle
import androidx.annotation.RequiresApi
import androidx.credentials.provider.BeginGetCredentialResponse
import androidx.credentials.provider.PendingIntentHandler

/**
 * Unlocking PswManager for another app, outside the app's window (#161): the
 * fingerprint (the key sealed for biometric unlock) when it is allowed, else
 * the master password. The database is then unlocked in Rust for the app too,
 * and [done] answers what the other app asked for, now unlocked.
 */
abstract class UnlockScreen : PasswordScreen() {
  /** What the unlock is for, under the title. */
  protected abstract val purpose: String

  /** Unlocked: answers the request that opened the screen, and closes it. */
  protected abstract fun done()

  override val heading = "Unlock PswManager"
  override val action = "Unlock"
  override val working = "Unlocking…"
  override val promptTitle = "Unlock PswManager"
  override val invalidated = "A new fingerprint or face was added: unlock with the master password, then in PswManager once"

  override fun body(fingerprint: Boolean) = purpose

  override fun attempt(typed: String?, sealed: String?) =
    Answer.of(ProviderBridge.unlock(applicationContext, ProviderBridge.state(this), typed, sealed))

  override fun succeeded(answer: Answer.Ok) = done()

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    showScreen()
    if (ProviderBridge.isUnlocked()) return done()
    askFirst(savedInstanceState)
  }
}

/** Unlocking for the credential provider: Android gets the provider's answer again, now unlocked. */
@RequiresApi(34)
class UnlockActivity : UnlockScreen() {
  override val purpose = "For a passkey or password another app asks for."

  /** Unlocked: Android gets what the site or app asked for. */
  override fun done() {
    val result = Intent()
    val request = PendingIntentHandler.retrieveBeginGetCredentialRequest(intent)
    val response = request?.let { PswmCredentialService.credentials(this, it) } ?: BeginGetCredentialResponse()
    PendingIntentHandler.setBeginGetCredentialResponse(result, response)
    setResult(RESULT_OK, result)
    finish()
  }
}
