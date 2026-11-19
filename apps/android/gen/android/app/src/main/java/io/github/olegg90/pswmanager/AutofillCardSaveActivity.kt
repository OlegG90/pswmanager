package io.github.olegg90.pswmanager

import android.view.Gravity
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast
import org.json.JSONObject
import kotlin.concurrent.thread

/**
 * Saving a card typed into another app or page (#412), once the user chose
 * *Save* in Android's *Save to PswManager?*: unlocked first if need be, then
 * where it goes, as a login does ([SaveLogin]): the card with the same number
 * first, a new card, or another card, verified with the fingerprint or the
 * screen lock. It goes up with the next sync; a toast names the entry it went
 * to. A card is not for one app or site, so nothing is linked.
 */
class AutofillCardSaveActivity : UnlockScreen() {
  override val purpose = "To save a card typed into another app."

  private var cloud = true
  /** A card is being saved: no second save meanwhile. */
  private var busy = false

  override fun done() {
    val (_, caller) = PswmAutofillService.fieldsOf(intent) ?: return finish()
    val token = intent.getStringExtra(PswmAutofillService.EXTRA_TYPED) ?: return finish()
    val card = PswmAutofillService.typedCards.take(token) ?: return tell("The card typed in is gone: type it again to save it")
    // Off the main thread: a sync or a save may hold the database a moment.
    thread {
      val answer = Answer.of(ProviderBridge.cardChoices(ProviderBridge.state(this), caller.origin, caller.packageName, caller.certificate, card.number))
      runOnUiThread { if (!isDestroyed) show(answer, card, caller) }
    }
  }

  /** Where the card can go, as buttons: the same card first, a new one, the others. */
  private fun show(answer: Answer, card: TypedCard, caller: Caller) {
    if (answer !is Answer.Ok) return tell(answer.why)
    cloud = answer.json.getBoolean("cloud")
    val entries = answer.json.getJSONArray("entries").let { all -> (0 until all.length()).map { all.getJSONObject(it) } }
    val label = { entry: JSONObject -> listOf(entry.optString("title"), entry.optString("summary")).filter { it.isNotEmpty() }.joinToString(" · ") }
    val pad = (24 * resources.displayMetrics.density).toInt()
    val list = LinearLayout(this).apply {
      orientation = LinearLayout.VERTICAL
      gravity = Gravity.CENTER_VERTICAL
      setPadding(pad, pad, pad, pad)
      addView(TextView(this@AutofillCardSaveActivity).apply { text = "Save the card ending ${CardValues.digits(card.number).takeLast(4)}"; textSize = 22f })
      addView(TextView(this@AutofillCardSaveActivity).apply { text = "Typed into ${caller.label}." })
      val (same, others) = entries.partition { it.optBoolean("same") }
      for (entry in same) addView(choice("Update ${label(entry)} (the same card)") { save(entry.getString("id"), label(entry), card) })
      addView(choice("New card") { save(null, "New card", card) })
      for (entry in others) addView(choice("Update ${label(entry)}") { save(entry.getString("id"), label(entry), card) })
      addView(choice("Cancel") { finish() })
    }
    setContentView(ScrollView(this).apply { addView(list) })
  }

  private fun choice(text: String, action: () -> Unit) = Button(this).apply {
    this.text = text
    isAllCaps = false
    setOnClickListener { action() }
  }

  /** Saves into card `id` (a new one when null), once the user is verified. */
  private fun save(id: String?, entry: String, card: TypedCard) {
    if (busy) return
    busy = true
    verifyUser(this, "Save the card", entry, { store(id, card) }, { busy = false }, ::tell)
  }

  private fun store(id: String?, card: TypedCard) {
    val (_, caller) = PswmAutofillService.fieldsOf(intent) ?: return finish()
    thread {
      val answer = Answer.of(ProviderBridge.saveCard(id, caller.origin, caller.packageName, caller.certificate, card.name, card.number, card.expiry, card.code))
      if (answer is Answer.Ok) UploadWorker.schedule(applicationContext, ProviderBridge.state(this), cloud)
      runOnUiThread {
        // The screen closes and work goes on in the other app: say where the card is.
        if (answer is Answer.Ok) tell("Saved in PswManager: ${answer.json.optString("title")}") else tell(answer.why)
      }
    }
  }

  /** Android's save sheet is gone by now: the outcome is told in a toast. */
  private fun tell(text: String) {
    Toast.makeText(applicationContext, text, Toast.LENGTH_LONG).show()
    finish()
  }
}
