package io.github.olegg90.pswmanager

import android.app.Activity
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.webkit.WebView
import androidx.core.content.ContextCompat
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Plugin

/** The screen turning off tells the page (`window.pswmScreenOff()`), which locks. */
@TauriPlugin
class ScreenPlugin(private val activity: Activity) : Plugin(activity) {
  override fun load(webView: WebView) {
    val receiver = object : BroadcastReceiver() {
      override fun onReceive(context: Context, intent: Intent) {
        webView.post { webView.evaluateJavascript("window.pswmScreenOff && window.pswmScreenOff()", null) }
      }
    }
    ContextCompat.registerReceiver(activity, receiver, IntentFilter(Intent.ACTION_SCREEN_OFF), ContextCompat.RECEIVER_NOT_EXPORTED)
  }
}
