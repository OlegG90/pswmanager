package io.github.olegg90.pswmanager

import android.os.Bundle
import android.util.Log
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import androidx.credentials.CreatePasswordRequest
import androidx.credentials.CredentialManager
import androidx.credentials.GetCredentialRequest
import androidx.credentials.GetPasswordOption
import androidx.credentials.PasswordCredential
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.launch

/**
 * TEMP (#164), removed before the PR: an app asking for a password through
 * Credential Manager, to try PswManager as the provider. Started with
 * `adb shell am start -n io.github.olegg90.pswmanager/.CredTestActivity --es mode save|get`.
 * Logs (tag PswmTest) the user name and the password's length only.
 */
class CredTestActivity : AppCompatActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    val out = TextView(this).apply { setPadding(48, 160, 48, 48); textSize = 18f }
    setContentView(out)
    val mode = intent.getStringExtra("mode") ?: "get"
    out.text = "Asking ($mode)…"
    lifecycleScope.launch {
      val text = try {
        val manager = CredentialManager.create(this@CredTestActivity)
        if (mode == "save") {
          manager.createCredential(this@CredTestActivity, CreatePasswordRequest("pswm-test-user", "pswm-test-pass-" + System.currentTimeMillis() % 1000))
          "saved"
        } else {
          val credential = manager.getCredential(this@CredTestActivity, GetCredentialRequest(listOf(GetPasswordOption()))).credential
          if (credential is PasswordCredential) "got ${credential.id}, password of ${credential.password.length} chars" else "got ${credential.type}"
        }
      } catch (e: Exception) {
        "${e.javaClass.simpleName}: ${e.message}"
      }
      out.text = text
      Log.i("PswmTest", text)
    }
  }
}
