package io.github.olegg90.pswmanager

import android.content.Intent
import android.view.autofill.AutofillManager
import kotlin.concurrent.thread

/** Unlocking for an autofill request: the fields then offer the entries for who asks. */
class AutofillUnlockActivity : UnlockScreen() {
  override val purpose = "To fill in a password another app asks for."

  override fun done() {
    val (fields, caller) = PswmAutofillService.fieldsOf(intent) ?: return finish()
    // Off the main thread: a sync or a save may hold the database a moment.
    thread {
      // As onFillRequest: a failure here offers nothing rather than ending the app.
      val offers = try {
        PswmAutofillService.offers(this, fields, caller)
      } catch (e: Exception) {
        android.util.Log.w("PswmAutofill", "No offer: ${e.javaClass.simpleName}")
        null
      }
      runOnUiThread {
        if (offers == null) setResult(RESULT_CANCELED)
        else setResult(RESULT_OK, Intent().putExtra(AutofillManager.EXTRA_AUTHENTICATION_RESULT, offers))
        finish()
      }
    }
  }
}
