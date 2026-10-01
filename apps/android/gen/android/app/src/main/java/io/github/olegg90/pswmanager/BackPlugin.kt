package io.github.olegg90.pswmanager

import android.app.Activity
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback
import androidx.appcompat.app.AppCompatActivity
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Plugin

/**
 * Back (the button or the gesture) goes to the page first: `window.pswmBack()`
 * closes a panel or returns to the previous screen and answers true. When the
 * page has nothing to close, Back does what it does anyway (leaves the app).
 * Added after the webview's own handler, so it is asked first.
 */
@TauriPlugin
class BackPlugin(private val activity: Activity) : Plugin(activity) {
  override fun load(webView: WebView) {
    val app = activity as AppCompatActivity
    val callback = object : OnBackPressedCallback(true) {
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
    app.onBackPressedDispatcher.addCallback(app, callback)
  }
}
