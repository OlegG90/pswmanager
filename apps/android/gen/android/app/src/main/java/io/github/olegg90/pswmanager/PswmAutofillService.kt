package io.github.olegg90.pswmanager

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.CancellationSignal
import android.service.autofill.AutofillService
import android.service.autofill.Dataset
import android.service.autofill.FillCallback
import android.service.autofill.FillRequest
import android.service.autofill.FillResponse
import android.service.autofill.SaveCallback
import android.service.autofill.SaveInfo
import android.service.autofill.SaveRequest
import android.view.autofill.AutofillId
import android.view.autofill.AutofillManager
import android.view.autofill.AutofillValue
import android.widget.RemoteViews
import org.json.JSONArray
import org.json.JSONObject
import kotlin.concurrent.thread

/**
 * PswManager as Android's autofill service (#236): the user name and password
 * fields of apps and of pages in browsers offer the entries for them, matched
 * as for the credential provider (an app by its package and signing
 * certificate, a page by its host when a privileged browser shows it; see
 * [Caller.forAutofill]). While the database is locked the only offer is
 * *Unlock PswManager* ([AutofillUnlockActivity]); nothing about entries is
 * told. Unlocked, each entry is offered by its title and user name only:
 * its values leave the database when it is picked ([AutofillLoginActivity]).
 * Any other entry can be picked and linked to the app or site, as in
 * Keepass2Android ([AutofillPickActivity], #270). A password typed in is
 * offered to save (#270): Android asks *Save to PswManager?*, and
 * [AutofillSaveActivity] saves it, unlocking first. A card's fields offer the
 * cards (#412), each by its title, brand and last four; its values go once
 * the user is verified ([AutofillCardActivity]).
 */
class PswmAutofillService : AutofillService() {
  override fun onFillRequest(request: FillRequest, cancellationSignal: CancellationSignal, callback: FillCallback) {
    // Off the main thread: a page in a browser can have thousands of nodes to
    // read, and a sync or a save may hold the database a moment.
    thread {
      val response = try {
        val fields = AutofillFields.of(request.fillContexts.lastOrNull()?.structure)
        // PswManager's own screens are not filled from it.
        if (fields == null || fields.packageName == packageName) null else respond(fields)
      } catch (e: Exception) {
        android.util.Log.w("PswmAutofill", "No offer: ${e.javaClass.simpleName}")
        null
      }
      if (!cancellationSignal.isCanceled) callback.onSuccess(response)
    }
  }

  /** What the fields are offered: *Unlock PswManager* while locked, else the entries for who asks. */
  private fun respond(fields: AutofillFields): FillResponse? {
    val caller = Caller.forAutofill(this, fields) ?: return null
    if (ProviderBridge.isUnlocked()) return offers(this, fields, caller)
    val unlock = Intent(this, AutofillUnlockActivity::class.java).putFields(fields, caller, "unlock")
    val sender = pendingIntent(this, unlock, 1).intentSender
    val response = FillResponse.Builder().setAuthentication(fields.ids.toTypedArray(), sender, presentation(this, "Unlock PswManager"))
    // A login typed in while locked is offered to save too: the save screen unlocks.
    saveInfo(fields)?.let { response.setSaveInfo(it) }
    return response.build()
  }

  /**
   * The user chose *Save* in Android's *Save to PswManager?*: the values typed
   * in, read from the screen (a user name from an earlier step of the same
   * sign-in when this one has none), go to [AutofillSaveActivity], which
   * unlocks if need be and asks where they go.
   */
  override fun onSaveRequest(request: SaveRequest, callback: SaveCallback) {
    val fields = AutofillFields.of(request.fillContexts.lastOrNull()?.structure)
    val password = fields?.passwordText
    if (fields == null || fields.packageName == packageName || password.isNullOrEmpty()) return callback.onFailure("There is no password to save")
    val caller = Caller.forAutofill(this, fields) ?: return callback.onFailure("Cannot tell who asks")
    val save = Intent(this, AutofillSaveActivity::class.java)
      .putFields(fields, caller, "save")
      .putExtra(EXTRA_TYPED, typed.keep(typedUsername(request, fields), password))
    // Immutable and once: nothing is added to it, and it is not answered twice.
    val flags = PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_ONE_SHOT or PendingIntent.FLAG_CANCEL_CURRENT
    callback.onSuccess(PendingIntent.getActivity(this, 3, save, flags).intentSender)
  }

  /** The user name typed beside the password, else in the latest earlier step on the same page. */
  private fun typedUsername(request: SaveRequest, fields: AutofillFields): String =
    fields.usernameText?.ifEmpty { null } ?: request.fillContexts.dropLast(1).asReversed().asSequence()
      .mapNotNull { AutofillFields.of(it.structure) }
      .filter { it.packageName == fields.packageName && it.webDomain == fields.webDomain }
      .firstNotNullOfOrNull { it.usernameText?.ifEmpty { null } }
      .orEmpty()

  companion object {
    /**
     * What the fields are offered once unlocked: the logins for the login
     * fields ([loginOffers]), the cards for a card's ([cardOffers]); Android
     * shows each offer under the fields it fills. Null when there is none.
     */
    fun offers(context: Context, fields: AutofillFields, caller: Caller): FillResponse? {
      val response = FillResponse.Builder()
      var offered = false
      if (fields.loginIds.isNotEmpty()) {
        loginOffers(context, fields, caller, response)
        offered = true
      }
      fields.card?.let { offered = cardOffers(context, fields, it, caller, response) || offered }
      saveInfo(fields)?.let {
        response.setSaveInfo(it)
        offered = true
      }
      return if (offered) response.build() else null
    }

    /**
     * The entries for who asks, each a dataset that fills the login fields once
     * it is picked and [AutofillLoginActivity] has its values, at most
     * [MAX_OFFERS]; last, choosing another entry ([AutofillPickActivity]),
     * which finds the rest too.
     */
    private fun loginOffers(context: Context, fields: AutofillFields, caller: Caller, response: FillResponse.Builder) {
      val found = JSONArray(ProviderBridge.logins(caller.origin, caller.packageName, caller.certificate) ?: "[]")
      // Capped: every offer goes to Android in one binder call, whose size is limited.
      for (i in 0 until minOf(found.length(), MAX_OFFERS)) {
        val login = found.getJSONObject(i)
        val id = login.getString("id")
        val pick = Intent(context, AutofillLoginActivity::class.java)
          .putExtra(ProviderBridge.EXTRA_ENTRY, id)
          .putExtra(ProviderBridge.EXTRA_TITLE, login.optString("title"))
          .putFields(fields, caller, "login/$id")
        response.addDataset(offer(context, fields.loginIds, entryLabel(login).ifEmpty { "PswManager entry" }, pick, 2))
      }
      val choose = Intent(context, AutofillPickActivity::class.java).putFields(fields, caller, "pick")
      val label = when {
        found.length() == 0 -> "Choose an entry in PswManager…"
        found.length() > MAX_OFFERS -> "More entries in PswManager…"
        else -> "Another entry in PswManager…"
      }
      response.addDataset(offer(context, fields.loginIds, label, choose, 4))
    }

    /**
     * The cards (none while turned off in Settings › Modules), each a dataset
     * that fills the card's fields once it is picked and [AutofillCardActivity]
     * verified the user, at most [MAX_OFFERS]: shown by title, brand and last
     * four only. False when there is no card.
     */
    private fun cardOffers(context: Context, fields: AutofillFields, card: CardFields, caller: Caller, response: FillResponse.Builder): Boolean {
      val found = JSONArray(ProviderBridge.cards(caller.origin, caller.packageName, caller.certificate) ?: "[]")
      for (i in 0 until minOf(found.length(), MAX_OFFERS)) {
        val choice = found.getJSONObject(i)
        val id = choice.getString("id")
        val title = choice.optString("title")
        val pick = Intent(context, AutofillCardActivity::class.java)
          .putExtra(ProviderBridge.EXTRA_ENTRY, id)
          .putExtra(ProviderBridge.EXTRA_TITLE, title)
          .putFields(fields, caller, "card/$id")
        val label = listOf(title, choice.optString("summary")).filter { it.isNotEmpty() }.joinToString(" · ")
        response.addDataset(offer(context, card.ids, label.ifEmpty { "PswManager card" }, pick, 5))
      }
      return found.length() > 0
    }

    /** An offer under the fields `ids`, with no values yet: they come once `intent`'s activity has them. */
    private fun offer(context: Context, ids: List<AutofillId>, label: String, intent: Intent, code: Int): Dataset {
      val view = presentation(context, label)
      val dataset = Dataset.Builder(view)
      for (field in ids) dataset.setValue(field, null, view)
      return dataset.setAuthentication(pendingIntent(context, intent, code).intentSender).build()
    }

    /**
     * The card's fields filled with its values (`{name, number, expiry, code}`),
     * as Android gets them back: the number as digits, the expiry as one field
     * takes it (`MM/YY`) or as its month and year.
     */
    fun cardFilled(context: Context, card: CardFields, values: JSONObject): Intent {
      val view = presentation(context, "PswManager")
      val dataset = Dataset.Builder(view)
      val set: (AutofillId?, String) -> Unit = { id, value ->
        if (id != null && value.isNotEmpty()) dataset.setValue(id, AutofillValue.forText(value), view)
      }
      val expiry = values.optString("expiry")
      set(card.number, CardValues.digits(values.getString("number")))
      set(card.code, values.optString("code"))
      set(card.name, values.optString("name"))
      set(card.expiry, CardValues.short(expiry))
      CardValues.monthYear(expiry)?.let { (month, year) ->
        set(card.month, month)
        set(card.year, year)
      }
      return Intent().putExtra(AutofillManager.EXTRA_AUTHENTICATION_RESULT, dataset.build())
    }

    /** The fields filled with an entry's login (`{username, password}`), as Android gets it back. */
    fun filled(context: Context, fields: AutofillFields, login: JSONObject): Intent {
      val view = presentation(context, "PswManager")
      val dataset = Dataset.Builder(view)
      fields.username?.let { dataset.setValue(it, AutofillValue.forText(login.getString("username")), view) }
      fields.password?.let { dataset.setValue(it, AutofillValue.forText(login.getString("password")), view) }
      return Intent().putExtra(AutofillManager.EXTRA_AUTHENTICATION_RESULT, dataset.build())
    }

    /**
     * What Android offers to save once the screen is done with (the activity
     * finished, or the fields gone, as a page in a browser does): the password
     * field, with the user name field when there is one; null without a
     * password field (the first step of a sign-in in two).
     */
    private fun saveInfo(fields: AutofillFields): SaveInfo? {
      val password = fields.password ?: return null
      val info = SaveInfo.Builder(SaveInfo.SAVE_DATA_TYPE_USERNAME or SaveInfo.SAVE_DATA_TYPE_PASSWORD, arrayOf(password))
      fields.username?.let { info.setOptionalIds(arrayOf(it)) }
      return info.setFlags(SaveInfo.FLAG_SAVE_ON_ALL_VIEWS_INVISIBLE).build()
    }

    /** One line of the offer, as Android shows it under the field. */
    fun presentation(context: Context, text: String): RemoteViews =
      RemoteViews(context.packageName, android.R.layout.simple_list_item_1).apply { setTextViewText(android.R.id.text1, text) }

    /** Mutable: Android adds the screen's structure to the intent. */
    private fun pendingIntent(context: Context, intent: Intent, code: Int): PendingIntent =
      PendingIntent.getActivity(context, code, intent, PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)

    private const val EXTRA_PACKAGE = "package"
    private const val EXTRA_DOMAIN = "domain"
    private const val EXTRA_SCHEME = "scheme"
    private const val EXTRA_USERNAME = "username"
    private const val EXTRA_PASSWORD = "password"
    private const val EXTRA_ORIGIN = "origin"
    private const val EXTRA_CERTIFICATE = "certificate"
    private const val EXTRA_CARD_NUMBER = "cardNumber"
    private const val EXTRA_CARD_CODE = "cardCode"
    private const val EXTRA_CARD_EXPIRY = "cardExpiry"
    private const val EXTRA_CARD_MONTH = "cardMonth"
    private const val EXTRA_CARD_YEAR = "cardYear"
    private const val EXTRA_CARD_NAME = "cardName"

    /** Which login typed in [AutofillSaveActivity] saves ([TypedLogins.take]). */
    internal const val EXTRA_TYPED = "typed"

    /** The login typed in, until [AutofillSaveActivity] takes it. */
    internal val typed = TypedLogins()

    /** The most entries offered under the fields; the rest are found through [AutofillPickActivity]. */
    private const val MAX_OFFERS = 10

    /**
     * The fields and who asks, into an intent for `what` the offer does. Its
     * identity is the app, the page and the fields too: a pending intent with
     * the same identity takes the newest extras, and an offer for another site
     * still on screen in a browser would fill from them.
     */
    private fun Intent.putFields(fields: AutofillFields, caller: Caller, what: String): Intent =
      setIdentifier("$what|${fields.packageName}|${caller.origin}|${fields.username}|${fields.password}|${fields.card?.number}")
      .putExtra(EXTRA_PACKAGE, fields.packageName)
      .putExtra(EXTRA_DOMAIN, fields.webDomain)
      .putExtra(EXTRA_SCHEME, fields.webScheme)
      .putExtra(EXTRA_USERNAME, fields.username)
      .putExtra(EXTRA_PASSWORD, fields.password)
      .putExtra(EXTRA_ORIGIN, caller.origin)
      .putExtra(EXTRA_CERTIFICATE, caller.certificate)
      .putExtra(EXTRA_CARD_NUMBER, fields.card?.number)
      .putExtra(EXTRA_CARD_CODE, fields.card?.code)
      .putExtra(EXTRA_CARD_EXPIRY, fields.card?.expiry)
      .putExtra(EXTRA_CARD_MONTH, fields.card?.month)
      .putExtra(EXTRA_CARD_YEAR, fields.card?.year)
      .putExtra(EXTRA_CARD_NAME, fields.card?.name)

    /** The fields and who asks, as [putFields] put them into the intent. */
    @Suppress("DEPRECATION")
    fun fieldsOf(intent: Intent): Pair<AutofillFields, Caller>? {
      val packageName = intent.getStringExtra(EXTRA_PACKAGE) ?: return null
      val id = { name: String -> intent.getParcelableExtra<AutofillId>(name) }
      val card = id(EXTRA_CARD_NUMBER)?.let { CardFields(it, id(EXTRA_CARD_CODE), id(EXTRA_CARD_EXPIRY), id(EXTRA_CARD_MONTH), id(EXTRA_CARD_YEAR), id(EXTRA_CARD_NAME)) }
      val fields = AutofillFields(
        packageName,
        intent.getStringExtra(EXTRA_DOMAIN),
        intent.getStringExtra(EXTRA_SCHEME),
        id(EXTRA_USERNAME),
        id(EXTRA_PASSWORD),
        card = card,
      )
      return fields to Caller(intent.getStringExtra(EXTRA_ORIGIN), packageName, intent.getByteArrayExtra(EXTRA_CERTIFICATE))
    }
  }
}
