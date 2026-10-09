package io.github.olegg90.pswmanager

import android.app.Activity
import android.view.WindowManager

/**
 * Whether the app's screens may be captured (Settings, *Allow screenshots*,
 * #192): off by default, so screenshots and screen recording show nothing
 * and the recent-apps preview is blank (`FLAG_SECURE`). The setting is
 * Rust's, read through [ProviderBridge] as every screen opens: the credential
 * provider's and the export's too, which may run without the app.
 */
object Screenshots {
  /** Lets `activity` be captured or not, as the setting is now. */
  fun apply(activity: Activity) {
    val allowed = ProviderBridge.screenshotsAllowed(ProviderBridge.state(activity))
    activity.window.setFlags(if (allowed) 0 else WindowManager.LayoutParams.FLAG_SECURE, WindowManager.LayoutParams.FLAG_SECURE)
  }
}
