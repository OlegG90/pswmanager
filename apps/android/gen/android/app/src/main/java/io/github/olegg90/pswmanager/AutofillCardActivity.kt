package io.github.olegg90.pswmanager

import android.widget.Toast
import kotlin.concurrent.thread

/**
 * Filling in the card the user picked (#412): the user is verified first,
 * every time (the fingerprint or the phone's screen lock), since a card's
 * number and code are not for one app or site; then its fields go to the
 * form. The offer showed only the brand and last four; the full values
 * leave the database here, once verified. An offer stays under the fields
 * after the database locked again (in the background, say): it is unlocked
 * first then, and the unlock is the check.
 */
class AutofillCardActivity : UnlockScreen() {
  override val purpose = "To fill in a card another app asks for."

  /** The database was unlocked here, just now: the user is verified already. */
  private var unlockedHere = false

  override fun succeeded(answer: Answer.Ok) {
    unlockedHere = true
    super.succeeded(answer)
  }

  override fun done() {
    val id = intent.getStringExtra(ProviderBridge.EXTRA_ENTRY) ?: return cancel()
    val (fields, caller) = PswmAutofillService.fieldsOf(intent) ?: return cancel()
    val card = fields.card ?: return cancel()
    if (unlockedHere) return give(id, card, caller)
    val title = intent.getStringExtra(ProviderBridge.EXTRA_TITLE).orEmpty().ifEmpty { "A card" }
    val to = caller.origin ?: appLabel(caller.packageName)
    verifyUser(this, "Fill in a card", "$title, for $to", { give(id, card, caller) }, ::cancel) { why ->
      Toast.makeText(applicationContext, why, Toast.LENGTH_SHORT).show()
      cancel()
    }
  }

  private fun give(id: String, card: CardFields, caller: Caller) {
    thread {
      val answer = Answer.of(ProviderBridge.card(id, caller.origin, caller.packageName, caller.certificate))
      runOnUiThread {
        if (answer is Answer.Ok) {
          setResult(RESULT_OK, PswmAutofillService.cardFilled(this, card, answer.json))
          finish()
        } else {
          Toast.makeText(applicationContext, answer.why, Toast.LENGTH_SHORT).show()
          cancel()
        }
      }
    }
  }

  private fun cancel() {
    setResult(RESULT_CANCELED)
    finish()
  }
}
