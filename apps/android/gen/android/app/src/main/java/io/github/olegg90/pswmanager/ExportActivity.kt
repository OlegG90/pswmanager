package io.github.olegg90.pswmanager

import android.app.Activity
import android.content.Context
import android.os.Bundle
import android.text.InputType
import android.util.Base64
import android.view.Gravity
import android.view.WindowManager
import android.view.inputmethod.EditorInfo
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import androidx.credentials.providerevents.IntentHandler
import androidx.credentials.providerevents.exception.ImportCredentialsNoExportOptionException
import androidx.credentials.providerevents.transfer.ImportCredentialsResponse
import androidx.credentials.providerevents.transfer.ProviderImportCredentialsRequest
import java.security.SecureRandom
import kotlin.concurrent.thread

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

  /** The database registered, as the importing app shows it. */
  fun name(context: Context): String? = prefs(context).getString(NAME, null)

  fun registered(context: Context, name: String) = prefs(context).edit().putString(NAME, name).apply()

  /** Cleared: a new id next time, so a request for the old one is refused. */
  fun cleared(context: Context) = prefs(context).edit().clear().apply()
}

/**
 * Another password manager imports from PswManager through Android's
 * Credential Transfer (#153): the system starts this activity with the
 * request. It is checked (Google Play services asks, with PswManager's own
 * secret id), the user sees where the entries go and confirms with the
 * fingerprint or the master password, every time, unlocked or not; Rust then
 * opens the database with that key and hands over its entries in use as CXF
 * JSON, which goes to the system's content URI. Never captured on screen.
 */
class ExportActivity : AppCompatActivity() {
  private lateinit var request: ProviderImportCredentialsRequest
  private lateinit var password: EditText
  private lateinit var message: TextView
  private lateinit var exportButton: Button
  /** An export is running: no second one meanwhile. */
  private var busy = false

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    window.setFlags(WindowManager.LayoutParams.FLAG_SECURE, WindowManager.LayoutParams.FLAG_SECURE)
    request = IntentHandler.retrieveProviderImportCredentialsRequest(intent) ?: return cancel()
    if (callingPackage != PLAY_SERVICES || request.credId != ExportRegistration.id(this)) {
      return refuse("PswManager exports only through Android's transfer")
    }
    val destination = label(request.callingAppInfo.packageName)
    val database = ExportRegistration.name(this) ?: "PswManager"
    val pad = (24 * resources.displayMetrics.density).toInt()
    password = EditText(this).apply {
      hint = "Master password"
      inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
      imeOptions = EditorInfo.IME_ACTION_DONE
      setOnEditorActionListener { _, _, _ -> exportWith(text.toString(), null); true }
    }
    message = TextView(this)
    exportButton = Button(this).apply { text = "Export"; setOnClickListener { exportWith(password.text.toString(), null) } }
    val offered = fingerprintOffered()
    setContentView(LinearLayout(this).apply {
      orientation = LinearLayout.VERTICAL
      gravity = Gravity.CENTER_VERTICAL
      setPadding(pad, pad, pad, pad)
      addView(TextView(this@ExportActivity).apply { text = "Export to $destination"; textSize = 22f })
      addView(TextView(this@ExportActivity).apply {
        text = "Every entry in use in “$database” goes to $destination: passwords, passkeys, TOTP secrets, notes and " +
          "fields. History, icons and files stay here; nothing here changes. Confirm with the master password" +
          (if (offered) " or the fingerprint." else ".")
      })
      addView(password)
      addView(message)
      addView(exportButton)
      if (offered) addView(Button(this@ExportActivity).apply { text = "Use fingerprint"; setOnClickListener { askFingerprint() } })
      addView(Button(this@ExportActivity).apply { text = "Cancel"; setOnClickListener { cancel() } })
    })
    if (savedInstanceState == null && offered) askFingerprint()
  }

  /** The app the entries go to, by its name when Android tells it. */
  private fun label(packageName: String): String =
    try {
      packageManager.getApplicationLabel(packageManager.getApplicationInfo(packageName, 0)).toString()
    } catch (e: Exception) {
      packageName
    }

  private fun fingerprintOffered(): Boolean =
    BiometricKey.available(this) && BiometricKey.isStored(this) && ProviderBridge.fingerprintAllowed(ProviderBridge.state(this))

  private fun askFingerprint() {
    if (busy) return
    val cipher = try {
      BiometricKey.decrypting(this) ?: return
    } catch (e: BiometricKey.Invalidated) {
      return show("A new fingerprint or face was added: confirm with the master password")
    } catch (e: Exception) {
      return show(e.message ?: e.toString())
    }
    BiometricKey.prompt(this, "Export from PswManager", cipher, { ready ->
      val sealed = try {
        BiometricKey.open(this, ready)
      } catch (e: Exception) {
        return@prompt show(e.message ?: e.toString())
      }
      exportWith(null, sealed)
    }) { cancelled, text -> if (!cancelled) show(text) }
  }

  /**
   * Exports off the main thread, with the sealed key or the typed password
   * (a Kotlin string cannot be wiped; the field is cleared when this closes).
   */
  private fun exportWith(typed: String?, sealed: String?) {
    if (busy) return
    busy = true
    exportButton.isEnabled = false
    show("Exporting…")
    thread {
      val answer = ProviderBridge.export(applicationContext, ProviderBridge.state(this), typed, sealed)
      runOnUiThread {
        busy = false
        exportButton.isEnabled = true
        when {
          answer == null -> show("PswManager could not export: try again")
          answer.startsWith(ProviderBridge.FAILED) -> {
            val why = answer.removePrefix(ProviderBridge.FAILED)
            if (why.startsWith(ProviderBridge.STALE)) BiometricKey.forget(applicationContext)
            show(why.removePrefix(ProviderBridge.STALE))
          }
          else -> respond(answer)
        }
      }
    }
  }

  /** The export goes to the system's content URI; the importer reads it from there. */
  private fun respond(json: String) {
    try {
      IntentHandler.setImportCredentialsResponse(this, request.uri, intent, ImportCredentialsResponse(json))
    } catch (e: Exception) {
      return show(e.message ?: e.toString())
    }
    setResult(Activity.RESULT_OK, intent)
    finish()
  }

  /** Refused, with why, for the importing app (it is read from the result on RESULT_OK). */
  private fun refuse(why: String) {
    IntentHandler.setImportCredentialsException(intent, ImportCredentialsNoExportOptionException(why))
    setResult(Activity.RESULT_OK, intent)
    finish()
  }

  private fun cancel() {
    setResult(Activity.RESULT_CANCELED)
    finish()
  }

  override fun onDestroy() {
    if (::password.isInitialized) password.text.clear()
    super.onDestroy()
  }

  private fun show(text: String) {
    message.text = text
  }

  companion object {
    /** Who starts the export: Android's Credential Transfer, in Google Play services. */
    private const val PLAY_SERVICES = "com.google.android.gms"
  }
}
