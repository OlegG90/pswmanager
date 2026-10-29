package io.github.olegg90.pswmanager

import android.widget.Toast

/**
 * Saving a login typed into another app (#270), once the user chose *Save*
 * in Android's *Save to PswManager?*: unlocked first if need be, then where
 * it goes ([SaveLogin]), as for a login offered through Credential Manager.
 */
class AutofillSaveActivity : UnlockScreen() {
  override val purpose = "To save a password typed into another app."

  override fun done() {
    val (_, caller) = PswmAutofillService.fieldsOf(intent) ?: return finish()
    val token = intent.getStringExtra(PswmAutofillService.EXTRA_TYPED) ?: return finish()
    val (username, password) = PswmAutofillService.typed.take(token) ?: return tell("The password typed in is gone: type it again to save it")
    SaveLogin(this, caller, username, password, ::finish, ::finish, ::tell).choose()
  }

  /** Android's save sheet is gone by now: the outcome is told in a toast. */
  private fun tell(text: String) {
    Toast.makeText(applicationContext, text, Toast.LENGTH_SHORT).show()
    finish()
  }
}
