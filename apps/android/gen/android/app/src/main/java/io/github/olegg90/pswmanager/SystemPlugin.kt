package io.github.olegg90.pswmanager

import android.app.Activity
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Plugin

/**
 * What Android tells the page. One plugin: Tauri keeps one Kotlin plugin per
 * Rust plugin, so a second one registered under the same name replaces it.
 *
 * - Back (the button or the gesture) goes to the page first: `window.pswmBack()`
 *   closes a panel or returns to the previous screen and answers true. When the
 *   page has nothing to close, Back does what it does anyway (leaves the app).
 *   Added after the webview's own handler, so it is asked first.
 * - The screen turning off: `window.pswmScreenOff()`, which locks.
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
}
