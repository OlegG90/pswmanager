package io.github.olegg90.pswmanager

import android.content.Intent
import android.os.Bundle
import android.view.Gravity
import android.view.WindowManager
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import androidx.activity.result.contract.ActivityResultContracts
import androidx.annotation.RequiresApi
import androidx.appcompat.app.AppCompatActivity
import androidx.credentials.CreatePublicKeyCredentialRequest
import androidx.credentials.CreatePublicKeyCredentialResponse
import androidx.credentials.exceptions.CreateCredentialCancellationException
import androidx.credentials.exceptions.CreateCredentialException
import androidx.credentials.exceptions.CreateCredentialUnknownException
import androidx.credentials.exceptions.domerrors.InvalidStateError
import androidx.credentials.exceptions.publickeycredential.CreatePublicKeyCredentialDomException
import androidx.credentials.provider.PendingIntentHandler
import org.json.JSONArray
import org.json.JSONObject
import kotlin.concurrent.thread

/**
 * Saving a new passkey (#163), when the user picked PswManager to keep it:
 * unlocked first if need be ([UnlockActivity]), then where it goes (a new
 * entry for the site, or one of the site's entries without a passkey), the
 * user verified, and the core makes it and saves the database; it goes up
 * with the next sync. The site's passkey already in the database: none made.
 */
@RequiresApi(34)
class CreatePasskeyActivity : AppCompatActivity() {
  private lateinit var request: CreatePublicKeyCredentialRequest
  private lateinit var caller: Caller

  private val unlock = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
    if (result.resultCode == RESULT_OK && ProviderBridge.isUnlocked()) choose() else cancel()
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    window.setFlags(WindowManager.LayoutParams.FLAG_SECURE, WindowManager.LayoutParams.FLAG_SECURE)
    // Made again (the choice is gone with the old one): Android hears it was cancelled.
    if (savedInstanceState != null) return cancel()
    val provided = PendingIntentHandler.retrieveProviderCreateCredentialRequest(intent)
    request = provided?.callingRequest as? CreatePublicKeyCredentialRequest ?: return fail("Nothing to make a passkey for")
    caller = try {
      Caller.of(this, provided.callingAppInfo)
    } catch (e: Exception) {
      return fail("Cannot tell who asks: ${e.message}")
    }
    if (ProviderBridge.isUnlocked()) choose() else unlock.launch(Intent(this, UnlockActivity::class.java))
  }

  /** Where the passkey goes: a new entry, or one of the site's own. */
  private fun choose() {
    val found = ProviderBridge.entriesForNewPasskey(request.requestJson)
      ?: return finishWith(CreatePublicKeyCredentialDomException(InvalidStateError(), "PswManager has a passkey for this site already"))
    val entries = JSONArray(found)
    val site = try {
      JSONObject(request.requestJson).getJSONObject("rp").let { it.optString("name").ifEmpty { it.optString("id") } }
    } catch (e: Exception) {
      ""
    }
    val pad = (24 * resources.displayMetrics.density).toInt()
    val list = LinearLayout(this).apply {
      orientation = LinearLayout.VERTICAL
      gravity = Gravity.CENTER_VERTICAL
      setPadding(pad, pad, pad, pad)
      addView(TextView(this@CreatePasskeyActivity).apply { text = "Save a passkey for $site"; textSize = 22f })
      addView(TextView(this@CreatePasskeyActivity).apply { text = "In PswManager's database, as KeePassXC keeps passkeys." })
      addView(Button(this@CreatePasskeyActivity).apply { text = "New entry"; setOnClickListener { make(null, site) } })
      for (i in 0 until entries.length()) {
        val entry = entries.getJSONObject(i)
        val label = listOf(entry.optString("title"), entry.optString("username")).filter { it.isNotEmpty() }.joinToString(" · ")
        addView(Button(this@CreatePasskeyActivity).apply { text = "Add to $label"; setOnClickListener { make(entry.getString("id"), label) } })
      }
      addView(Button(this@CreatePasskeyActivity).apply { text = "Cancel"; setOnClickListener { cancel() } })
    }
    setContentView(ScrollView(this).apply { addView(list) })
  }

  /** Makes the passkey into entry `id` (a new one when null), once the user is verified. */
  private fun make(id: String?, where: String) {
    verifyUser(this, "Save a passkey", where, {
      thread {
        val answer = ProviderBridge.makePasskey(id, request.requestJson, caller.origin, request.clientDataHash.takeIf { caller.origin != null }, caller.packageName, caller.certificate)
        if (!answer.startsWith(ProviderBridge.FAILED)) UploadWorker.schedule(applicationContext, ProviderBridge.state(this), true)
        runOnUiThread { if (answer.startsWith(ProviderBridge.FAILED)) fail(answer.removePrefix(ProviderBridge.FAILED)) else made(answer) }
      }
    }, ::cancel, ::fail)
  }

  private fun made(json: String) {
    val result = Intent()
    PendingIntentHandler.setCreateCredentialResponse(result, CreatePublicKeyCredentialResponse(json))
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
