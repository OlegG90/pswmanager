package io.github.olegg90.pswmanager

import android.content.Intent
import android.os.Bundle
import android.view.Gravity
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
  /** The database syncs with a cloud store: the upload waits for a network. */
  private var cloud = true
  /** A passkey is being made: no second one meanwhile. */
  private var busy = false

  private val unlock = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
    if (!::request.isInitialized) return@registerForActivityResult
    if (result.resultCode == RESULT_OK && ProviderBridge.isUnlocked()) choose() else cancel()
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    Screenshots.apply(this)
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
    val answer = ProviderBridge.newPasskeyChoices(ProviderBridge.state(this), request.requestJson)
    if (answer.startsWith(ProviderBridge.FAILED)) return fail(answer.removePrefix(ProviderBridge.FAILED))
    val choices = JSONObject(answer)
    if (choices.getBoolean("excluded")) {
      return finishWith(CreatePublicKeyCredentialDomException(InvalidStateError(), "PswManager has a passkey for this site already"))
    }
    cloud = choices.getBoolean("cloud")
    val site = choices.getString("site")
    val entries = choices.getJSONArray("entries")
    val pad = (24 * resources.displayMetrics.density).toInt()
    val list = LinearLayout(this).apply {
      orientation = LinearLayout.VERTICAL
      gravity = Gravity.CENTER_VERTICAL
      setPadding(pad, pad, pad, pad)
      addView(TextView(this@CreatePasskeyActivity).apply { text = "Save a passkey for $site"; textSize = 22f })
      addView(TextView(this@CreatePasskeyActivity).apply { text = "Asked by ${caller.label}. Kept in PswManager's database, as KeePassXC keeps passkeys." })
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

  /** Makes the passkey into entry `id` (a new one when null), once the user is verified; `entry` names it. */
  private fun make(id: String?, entry: String) {
    if (busy) return
    busy = true
    verifyUser(this, "Save a passkey", entry, {
      thread {
        val answer = ProviderBridge.makePasskey(id, request.requestJson, caller.origin, caller.clientDataHash(request.clientDataHash), caller.packageName, caller.certificate)
        // Saved: it goes up even if the app is not running (WorkManager keeps one such upload).
        if (!answer.startsWith(ProviderBridge.FAILED)) UploadWorker.schedule(applicationContext, ProviderBridge.state(this), cloud)
        runOnUiThread { if (answer.startsWith(ProviderBridge.FAILED)) fail(answer.removePrefix(ProviderBridge.FAILED)) else made(answer) }
      }
    }, { busy = false; cancel() }, ::fail)
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
