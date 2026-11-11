package io.github.olegg90.pswmanager

import android.os.Bundle
import android.text.InputType
import android.view.Gravity
import android.view.inputmethod.EditorInfo
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import kotlin.concurrent.thread

/**
 * A screen PswManager shows over another app that asks for the master
 * password or the fingerprint (the key sealed for biometric unlock, when it
 * is allowed): unlocking ([UnlockScreen]) and confirming an export
 * ([ExportActivity], #381). Rust decides with what was given ([attempt]);
 * a sealed key that no longer opens the database is forgotten. Captured on
 * screen only if the settings allow screenshots.
 */
abstract class PasswordScreen : AppCompatActivity() {
  /** The screen's title. */
  protected abstract val heading: String
  /** The button that goes ahead with the typed password. */
  protected abstract val action: String
  /** Said while [attempt] runs. */
  protected abstract val working: String
  /** The fingerprint prompt's title. */
  protected abstract val promptTitle: String
  /** Said when a new fingerprint or face made the sealed key unusable. */
  protected abstract val invalidated: String

  /** The text under the title; `fingerprint` says whether the fingerprint is offered. */
  protected abstract fun body(fingerprint: Boolean): String

  /** Off the main thread: Rust's answer for the typed password or the sealed key. */
  protected abstract fun attempt(typed: String?, sealed: String?): Answer

  /** [attempt] succeeded: answers the request that opened the screen. */
  protected abstract fun succeeded(answer: Answer.Ok)

  /** Cancel was pressed. */
  protected open fun cancelled() = finish()

  private lateinit var password: EditText
  private lateinit var message: TextView
  private lateinit var actionButton: Button
  private var offered = false
  /** An attempt is running: no second one meanwhile. */
  private var busy = false

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    Screenshots.apply(this)
  }

  /** Shows the screen: the password field, the buttons, the fingerprint when offered. */
  protected fun showScreen() {
    keepOutOfAutofill(this)
    val pad = (24 * resources.displayMetrics.density).toInt()
    password = EditText(this).apply {
      hint = "Master password"
      inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
      imeOptions = EditorInfo.IME_ACTION_DONE
      setOnEditorActionListener { _, _, _ -> submit(text.toString(), null); true }
    }
    message = TextView(this)
    actionButton = Button(this).apply { text = action; setOnClickListener { submit(password.text.toString(), null) } }
    offered = BiometricKey.available(this) && BiometricKey.isStored(this) && ProviderBridge.fingerprintAllowed(ProviderBridge.state(this))
    setContentView(LinearLayout(this).apply {
      orientation = LinearLayout.VERTICAL
      gravity = Gravity.CENTER_VERTICAL
      setPadding(pad, pad, pad, pad)
      addView(TextView(this@PasswordScreen).apply { text = heading; textSize = 22f })
      addView(TextView(this@PasswordScreen).apply { text = body(offered) })
      addView(password)
      addView(message)
      addView(actionButton)
      if (offered) addView(Button(this@PasswordScreen).apply { text = "Use fingerprint"; setOnClickListener { askFingerprint() } })
      addView(Button(this@PasswordScreen).apply { text = "Cancel"; setOnClickListener { cancelled() } })
    })
  }

  /** The fingerprint at once when it is offered, unless the screen was made again (a rotation). */
  protected fun askFirst(savedInstanceState: Bundle?) {
    if (savedInstanceState == null && offered) askFingerprint()
  }

  private fun askFingerprint() {
    if (busy) return
    val cipher = try {
      BiometricKey.decrypting(this) ?: return
    } catch (e: BiometricKey.Invalidated) {
      return show(invalidated)
    } catch (e: Exception) {
      return show(e.message ?: e.toString())
    }
    BiometricKey.prompt(this, promptTitle, cipher, { ready ->
      val sealed = try {
        BiometricKey.open(this, ready)
      } catch (e: Exception) {
        return@prompt show(e.message ?: e.toString())
      }
      submit(null, sealed)
    }) { cancelled, text -> if (!cancelled) show(text) }
  }

  /**
   * Runs [attempt] off the main thread, with the sealed key or the typed
   * password (a Kotlin string cannot be wiped; the field is cleared when this
   * closes).
   */
  private fun submit(typed: String?, sealed: String?) {
    if (busy) return
    busy = true
    actionButton.isEnabled = false
    show(working)
    thread {
      val answer = attempt(typed, sealed)
      if (answer is Answer.Failed && answer.stale) BiometricKey.forget(applicationContext)
      runOnUiThread {
        busy = false
        actionButton.isEnabled = true
        if (answer is Answer.Ok) succeeded(answer) else show(answer.why)
      }
    }
  }

  override fun onDestroy() {
    if (::password.isInitialized) password.text.clear()
    super.onDestroy()
  }

  protected fun show(text: String) {
    message.text = text
  }
}
