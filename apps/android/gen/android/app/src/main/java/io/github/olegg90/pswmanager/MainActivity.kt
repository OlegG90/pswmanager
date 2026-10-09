package io.github.olegg90.pswmanager

import android.os.Bundle
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    // Not captured unless the settings allow it: screenshots and screen
    // recording show nothing, and the recent-apps thumbnail is blank.
    Screenshots.apply(this)
    super.onCreate(savedInstanceState)
  }
}
