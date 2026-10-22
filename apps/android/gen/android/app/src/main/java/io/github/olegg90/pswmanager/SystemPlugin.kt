package io.github.olegg90.pswmanager

import android.app.Activity
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.net.Uri
import android.provider.Settings
import android.view.autofill.AutofillManager
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class UploadArgs {
  /** The app's state file. */
  lateinit var state: String
  /** The database syncs with a cloud store: the upload waits for a network. */
  var cloud: Boolean = true
}

/**
 * What Android tells the page. One plugin: Tauri keeps one Kotlin plugin per
 * Rust plugin, so a second one registered under the same name replaces it.
 *
 * - Back (the button or the gesture) goes to the page first: `window.pswmBack()`
 *   closes a panel or returns to the previous screen and answers true. When the
 *   page has nothing to close, Back does what it does anyway (leaves the app).
 *   Added after the webview's own handler, so it is asked first.
 * - The screen turning off: `window.pswmScreenOff()`, which locks.
 *
 * And what the app asks of Android: the background upload ([UploadWorker]),
 * whether its screens may be captured ([Screenshots]), and the phone's
 * autofill service ([PswmAutofillService]).
 */
@TauriPlugin
class SystemPlugin(private val activity: Activity) : Plugin(activity) {
  override fun load(webView: WebView) {
    val app = activity as AppCompatActivity
    val back = object : OnBackPressedCallback(true) {
      override fun handleOnBackPressed() {
        webView.evaluateJavascript("window.pswmBack ? window.pswmBack() : false") { handled ->
          if (handled != "true") {
            isEnabled = false
            app.onBackPressedDispatcher.onBackPressed()
            isEnabled = true
          }
        }
      }
    }
    app.onBackPressedDispatcher.addCallback(app, back)

    val screenOff = object : BroadcastReceiver() {
      override fun onReceive(context: Context, intent: Intent) {
        webView.post { webView.evaluateJavascript("window.pswmScreenOff && window.pswmScreenOff()", null) }
      }
    }
    ContextCompat.registerReceiver(activity, screenOff, IntentFilter(Intent.ACTION_SCREEN_OFF), ContextCompat.RECEIVER_NOT_EXPORTED)
  }

  /** The *Allow screenshots* setting changed: the app's window follows at once, the other screens as they open. */
  @Command
  fun applyScreenshots(invoke: Invoke) {
    activity.runOnUiThread { Screenshots.apply(activity) }
    invoke.resolve(JSObject())
  }

  /** Whether the phone can autofill, and whether PswManager is its autofill service (#236). */
  @Command
  fun autofillStatus(invoke: Invoke) {
    val manager = activity.getSystemService(AutofillManager::class.java)
    val status = JSObject()
    status.put("supported", manager?.isAutofillSupported == true)
    // True only when PswManager's own service is the one chosen.
    status.put("enabled", manager?.hasEnabledAutofillServices() == true)
    invoke.resolve(status)
  }

  /** Android's choice of autofill service, with PswManager offered (the user decides there). */
  @Command
  fun chooseAutofill(invoke: Invoke) {
    try {
      activity.startActivity(Intent(Settings.ACTION_REQUEST_SET_AUTOFILL_SERVICE, Uri.parse("package:${activity.packageName}")))
      invoke.resolve(JSObject())
    } catch (e: Exception) {
      invoke.reject(e.message ?: e.toString())
    }
  }

  /** Changes are waiting: they go up in the background if the app cannot send them first. */
  @Command
  fun scheduleUpload(invoke: Invoke) {
    try {
      val args = invoke.parseArgs(UploadArgs::class.java)
      UploadWorker.schedule(activity.applicationContext, args.state, args.cloud)
      invoke.resolve(JSObject())
    } catch (e: Exception) {
      invoke.reject(e.message ?: e.toString())
    }
  }
}
