package io.github.olegg90.pswmanager

import android.net.Uri
import android.view.Gravity
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import kotlin.concurrent.thread

/**
 * Saving a login another app offers, in `activity` once the database is
 * unlocked (#164, #270): who asks and where it goes (a new entry for the app
 * or site, one of its entries, whose password it replaces, the old version
 * going to history, or any other entry, which is linked to the app or site
 * first, as Keepass2Android does), verified with the fingerprint or the
 * screen lock. It goes up with the next sync. An app's entry keeps its
 * signing certificate; one for its package that keeps none yet is changed,
 * and another entry linked to the app, only once the user confirmed the app
 * ([confirmApp]).
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

  /**
   * Shows where the login goes: a new entry, one of the app's or site's (one
   * with the user name typed in first; for one with another user name only the
   * password is replaced), or another entry ([pickEntry]).
   */
  fun choose() {
    // Off the main thread: a sync or a save may hold the database a moment.
    thread {
      val answer = Answer.of(ProviderBridge.loginChoices(ProviderBridge.state(activity), caller.origin, caller.packageName, caller.certificate))
      activity.runOnUiThread { if (!activity.isDestroyed) show(answer) }
    }
  }

  /** The choices [choose] read (or why there are none), as buttons. */
  private fun show(answer: Answer) {
    if (answer !is Answer.Ok) return failed(answer.why)
    val choices = answer.json
    cloud = choices.getBoolean("cloud")
    val entries = choices.getJSONArray("entries").let { all -> (0 until all.length()).map { all.getJSONObject(it) } }
      .sortedBy { it.optString("username") != username }
    val pad = (24 * activity.resources.displayMetrics.density).toInt()
    val list = LinearLayout(activity).apply {
      orientation = LinearLayout.VERTICAL
      gravity = Gravity.CENTER_VERTICAL
      setPadding(pad, pad, pad, pad)
      addView(TextView(activity).apply { text = "Save the password for ${username.ifEmpty { caller.label }}"; textSize = 22f })
      addView(TextView(activity).apply { text = "Asked by ${caller.label}." })
      addView(Button(activity).apply { text = "New entry"; setOnClickListener { save(null, "New entry") } })
      for (entry in entries) {
        val label = entryLabel(entry)
        val kept = entry.optString("username")
        val action = when {
          kept.isNotEmpty() && kept != username -> "Replace the password of $label (its user name stays)"
          kept.isNotEmpty() && username.isNotEmpty() -> "Update $label (same user name)"
          else -> "Update $label"
        }
        addView(Button(activity).apply { text = action; isAllCaps = false; setOnClickListener { save(entry.getString("id"), label) } })
      }
      addView(Button(activity).apply { text = "Another entry…"; setOnClickListener { pickEntry(activity, "Save into which entry?", ::savePicked, ::choose) } })
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

  /**
   * Saves into an entry picked among all, once the user is verified and the
   * entry linked to who asks ([linkEntry]).
   */
  private fun savePicked(id: String, entry: String) {
    if (busy) return
    if (password.isEmpty()) return failed("There is no password to save")
    busy = true
    verifyUser(activity, "Save the password", entry, { linkEntry(activity, id, entry, caller, { store(id, entry, true) }, cancelled, failed) }, { busy = false; cancelled() }, failed)
  }

  /** Saves, the user verified; asks to confirm the app first when the entry needs it. */
  private fun store(id: String?, entry: String, confirmed: Boolean) {
    val title = caller.origin?.let { Uri.parse(it).host } ?: appName(caller.packageName)
    thread {
      val answer = Answer.of(ProviderBridge.saveLogin(id, caller.origin, caller.packageName, caller.certificate, title, username, password, confirmed))
      if (answer is Answer.Ok) UploadWorker.schedule(activity.applicationContext, ProviderBridge.state(activity), cloud)
      activity.runOnUiThread {
        when (answer) {
          is Answer.Ok -> saved()
          is Answer.Unconfirmed -> confirmApp(activity, caller.packageName, entry, answer.certificate, { store(id, entry, true) }, cancelled)
          is Answer.Failed -> failed(answer.why)
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
