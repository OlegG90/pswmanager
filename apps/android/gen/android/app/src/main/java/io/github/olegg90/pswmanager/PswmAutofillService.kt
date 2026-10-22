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
import android.service.autofill.SaveRequest
import android.view.autofill.AutofillId
import android.view.autofill.AutofillManager
import android.view.autofill.AutofillValue
import android.widget.RemoteViews
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
 * Saving what was typed is not offered yet.
 */
class PswmAutofillService : AutofillService() {
  override fun onFillRequest(request: FillRequest, cancellationSignal: CancellationSignal, callback: FillCallback) {
    val fields = request.fillContexts.lastOrNull()?.structure?.let { AutofillFields.of(it) }
    // PswManager's own screens are not filled from it.
    if (fields == null || fields.packageName == packageName) return callback.onSuccess(null)
    val caller = Caller.forAutofill(this, fields.packageName, fields.webDomain) ?: return callback.onSuccess(null)
    if (!ProviderBridge.isUnlocked()) {
      val unlock = Intent(this, AutofillUnlockActivity::class.java).putFields(fields, caller)
      val sender = pendingIntent(this, unlock, 1).intentSender
      return callback.onSuccess(FillResponse.Builder().setAuthentication(fields.ids.toTypedArray(), sender, presentation(this, "Unlock PswManager")).build())
    }
    callback.onSuccess(offers(this, fields, caller))
  }

  override fun onSaveRequest(request: SaveRequest, callback: SaveCallback) {
    // No save info is ever offered, so Android does not ask; nothing is kept if it does.
    callback.onSuccess()
  }

  companion object {
    /**
     * The entries for who asks, each a dataset that fills the fields once it
     * is picked and [AutofillLoginActivity] has its values; null when there
     * are none (or the database is locked again).
     */
    fun offers(context: Context, fields: AutofillFields, caller: Caller): FillResponse? {
      val found = JSONArray(ProviderBridge.logins(caller.origin, caller.packageName, caller.certificate) ?: "[]")
      if (found.length() == 0) return null
      val response = FillResponse.Builder()
      for (i in 0 until found.length()) {
        val login = found.getJSONObject(i)
        val id = login.getString("id")
        val title = login.optString("title")
        val label = listOf(title, login.optString("username")).filter { it.isNotEmpty() }.joinToString(" · ").ifEmpty { "PswManager entry" }
        val pick = Intent(context, AutofillLoginActivity::class.java)
          .setIdentifier(id)
          .putExtra(ProviderBridge.EXTRA_ENTRY, id)
          .putExtra(ProviderBridge.EXTRA_TITLE, title)
          .putFields(fields, caller)
        val view = presentation(context, label)
        val dataset = Dataset.Builder(view)
        // No values yet: they come from the entry once it is picked.
        for (field in fields.ids) dataset.setValue(field, null, view)
        dataset.setAuthentication(pendingIntent(context, pick, 2).intentSender)
        response.addDataset(dataset.build())
      }
      return response.build()
    }

    /** One line of the offer, as Android shows it under the field. */
    fun presentation(context: Context, text: String): RemoteViews =
      RemoteViews(context.packageName, android.R.layout.simple_list_item_1).apply { setTextViewText(android.R.id.text1, text) }

    /** Mutable: Android adds the screen's structure to the intent. */
    private fun pendingIntent(context: Context, intent: Intent, code: Int): PendingIntent =
      PendingIntent.getActivity(context, code, intent, PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)

    private const val EXTRA_PACKAGE = "package"
    private const val EXTRA_DOMAIN = "domain"
    private const val EXTRA_USERNAME = "username"
    private const val EXTRA_PASSWORD = "password"
    private const val EXTRA_ORIGIN = "origin"
    private const val EXTRA_CERTIFICATE = "certificate"

    private fun Intent.putFields(fields: AutofillFields, caller: Caller): Intent = putExtra(EXTRA_PACKAGE, fields.packageName)
      .putExtra(EXTRA_DOMAIN, fields.webDomain)
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
    if (offers == null) {
      setResult(RESULT_CANCELED)
    } else {
      setResult(RESULT_OK, Intent().putExtra(AutofillManager.EXTRA_AUTHENTICATION_RESULT, offers))
    }
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
          else -> fill(fields, JSONObject(answer))
        }
      }
    }
  }

  private fun fill(fields: AutofillFields, login: JSONObject) {
    val view = PswmAutofillService.presentation(this, "PswManager")
    val dataset = Dataset.Builder(view)
    fields.username?.let { dataset.setValue(it, AutofillValue.forText(login.getString("username")), view) }
    fields.password?.let { dataset.setValue(it, AutofillValue.forText(login.getString("password")), view) }
    setResult(RESULT_OK, Intent().putExtra(AutofillManager.EXTRA_AUTHENTICATION_RESULT, dataset.build()))
    finish()
  }

  private fun cancel() {
    setResult(RESULT_CANCELED)
    finish()
  }
}
