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
import androidx.work.ExistingWorkPolicy
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.Worker
import androidx.work.WorkerParameters
import androidx.work.workDataOf
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.util.UUID
import java.util.concurrent.TimeUnit

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
 *
 * The clearing is a timer in the app, which is lost when Android stops the
 * app (swiped away from the recent apps, say), so [ClipboardClearWorker] is
 * asked to clear the copy as well; the timer cancels it once it has decided.
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
      if (now != copied) {
        generation++
        ClipboardClearWorker.cancel(activity.applicationContext)
      }
    }
  }

  @Command
  fun copy(invoke: Invoke) {
    val args = invoke.parseArgs(CopyArgs::class.java)
    main.post {
      // Marks the copy for the clearing that outlives the app, which keeps this instead of the text.
      val token = UUID.randomUUID().toString()
      val clip = ClipData.newPlainText("PswManager", args.text)
      clip.description.extras = PersistableBundle().apply {
        if (Build.VERSION.SDK_INT >= 33) putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true)
        else putBoolean("android.content.extra.IS_SENSITIVE", true)
        putString(ClipboardClearWorker.TOKEN, token)
      }
      copied = args.text
      clipboard.setPrimaryClip(clip)
      val mine = ++generation
      ClipboardClearWorker.schedule(activity.applicationContext, token, args.clearAfterMs)
      main.postDelayed({
        // Only the latest copy's timer cancels the work: an older one would cancel the newer copy's.
        if (generation == mine) {
          clipboard.clearPrimaryClip()
          copied = null
          ClipboardClearWorker.cancel(activity.applicationContext)
        }
      }, args.clearAfterMs)
      invoke.resolve(JSObject())
    }
  }
}

/**
 * Clears a copied secret when Android stopped the app before its own timer
 * ran: WorkManager keeps the request across that. It holds no secret, only the
 * random token the copy carries in its description. An app in front may read
 * the clipboard, and then only that copy is cleared; an app in the background
 * cannot (Android 10+), and the clipboard is cleared all the same, as the
 * app's own timer does then.
 */
class ClipboardClearWorker(context: Context, params: WorkerParameters) : Worker(context, params) {
  override fun doWork(): Result {
    val token = inputData.getString(TOKEN) ?: return Result.failure()
    val clipboard = applicationContext.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    // Null when the clipboard is empty or the app may not read it.
    val description = clipboard.primaryClipDescription
    if (description == null || description.extras?.getString(TOKEN) == token) clipboard.clearPrimaryClip()
    return Result.success()
  }

  companion object {
    /** The copy's token: in the clip's description and in the work's input. */
    const val TOKEN = "io.github.olegg90.pswmanager.CLIP_TOKEN"
    private const val NAME = "clipboard-clear"
    /** Past the app's own clearing, so that goes first while the app runs and cancels this. */
    private const val LATER_MS = 5_000L

    /** Asks for the copy marked `token` to be cleared after `clearAfterMs`; it
     *  replaces the clearing asked for an earlier copy. */
    fun schedule(context: Context, token: String, clearAfterMs: Long) {
      val request = OneTimeWorkRequestBuilder<ClipboardClearWorker>()
        .setInputData(workDataOf(TOKEN to token))
        .setInitialDelay(clearAfterMs + LATER_MS, TimeUnit.MILLISECONDS)
        .build()
      WorkManager.getInstance(context).enqueueUniqueWork(NAME, ExistingWorkPolicy.REPLACE, request)
    }

    /** The app's own clearing has decided (cleared, or saw something else copied). */
    fun cancel(context: Context) {
      WorkManager.getInstance(context).cancelUniqueWork(NAME)
    }
  }
}
