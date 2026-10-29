package io.github.olegg90.pswmanager

import android.os.Bundle
import androidx.appcompat.app.AppCompatActivity
import kotlin.concurrent.thread

/**
 * Filling in the entry the user picked: its user name and password go to the
 * fields, if the entry is for who asks. An entry for the app's package that
 * keeps no certificate goes only once the user confirmed the app
 * ([confirmApp]), as for the credential provider.
 */
class AutofillLoginActivity : AppCompatActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    Screenshots.apply(this)
    if (savedInstanceState != null) return cancel()
    val id = intent.getStringExtra(ProviderBridge.EXTRA_ENTRY) ?: return cancel()
    val (fields, caller) = PswmAutofillService.fieldsOf(intent) ?: return cancel()
    release(id, intent.getStringExtra(ProviderBridge.EXTRA_TITLE) ?: "", fields, caller, false)
  }

  private fun release(id: String, title: String, fields: AutofillFields, caller: Caller, confirmed: Boolean) {
    thread {
      val answer = Answer.of(ProviderBridge.login(id, caller.origin, caller.packageName, caller.certificate, confirmed))
      runOnUiThread {
        when (answer) {
          is Answer.Ok -> {
            setResult(RESULT_OK, PswmAutofillService.filled(this, fields, answer.json))
            finish()
          }
          is Answer.Unconfirmed -> confirmApp(this, caller.packageName, title, answer.certificate, { release(id, title, fields, caller, true) }, ::cancel)
          is Answer.Failed -> cancel()
        }
      }
    }
  }

  private fun cancel() {
    setResult(RESULT_CANCELED)
    finish()
  }
}
