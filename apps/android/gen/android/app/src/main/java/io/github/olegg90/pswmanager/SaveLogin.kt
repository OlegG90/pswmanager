package io.github.olegg90.pswmanager

import android.net.Uri
import android.view.Gravity
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import org.json.JSONObject
import kotlin.concurrent.thread

/**
 * Saving a login another app offers, in `activity` once the database is
 * unlocked (#164, #270): who asks and where it goes (a new entry for the app
 * or site, or one of its entries, whose password it replaces; the old version
 * goes to history), verified with the fingerprint or the screen lock. It goes
 * up with the next sync. An app's entry keeps its signing certificate; one
 * for its package that keeps none yet is changed only once the user confirmed
 * the app ([confirmApp]).
 */
class SaveLogin(
  private val activity: AppCompatActivity,
  private val caller: Caller,
  private val username: String,
  private val password: String,
  private val saved: () -> Unit,
  private val cancelled: () -> Unit,
  private val failed: (String) -> Unit,
) {
  private var cloud = true
  /** A login is being saved: no second save meanwhile. */
  private var busy = false

  /** Shows where the login goes: a new entry, or one of the app's or site's. */
  fun choose() {
    val answer = ProviderBridge.loginChoices(ProviderBridge.state(activity), caller.origin, caller.packageName, caller.certificate)
    if (answer.startsWith(ProviderBridge.FAILED)) return failed(answer.removePrefix(ProviderBridge.FAILED))
    val choices = JSONObject(answer)
    cloud = choices.getBoolean("cloud")
    val entries = choices.getJSONArray("entries")
    val pad = (24 * activity.resources.displayMetrics.density).toInt()
    val list = LinearLayout(activity).apply {
      orientation = LinearLayout.VERTICAL
      gravity = Gravity.CENTER_VERTICAL
      setPadding(pad, pad, pad, pad)
      addView(TextView(activity).apply { text = "Save the password for ${username.ifEmpty { caller.label }}"; textSize = 22f })
      addView(TextView(activity).apply { text = "Asked by ${caller.label}." })
      addView(Button(activity).apply { text = "New entry"; setOnClickListener { save(null, "New entry") } })
      for (i in 0 until entries.length()) {
        val entry = entries.getJSONObject(i)
        val label = listOf(entry.optString("title"), entry.optString("username")).filter { it.isNotEmpty() }.joinToString(" · ")
        addView(Button(activity).apply { text = "Update $label"; setOnClickListener { save(entry.getString("id"), label) } })
      }
      addView(Button(activity).apply { text = "Cancel"; setOnClickListener { cancelled() } })
    }
    activity.setContentView(ScrollView(activity).apply { addView(list) })
  }

  /** Saves into entry `id` (a new one, named after the app or site, when null), once the user is verified. */
  private fun save(id: String?, entry: String) {
    if (busy) return
    if (password.isEmpty()) return failed("There is no password to save")
    busy = true
    verifyUser(activity, "Save the password", entry, { store(id, entry, false) }, { busy = false; cancelled() }, failed)
  }

  /** Saves, the user verified; asks to confirm the app first when the entry needs it. */
  private fun store(id: String?, entry: String, confirmed: Boolean) {
    val title = caller.origin?.let { Uri.parse(it).host } ?: appName(caller.packageName)
    thread {
      val answer = ProviderBridge.saveLogin(id, caller.origin, caller.packageName, caller.certificate, title, username, password, confirmed)
      if (answer.isEmpty()) UploadWorker.schedule(activity.applicationContext, ProviderBridge.state(activity), cloud)
      activity.runOnUiThread {
        when {
          answer.isEmpty() -> saved()
          answer.startsWith(ProviderBridge.UNCONFIRMED) ->
            confirmApp(activity, caller.packageName, entry, answer.removePrefix(ProviderBridge.UNCONFIRMED), { store(id, entry, true) }, cancelled)
          else -> failed(answer.removePrefix(ProviderBridge.FAILED))
        }
      }
    }
  }

  /** The app's name as the phone shows it, else its package (Android may hide other apps). */
  private fun appName(packageName: String): String = try {
    val packageManager = activity.packageManager
    packageManager.getApplicationLabel(packageManager.getApplicationInfo(packageName, 0)).toString()
  } catch (e: Exception) {
    packageName
  }
}
