package io.github.olegg90.pswmanager

import android.content.Intent
import android.os.Bundle
import android.view.WindowManager
import androidx.annotation.RequiresApi
import androidx.appcompat.app.AppCompatActivity
import androidx.credentials.GetCredentialResponse
import androidx.credentials.GetPublicKeyCredentialOption
import androidx.credentials.PublicKeyCredential
import androidx.credentials.exceptions.GetCredentialCancellationException
import androidx.credentials.exceptions.GetCredentialException
import androidx.credentials.exceptions.GetCredentialUnknownException
import androidx.credentials.provider.PendingIntentHandler
import kotlin.concurrent.thread

/**
 * Signing in with a passkey (#162), when the user picked one of PswManager's
 * in Android's list: the user is verified (fingerprint, or the phone's screen
 * lock), then the core signs the site's or app's challenge with the entry's
 * passkey. Who asks decides the client data: a browser on Google's list of
 * privileged apps speaks for the site (its origin and client data hash);
 * any other app signs for itself (its signing certificate).
 */
@RequiresApi(34)
class PasskeyActivity : AppCompatActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    window.setFlags(WindowManager.LayoutParams.FLAG_SECURE, WindowManager.LayoutParams.FLAG_SECURE)
    // Made again (the prompt is gone with the old one): Android hears it was cancelled.
    if (savedInstanceState != null) return cancel()
    val request = PendingIntentHandler.retrieveProviderGetCredentialRequest(intent)
    val option = request?.credentialOptions?.filterIsInstance<GetPublicKeyCredentialOption>()?.firstOrNull()
    val id = intent.getStringExtra(EXTRA_ENTRY)
    if (request == null || option == null || id == null) return fail("Nothing to sign in to")
    val caller = try {
      Caller.of(this, request.callingAppInfo)
    } catch (e: Exception) {
      return fail("Cannot tell who asks: ${e.message}")
    }
    verifyUser(this, "Sign in with your passkey", intent.getStringExtra(EXTRA_TITLE) ?: "", {
      thread {
        val answer = ProviderBridge.sign(id, option.requestJson, caller.origin, caller.clientDataHash(option.clientDataHash), caller.packageName, caller.certificate)
        runOnUiThread { if (answer.startsWith(ProviderBridge.FAILED)) fail(answer.removePrefix(ProviderBridge.FAILED)) else signedIn(answer) }
      }
    }, ::cancel, ::fail)
  }

  private fun signedIn(json: String) {
    val result = Intent()
    PendingIntentHandler.setGetCredentialResponse(result, GetCredentialResponse(PublicKeyCredential(json)))
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

  companion object {
    const val EXTRA_ENTRY = "entry"
    const val EXTRA_TITLE = "title"
  }
}
