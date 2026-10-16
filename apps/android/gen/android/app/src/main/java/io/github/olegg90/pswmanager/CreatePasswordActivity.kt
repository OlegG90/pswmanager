package io.github.olegg90.pswmanager

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.view.Gravity
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import androidx.activity.result.contract.ActivityResultContracts
import androidx.annotation.RequiresApi
import androidx.appcompat.app.AppCompatActivity
import androidx.credentials.CreatePasswordRequest
import androidx.credentials.CreatePasswordResponse
import androidx.credentials.exceptions.CreateCredentialCancellationException
import androidx.credentials.exceptions.CreateCredentialException
import androidx.credentials.exceptions.CreateCredentialUnknownException
import androidx.credentials.provider.PendingIntentHandler
import org.json.JSONObject
import kotlin.concurrent.thread

/**
 * Saving a login an app offers through Credential Manager (#164), when the
 * user picked PswManager: unlocked first if need be, then who asks and where
 * it goes (a new entry for the app or site, or one of its entries, whose
 * password it replaces; the old version goes to history). It goes up with
 * the next sync. An app's entry keeps its signing certificate; one for its
 * package that keeps none yet is changed only once the user confirmed the
 * app ([confirmApp]).
 */
@RequiresApi(34)
class CreatePasswordActivity : AppCompatActivity() {
  private lateinit var request: CreatePasswordRequest
  private lateinit var caller: Caller
  private var cloud = true
  /** A login is being saved: no second save meanwhile. */
  private var busy = false

  private val unlock = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
    if (!::request.isInitialized) return@registerForActivityResult
    if (result.resultCode == RESULT_OK && ProviderBridge.isUnlocked()) choose() else cancel()
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    Screenshots.apply(this)
    if (savedInstanceState != null) return cancel()
    val provided = PendingIntentHandler.retrieveProviderCreateCredentialRequest(intent)
    request = provided?.callingRequest as? CreatePasswordRequest ?: return fail("No password to save")
    caller = try {
      Caller.of(this, provided.callingAppInfo)
    } catch (e: Exception) {
      return fail("Cannot tell who asks: ${e.message}")
    }
    if (ProviderBridge.isUnlocked()) choose() else unlock.launch(Intent(this, UnlockActivity::class.java))
  }

  /** Where the login goes: a new entry, or one of the app's or site's. */
  private fun choose() {
    val answer = ProviderBridge.loginChoices(ProviderBridge.state(this), caller.origin, caller.packageName, caller.certificate)
    if (answer.startsWith(ProviderBridge.FAILED)) return fail(answer.removePrefix(ProviderBridge.FAILED))
    val choices = JSONObject(answer)
    cloud = choices.getBoolean("cloud")
    val entries = choices.getJSONArray("entries")
    val pad = (24 * resources.displayMetrics.density).toInt()
    val list = LinearLayout(this).apply {
      orientation = LinearLayout.VERTICAL
      gravity = Gravity.CENTER_VERTICAL
      setPadding(pad, pad, pad, pad)
      addView(TextView(this@CreatePasswordActivity).apply { text = "Save the password for ${request.id}"; textSize = 22f })
      addView(TextView(this@CreatePasswordActivity).apply { text = "Asked by ${caller.label}." })
      addView(Button(this@CreatePasswordActivity).apply { text = "New entry"; setOnClickListener { save(null, "New entry") } })
      for (i in 0 until entries.length()) {
        val entry = entries.getJSONObject(i)
        val label = listOf(entry.optString("title"), entry.optString("username")).filter { it.isNotEmpty() }.joinToString(" · ")
        addView(Button(this@CreatePasswordActivity).apply { text = "Update $label"; setOnClickListener { save(entry.getString("id"), label) } })
      }
      addView(Button(this@CreatePasswordActivity).apply { text = "Cancel"; setOnClickListener { cancel() } })
    }
    setContentView(ScrollView(this).apply { addView(list) })
  }

  /** Saves into entry `id` (a new one, named after the app or site, when null), once the user is verified. */
  private fun save(id: String?, entry: String) {
    if (busy) return
    if (request.password.isEmpty()) return fail("There is no password to save")
    busy = true
    verifyUser(this, "Save the password", entry, { store(id, entry, false) }, { busy = false; cancel() }, ::fail)
  }

  /** Saves, the user verified; asks to confirm the app first when the entry needs it. */
  private fun store(id: String?, entry: String, confirmed: Boolean) {
    val title = caller.origin?.let { Uri.parse(it).host } ?: appName(caller.packageName)
    thread {
      val answer = ProviderBridge.saveLogin(id, caller.origin, caller.packageName, caller.certificate, title, request.id, request.password, confirmed)
      if (answer.isEmpty()) UploadWorker.schedule(applicationContext, ProviderBridge.state(this), cloud)
      runOnUiThread {
        when {
          answer.isEmpty() -> saved()
          answer.startsWith(ProviderBridge.UNCONFIRMED) ->
            confirmApp(this, caller.packageName, entry, answer.removePrefix(ProviderBridge.UNCONFIRMED), { store(id, entry, true) }, ::cancel)
          else -> fail(answer.removePrefix(ProviderBridge.FAILED))
        }
      }
    }
  }

  /** The app's name as the phone shows it, else its package (Android may hide other apps). */
  private fun appName(packageName: String): String = try {
    packageManager.getApplicationLabel(packageManager.getApplicationInfo(packageName, 0)).toString()
  } catch (e: Exception) {
    packageName
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
