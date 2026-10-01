package io.github.olegg90.pswmanager

import android.app.Activity
import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.PersistableBundle
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class CopyArgs {
  lateinit var text: String
  /** Clear the clipboard after this long, if it still holds the text. */
  var clearAfterMs: Long = 20_000
}

/**
 * Copies a secret: marked sensitive, so the keyboard's clipboard suggestions
 * and the system's copy preview do not show it (Android 13+), and cleared
 * after a while unless something else was copied meanwhile. An app in the
 * background cannot read the clipboard, so "something else" is what this app
 * saw change since; a change it could not see is cleared all the same.
 */
@TauriPlugin
class ClipboardPlugin(private val activity: Activity) : Plugin(activity) {
  private val clipboard get() = activity.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
  private val main = Handler(Looper.getMainLooper())
  // Both are touched on the main thread only (the listener and the posts run there).
  /** Bumped by every copy and every change to something else, so only the latest copy's clearing runs. */
  private var generation = 0
  /** What this app copied last. */
  private var copied: String? = null

  override fun load(webView: android.webkit.WebView) {
    clipboard.addPrimaryClipChangedListener {
      val now = clipboard.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.text?.toString()
      if (now != copied) generation++
    }
  }

  @Command
  fun copy(invoke: Invoke) {
    val args = invoke.parseArgs(CopyArgs::class.java)
    main.post {
      val clip = ClipData.newPlainText("PswManager", args.text)
      clip.description.extras = PersistableBundle().apply {
        if (Build.VERSION.SDK_INT >= 33) putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true)
        else putBoolean("android.content.extra.IS_SENSITIVE", true)
      }
      copied = args.text
      clipboard.setPrimaryClip(clip)
      val mine = ++generation
      main.postDelayed({
        if (generation == mine) {
          clipboard.clearPrimaryClip()
          copied = null
        }
      }, args.clearAfterMs)
      invoke.resolve(JSObject())
    }
  }
}
