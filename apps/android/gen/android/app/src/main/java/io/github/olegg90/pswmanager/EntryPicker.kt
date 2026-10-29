package io.github.olegg90.pswmanager

import android.text.Editable
import android.text.InputType
import android.text.TextWatcher
import android.view.Gravity
import android.view.View
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import org.json.JSONArray
import org.json.JSONObject
import kotlin.concurrent.thread

/** An entry as a choice shows it: its title and user name. */
fun entryLabel(entry: JSONObject): String = entryLabel(entry.optString("title"), entry.optString("username"))

/** An entry titled `title` for `username`, as a choice shows it. */
fun entryLabel(title: String, username: String): String =
  listOf(title, username).filter { it.isNotEmpty() }.joinToString(" · ")

/** How long the search waits after a keystroke for the next one. */
private const val SEARCH_DELAY_MS = 250L

/**
 * Lets the user pick any entry in use for a login no entry is for yet
 * (#270), as Keepass2Android does: a search over titles, user names and URLs,
 * the matches (those with a password first) to tap. Shown in `activity`, the
 * database unlocked.
 */
fun pickEntry(activity: AppCompatActivity, heading: String, picked: (id: String, label: String) -> Unit, cancelled: () -> Unit) {
  keepOutOfAutofill(activity)
  val pad = (24 * activity.resources.displayMetrics.density).toInt()
  val results = LinearLayout(activity).apply { orientation = LinearLayout.VERTICAL }
  // One pick: a second tap meanwhile would link and fill twice.
  var chosen = false
  fun list(found: JSONArray) {
    results.removeAllViews()
    if (found.length() == 0) results.addView(TextView(activity).apply { text = "No entry holds this." })
    for (i in 0 until found.length()) {
      val entry = found.getJSONObject(i)
      val label = entryLabel(entry).ifEmpty { "Untitled entry" }
      results.addView(Button(activity).apply { text = label; isAllCaps = false; setOnClickListener { if (!chosen) { chosen = true; picked(entry.getString("id"), label) } } })
    }
  }
  // The latest query: an earlier search that ends after it does not replace its matches.
  var latest = 0
  fun show(query: String) {
    val ticket = ++latest
    // Off the main thread, as a sync or a save may hold the database a moment;
    // a moment after the last keystroke, so typing does not queue a search per letter.
    results.postDelayed({
      if (ticket != latest) return@postDelayed
      thread {
        val found = JSONArray(ProviderBridge.searchLogins(query) ?: "[]")
        activity.runOnUiThread { if (ticket == latest && !activity.isDestroyed) list(found) }
      }
    }, if (query.isEmpty()) 0L else SEARCH_DELAY_MS)
  }
  val search = EditText(activity).apply {
    hint = "Search title, user name or URL"
    inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
    addTextChangedListener(object : TextWatcher {
      override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
      override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {}
      override fun afterTextChanged(s: Editable?) = show(s.toString())
    })
  }
  activity.setContentView(LinearLayout(activity).apply {
    orientation = LinearLayout.VERTICAL
    gravity = Gravity.TOP
    setPadding(pad, pad, pad, pad)
    addView(TextView(activity).apply { text = heading; textSize = 22f })
    addView(search)
    addView(ScrollView(activity).apply { addView(results) }, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f))
    addView(Button(activity).apply { text = "Cancel"; setOnClickListener { cancelled() } })
  })
  show("")
}

/**
 * Keeps the fields of a screen PswManager shows for another app out of
 * autofill: typing in them would start an autofill session of its own on top
 * of the one that opened the screen, and the other app's fields would not get
 * what the user picked.
 */
fun keepOutOfAutofill(activity: AppCompatActivity) {
  activity.window.decorView.importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS
}

/**
 * Links entry `id` (`label` names it), which the user picked, to who asks
 * ([ProviderBridge.linkLogin]): an app only once the user confirmed it
 * ([confirmApp]), as linking hands the entry to it.
 */
fun linkEntry(activity: AppCompatActivity, id: String, label: String, caller: Caller, linked: () -> Unit, cancelled: () -> Unit, failed: (String) -> Unit) {
  fun link(confirmed: Boolean) {
    thread {
      val answer = Answer.of(ProviderBridge.linkLogin(id, caller.origin, caller.packageName, caller.certificate, confirmed))
      activity.runOnUiThread {
        when (answer) {
          is Answer.Ok -> linked()
          is Answer.Unconfirmed -> confirmApp(activity, caller.packageName, label, answer.certificate, { link(true) }, cancelled, linking = true)
          is Answer.Failed -> failed(answer.why)
        }
      }
    }
  }
  link(false)
}
