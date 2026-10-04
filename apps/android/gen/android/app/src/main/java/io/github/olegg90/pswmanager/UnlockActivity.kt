package io.github.olegg90.pswmanager

import android.content.Intent
import android.os.Bundle
import android.text.InputType
import android.view.Gravity
import android.view.WindowManager
import android.view.inputmethod.EditorInfo
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import androidx.annotation.RequiresApi
import androidx.appcompat.app.AppCompatActivity
import androidx.credentials.provider.BeginGetCredentialResponse
import androidx.credentials.provider.PendingIntentHandler
import kotlin.concurrent.thread

/**
 * Unlocking PswManager for the credential provider, outside the app's window
 * (#161): the fingerprint (the key sealed for biometric unlock) when it is
 * allowed, else the master password. The database is then unlocked in Rust
 * for the app too, and Android gets the provider's answer again, now
 * unlocked. Never captured on screen.
 */
@RequiresApi(34)
class UnlockActivity : AppCompatActivity() {
  private lateinit var password: EditText
  private lateinit var message: TextView
  private lateinit var unlockButton: Button
  /** An unlock is running: no second one meanwhile. */
  private var busy = false

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    window.setFlags(WindowManager.LayoutParams.FLAG_SECURE, WindowManager.LayoutParams.FLAG_SECURE)
    val pad = (24 * resources.displayMetrics.density).toInt()
    password = EditText(this).apply {
      hint = "Master password"
      inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
      imeOptions = EditorInfo.IME_ACTION_DONE
      setOnEditorActionListener { _, _, _ -> unlockWith(text.toString(), null); true }
    }
    message = TextView(this)
    unlockButton = Button(this).apply { text = "Unlock"; setOnClickListener { unlockWith(password.text.toString(), null) } }
    val offered = fingerprintOffered()
    val cancel = Button(this).apply { text = "Cancel"; setOnClickListener { finish() } }
    setContentView(LinearLayout(this).apply {
      orientation = LinearLayout.VERTICAL
      gravity = Gravity.CENTER_VERTICAL
      setPadding(pad, pad, pad, pad)
      addView(TextView(this@UnlockActivity).apply { text = "Unlock PswManager"; textSize = 22f })
      addView(TextView(this@UnlockActivity).apply { text = "For a passkey or password another app asks for." })
      addView(password)
      addView(message)
      addView(unlockButton)
      if (offered) addView(Button(this@UnlockActivity).apply { text = "Use fingerprint"; setOnClickListener { askFingerprint() } })
      addView(cancel)
    })
    if (ProviderBridge.isUnlocked()) return done()
    if (savedInstanceState == null && offered) askFingerprint()
  }

  private fun fingerprintOffered(): Boolean =
    BiometricKey.available(this) && BiometricKey.isStored(this) && ProviderBridge.fingerprintAllowed(ProviderBridge.state(this))

  private fun askFingerprint() {
    if (busy) return
    val cipher = try {
      BiometricKey.decrypting(this) ?: return
    } catch (e: BiometricKey.Invalidated) {
      return show("A new fingerprint or face was added: unlock with the master password, then in PswManager once")
    } catch (e: Exception) {
      return show(e.message ?: e.toString())
    }
    BiometricKey.prompt(this, "Unlock PswManager", cipher, { ready ->
      val sealed = try {
        BiometricKey.open(this, ready)
      } catch (e: Exception) {
        return@prompt show(e.message ?: e.toString())
      }
      unlockWith(null, sealed)
    }) { cancelled, text -> if (!cancelled) show(text) }
  }

  /**
   * Unlocks off the main thread; with the sealed key, or the typed password
   * (a Kotlin string cannot be wiped; the field is cleared when this closes).
   */
  private fun unlockWith(typed: String?, sealed: String?) {
    if (busy) return
    busy = true
    unlockButton.isEnabled = false
    show("Unlocking…")
    thread {
      val failure = ProviderBridge.unlock(applicationContext, ProviderBridge.state(this), typed, sealed)
      if (failure?.startsWith(ProviderBridge.STALE) == true) BiometricKey.forget(applicationContext)
      runOnUiThread {
        busy = false
        unlockButton.isEnabled = true
        if (failure == null) done() else show(failure.removePrefix(ProviderBridge.STALE))
      }
    }
  }

  override fun onDestroy() {
    password.text.clear()
    super.onDestroy()
  }

  private fun show(text: String) {
    message.text = text
  }

  /** Unlocked: Android gets what the site or app asked for. */
  private fun done() {
    val result = Intent()
    val request = PendingIntentHandler.retrieveBeginGetCredentialRequest(intent)
    val response = request?.let { PswmCredentialService.credentials(this, it) } ?: BeginGetCredentialResponse()
    PendingIntentHandler.setBeginGetCredentialResponse(result, response)
    setResult(RESULT_OK, result)
    finish()
  }
}
