package io.github.olegg90.pswmanager

import android.app.Activity
import android.content.Context
import android.view.WindowManager

/**
 * Whether the app's screens may be captured (Settings, *Allow screenshots*,
 * #192): off by default, so screenshots and screen recording show nothing
 * and the recent-apps preview is blank (`FLAG_SECURE`). Rust keeps the
 * setting; this copy is what every screen reads as it opens, the credential
 * provider's and the export's too, which may run without the app.
 */
object Screenshots {
  private const val PREFS = "screenshots"
  private const val ALLOWED = "allowed"

  private fun prefs(context: Context) = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

  fun allowed(context: Context): Boolean = prefs(context).getBoolean(ALLOWED, false)

  /** The setting changed in the app (SystemPlugin.kt). */
  fun set(context: Context, allowed: Boolean) = prefs(context).edit().putBoolean(ALLOWED, allowed).apply()

  /** Lets `activity` be captured or not, as the setting is. */
  fun apply(activity: Activity) {
    if (allowed(activity)) {
      activity.window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
    } else {
      activity.window.setFlags(WindowManager.LayoutParams.FLAG_SECURE, WindowManager.LayoutParams.FLAG_SECURE)
    }
  }
}
