package io.github.olegg90.pswmanager

import android.content.Intent
import android.os.Bundle
import android.view.WindowManager
import androidx.annotation.RequiresApi
import androidx.appcompat.app.AppCompatActivity
import androidx.credentials.GetCredentialResponse
import androidx.credentials.PasswordCredential
import androidx.credentials.exceptions.GetCredentialCancellationException
import androidx.credentials.exceptions.GetCredentialException
import androidx.credentials.exceptions.GetCredentialUnknownException
import androidx.credentials.provider.PendingIntentHandler
import org.json.JSONObject
import kotlin.concurrent.thread

/**
 * Handing a login to an app that asked through Credential Manager (#164),
 * when the user picked one of PswManager's: the user is verified, then the
 * entry's user name and password go to the app.
 */
@RequiresApi(34)
class PasswordActivity : AppCompatActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    window.setFlags(WindowManager.LayoutParams.FLAG_SECURE, WindowManager.LayoutParams.FLAG_SECURE)
    // Made again (the prompt is gone with the old one): Android hears it was cancelled.
    if (savedInstanceState != null) return cancel()
    val id = intent.getStringExtra(EXTRA_ENTRY) ?: return fail("No login chosen")
    verifyUser(this, "Use your password", intent.getStringExtra(EXTRA_TITLE) ?: "", {
      thread {
        val login = ProviderBridge.login(id)?.let { JSONObject(it) }
        runOnUiThread {
          if (login == null) fail("The database is locked or the entry is gone") else handOver(login.getString("username"), login.getString("password"))
        }
      }
    }, ::cancel, ::fail)
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

  companion object {
    const val EXTRA_ENTRY = "entry"
    const val EXTRA_TITLE = "title"
  }
}
