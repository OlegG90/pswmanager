package io.github.olegg90.pswmanager

import android.content.Intent
import android.os.Bundle
import androidx.annotation.RequiresApi
import androidx.appcompat.app.AppCompatActivity
import androidx.credentials.GetCredentialResponse
import androidx.credentials.PasswordCredential
import androidx.credentials.exceptions.GetCredentialCancellationException
import androidx.credentials.exceptions.GetCredentialException
import androidx.credentials.exceptions.GetCredentialUnknownException
import androidx.credentials.provider.PendingIntentHandler
import kotlin.concurrent.thread

/**
 * Handing a login to an app that asked through Credential Manager (#164),
 * when the user picked one of PswManager's: the user is verified, then the
 * entry's user name and password go to the app, if the entry is for it (its
 * package and signing certificate). An entry for its package that keeps no
 * certificate goes only once the user confirmed the app ([confirmApp]).
 */
@RequiresApi(34)
class PasswordActivity : AppCompatActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    Screenshots.apply(this)
    // Made again (the prompt is gone with the old one): Android hears it was cancelled.
    if (savedInstanceState != null) return cancel()
    val id = intent.getStringExtra(ProviderBridge.EXTRA_ENTRY) ?: return fail("No login chosen")
    val request = PendingIntentHandler.retrieveProviderGetCredentialRequest(intent) ?: return fail("Cannot tell who asks")
    val caller = try {
      Caller.of(this, request.callingAppInfo)
    } catch (e: Exception) {
      return fail("Cannot tell who asks: ${e.message}")
    }
    val title = intent.getStringExtra(ProviderBridge.EXTRA_TITLE) ?: ""
    verifyUser(this, "Use your password", title, { release(id, title, caller, false) }, ::cancel, ::fail)
  }

  /** Hands entry `id`'s login over, the user verified; asks to confirm the app first when the entry needs it. */
  private fun release(id: String, title: String, caller: Caller, confirmed: Boolean) {
    thread {
      val answer = Answer.of(ProviderBridge.login(id, caller.origin, caller.packageName, caller.certificate, confirmed))
      runOnUiThread {
        when (answer) {
          is Answer.Ok -> answer.json.let { handOver(it.getString("username"), it.getString("password")) }
          is Answer.Unconfirmed -> confirmApp(this, caller.packageName, title, answer.certificate, { release(id, title, caller, true) }, ::cancel)
          is Answer.Failed -> fail(answer.why)
        }
      }
    }
  }

  private fun handOver(username: String, password: String) {
    val result = Intent()
    PendingIntentHandler.setGetCredentialResponse(result, GetCredentialResponse(PasswordCredential(username, password)))
    setResult(RESULT_OK, result)
    finish()
  }

  private fun fail(why: String) = finishWith(GetCredentialUnknownException(why))

  private fun cancel() = finishWith(GetCredentialCancellationException("Cancelled"))

  private fun finishWith(exception: GetCredentialException) {
    val result = Intent()
    PendingIntentHandler.setGetCredentialException(result, exception)
    setResult(RESULT_OK, result)
    finish()
  }

}
