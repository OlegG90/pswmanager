package io.github.olegg90.pswmanager

import android.widget.Toast
import kotlin.concurrent.thread

/**
 * Picking any entry for an app or site that none is for yet, or another one
 * (#270): unlocked first if need be, the user searches the entries, and the
 * one picked is linked to who asks ([linkEntry]; an app only once the user
 * confirmed it) and fills the fields.
 */
class AutofillPickActivity : UnlockScreen() {
  override val purpose = "To fill in a password another app asks for."

  override fun done() {
    val (fields, caller) = PswmAutofillService.fieldsOf(intent) ?: return finish()
    pickEntry(this, "Fill in from which entry?", { id, label ->
      linkEntry(this, id, label, caller, { fill(id, fields, caller) }, ::finish, ::fail)
    }, ::finish)
  }

  /** The entry is for who asks now: its login fills the fields. */
  private fun fill(id: String, fields: AutofillFields, caller: Caller) {
    thread {
      val answer = Answer.of(ProviderBridge.login(id, caller.origin, caller.packageName, caller.certificate, false))
      runOnUiThread {
        if (answer is Answer.Ok) setResult(RESULT_OK, PswmAutofillService.filled(this, fields, answer.json))
        finish()
      }
    }
  }

  private fun fail(why: String) {
    Toast.makeText(applicationContext, why, Toast.LENGTH_SHORT).show()
    finish()
  }
}
