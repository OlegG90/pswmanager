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
 * [AutofillSaveActivity] saves it, unlocking first.
 */
class PswmAutofillService : AutofillService() {
  override fun onFillRequest(request: FillRequest, cancellationSignal: CancellationSignal, callback: FillCallback) {
    val fields = request.fillContexts.lastOrNull()?.structure?.let { AutofillFields.of(it) }
    // PswManager's own screens are not filled from it.
    if (fields == null || fields.packageName == packageName) return callback.onSuccess(null)
    // Off the main thread: a sync or a save may hold the database a moment.
    thread {
      val response = try {
        respond(fields)
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
    val fields = request.fillContexts.lastOrNull()?.structure?.let { AutofillFields.of(it) }
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
     * The entries for who asks, each a dataset that fills the fields once it
     * is picked and [AutofillLoginActivity] has its values, at most
     * [MAX_OFFERS]; last, choosing another entry ([AutofillPickActivity]),
     * which finds the rest too; and the save info for a login typed in.
     */
    fun offers(context: Context, fields: AutofillFields, caller: Caller): FillResponse {
      val found = JSONArray(ProviderBridge.logins(caller.origin, caller.packageName, caller.certificate) ?: "[]")
      val response = FillResponse.Builder()
      saveInfo(fields)?.let { response.setSaveInfo(it) }
      // Capped: every offer goes to Android in one binder call, whose size is limited.
      for (i in 0 until minOf(found.length(), MAX_OFFERS)) {
        val login = found.getJSONObject(i)
        val id = login.getString("id")
        val pick = Intent(context, AutofillLoginActivity::class.java)
          .putExtra(ProviderBridge.EXTRA_ENTRY, id)
          .putExtra(ProviderBridge.EXTRA_TITLE, login.optString("title"))
          .putFields(fields, caller, "login/$id")
        response.addDataset(offer(context, fields, entryLabel(login).ifEmpty { "PswManager entry" }, pick, 2))
      }
      val choose = Intent(context, AutofillPickActivity::class.java).putFields(fields, caller, "pick")
      val label = when {
        found.length() == 0 -> "Choose an entry in PswManager…"
        found.length() > MAX_OFFERS -> "More entries in PswManager…"
        else -> "Another entry in PswManager…"
      }
      response.addDataset(offer(context, fields, label, choose, 4))
      return response.build()
    }

    /** An offer under the fields, with no values yet: they come once `intent`'s activity has them. */
    private fun offer(context: Context, fields: AutofillFields, label: String, intent: Intent, code: Int): Dataset {
      val view = presentation(context, label)
      val dataset = Dataset.Builder(view)
      for (field in fields.ids) dataset.setValue(field, null, view)
      return dataset.setAuthentication(pendingIntent(context, intent, code).intentSender).build()
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
      setIdentifier("$what|${fields.packageName}|${caller.origin}|${fields.username}|${fields.password}")
      .putExtra(EXTRA_PACKAGE, fields.packageName)
      .putExtra(EXTRA_DOMAIN, fields.webDomain)
      .putExtra(EXTRA_SCHEME, fields.webScheme)
      .putExtra(EXTRA_USERNAME, fields.username)
      .putExtra(EXTRA_PASSWORD, fields.password)
      .putExtra(EXTRA_ORIGIN, caller.origin)
      .putExtra(EXTRA_CERTIFICATE, caller.certificate)

    /** The fields and who asks, as [putFields] put them into the intent. */
    @Suppress("DEPRECATION")
    fun fieldsOf(intent: Intent): Pair<AutofillFields, Caller>? {
      val packageName = intent.getStringExtra(EXTRA_PACKAGE) ?: return null
      val fields = AutofillFields(
        packageName,
        intent.getStringExtra(EXTRA_DOMAIN),
        intent.getStringExtra(EXTRA_SCHEME),
        intent.getParcelableExtra<AutofillId>(EXTRA_USERNAME),
        intent.getParcelableExtra<AutofillId>(EXTRA_PASSWORD),
      )
      return fields to Caller(intent.getStringExtra(EXTRA_ORIGIN), packageName, intent.getByteArrayExtra(EXTRA_CERTIFICATE))
    }
  }
}
