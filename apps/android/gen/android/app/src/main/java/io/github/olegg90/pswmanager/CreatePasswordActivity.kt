package io.github.olegg90.pswmanager

import android.content.Intent
import android.os.Bundle
import androidx.activity.result.contract.ActivityResultContracts
import androidx.annotation.RequiresApi
import androidx.appcompat.app.AppCompatActivity
import androidx.credentials.CreatePasswordRequest
import androidx.credentials.CreatePasswordResponse
import androidx.credentials.exceptions.CreateCredentialCancellationException
import androidx.credentials.exceptions.CreateCredentialException
import androidx.credentials.exceptions.CreateCredentialUnknownException
import androidx.credentials.provider.PendingIntentHandler

/**
 * Saving a login an app offers through Credential Manager (#164), when the
 * user picked PswManager: unlocked first if need be, then where it goes
 * ([SaveLogin]).
 */
@RequiresApi(34)
class CreatePasswordActivity : AppCompatActivity() {
  private lateinit var saving: SaveLogin

  private val unlock = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
    if (!::saving.isInitialized) return@registerForActivityResult
    if (result.resultCode == RESULT_OK && ProviderBridge.isUnlocked()) saving.choose() else cancel()
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    Screenshots.apply(this)
    if (savedInstanceState != null) return cancel()
    val provided = PendingIntentHandler.retrieveProviderCreateCredentialRequest(intent)
    val request = provided?.callingRequest as? CreatePasswordRequest ?: return fail("No password to save")
    val caller = try {
      Caller.of(this, provided.callingAppInfo)
    } catch (e: Exception) {
      return fail("Cannot tell who asks: ${e.message}")
    }
    saving = SaveLogin(this, caller, request.id, request.password, ::saved, ::cancel, ::fail)
    if (ProviderBridge.isUnlocked()) saving.choose() else unlock.launch(Intent(this, UnlockActivity::class.java))
  }

  private fun saved() {
    val result = Intent()
    PendingIntentHandler.setCreateCredentialResponse(result, CreatePasswordResponse())
    setResult(RESULT_OK, result)
    finish()
  }

  private fun fail(why: String) = finishWith(CreateCredentialUnknownException(why))

  private fun cancel() = finishWith(CreateCredentialCancellationException("Cancelled"))

  private fun finishWith(exception: CreateCredentialException) {
    val result = Intent()
    PendingIntentHandler.setCreateCredentialException(result, exception)
    setResult(RESULT_OK, result)
    finish()
  }
}
