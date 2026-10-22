package io.github.olegg90.pswmanager

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Bundle
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
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
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
 * Keepass2Android ([AutofillPickActivity], #270). A password typed in is offered to save (#270): Android asks *Save to
 * PswManager?*, and [AutofillSaveActivity] saves it, unlocking first.
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
    val unlock = Intent(this, AutofillUnlockActivity::class.java).setIdentifier(fields.packageName).putFields(fields, caller)
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
      .putFields(fields, caller)
      .putExtra(EXTRA_TYPED_USERNAME, typedUsername(request, fields))
      .putExtra(EXTRA_TYPED_PASSWORD, password)
    // Immutable and once: nothing is added to it, and the password in it is not sent twice.
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
     * is picked and [AutofillLoginActivity] has its values; last, choosing
     * another entry ([AutofillPickActivity]); and the save info for a login
     * typed in.
     */
    fun offers(context: Context, fields: AutofillFields, caller: Caller): FillResponse {
      val found = JSONArray(ProviderBridge.logins(caller.origin, caller.packageName, caller.certificate) ?: "[]")
      val response = FillResponse.Builder()
      saveInfo(fields)?.let { response.setSaveInfo(it) }
      for (i in 0 until found.length()) {
        val login = found.getJSONObject(i)
        val id = login.getString("id")
        // One intent per app and entry: an offer to another app does not replace this one.
        val pick = Intent(context, AutofillLoginActivity::class.java)
          .setIdentifier("${fields.packageName}/$id")
          .putExtra(ProviderBridge.EXTRA_ENTRY, id)
          .putExtra(ProviderBridge.EXTRA_TITLE, login.optString("title"))
          .putFields(fields, caller)
        response.addDataset(offer(context, fields, entryLabel(login).ifEmpty { "PswManager entry" }, pick, 2))
      }
      val choose = Intent(context, AutofillPickActivity::class.java).setIdentifier("${fields.packageName}/pick").putFields(fields, caller)
      val label = if (found.length() == 0) "Choose an entry in PswManager…" else "Another entry in PswManager…"
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

    /** The user name and password typed in, for [AutofillSaveActivity]. */
    internal const val EXTRA_TYPED_USERNAME = "typedUsername"
    internal const val EXTRA_TYPED_PASSWORD = "typedPassword"

    private fun Intent.putFields(fields: AutofillFields, caller: Caller): Intent = putExtra(EXTRA_PACKAGE, fields.packageName)
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

/** Unlocking for an autofill request: the fields then offer the entries for who asks. */
class AutofillUnlockActivity : UnlockScreen() {
  override val purpose = "To fill in a password another app asks for."

  override fun done() {
    val (fields, caller) = PswmAutofillService.fieldsOf(intent) ?: return finish()
    val offers = PswmAutofillService.offers(this, fields, caller)
    setResult(RESULT_OK, Intent().putExtra(AutofillManager.EXTRA_AUTHENTICATION_RESULT, offers))
    finish()
  }
}

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
      val answer = ProviderBridge.login(id, caller.origin, caller.packageName, caller.certificate, false)
      runOnUiThread {
        if (answer == null || answer.startsWith(ProviderBridge.UNCONFIRMED)) return@runOnUiThread finish()
        setResult(RESULT_OK, PswmAutofillService.filled(this, fields, JSONObject(answer)))
        finish()
      }
    }
  }

  private fun fail(why: String) {
    Toast.makeText(applicationContext, why, Toast.LENGTH_SHORT).show()
    finish()
  }
}

/**
 * Saving a login typed into another app (#270), once the user chose *Save*
 * in Android's *Save to PswManager?*: unlocked first if need be, then where
 * it goes ([SaveLogin]), as for a login offered through Credential Manager.
 */
class AutofillSaveActivity : UnlockScreen() {
  override val purpose = "To save a password typed into another app."

  override fun done() {
    val (_, caller) = PswmAutofillService.fieldsOf(intent) ?: return finish()
    val username = intent.getStringExtra(PswmAutofillService.EXTRA_TYPED_USERNAME).orEmpty()
    val password = intent.getStringExtra(PswmAutofillService.EXTRA_TYPED_PASSWORD) ?: return finish()
    SaveLogin(this, caller, username, password, { tell("Saved in PswManager") }, ::finish, ::tell).choose()
  }

  /** Android's save sheet is gone by now: the outcome is told in a toast. */
  private fun tell(text: String) {
    Toast.makeText(applicationContext, text, Toast.LENGTH_SHORT).show()
    finish()
  }
}

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
      val answer = ProviderBridge.login(id, caller.origin, caller.packageName, caller.certificate, confirmed)
      runOnUiThread {
        when {
          answer == null -> cancel()
          answer.startsWith(ProviderBridge.UNCONFIRMED) ->
            confirmApp(this, caller.packageName, title, answer.removePrefix(ProviderBridge.UNCONFIRMED), { release(id, title, fields, caller, true) }, ::cancel)
          else -> {
            setResult(RESULT_OK, PswmAutofillService.filled(this, fields, JSONObject(answer)))
            finish()
          }
        }
      }
    }
  }

  private fun cancel() {
    setResult(RESULT_CANCELED)
    finish()
  }
}
