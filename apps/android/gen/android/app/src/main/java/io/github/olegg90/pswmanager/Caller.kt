package io.github.olegg90.pswmanager

import android.content.Context
import androidx.annotation.RequiresApi
import androidx.biometric.BiometricManager.Authenticators.BIOMETRIC_STRONG
import androidx.biometric.BiometricManager.Authenticators.DEVICE_CREDENTIAL
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.credentials.provider.CallingAppInfo
import androidx.fragment.app.FragmentActivity

/**
 * Who asks for a passkey: a browser on Google's list of privileged apps
 * speaks for the site (`origin`, with the client data hash it gives); any
 * other app signs for itself, with its package and current signing
 * certificate (the site checks it is its own app).
 */
class Caller(val origin: String?, val packageName: String, val certificate: ByteArray?) {
  companion object {
    /** Throws when the list of privileged browsers cannot be read: who asks is then unknown. */
    @RequiresApi(34)
    fun of(context: Context, app: CallingAppInfo): Caller {
      val origin = app.getOrigin(PswmCredentialService.allowlist(context))
      // The app's current signing certificate (the last after a rotation).
      val signing = app.signingInfoCompat
      val certificate = (if (signing.hasMultipleSigners) signing.apkContentsSigners.firstOrNull() else signing.signingCertificateHistory.lastOrNull())?.toByteArray()
      return Caller(origin, app.packageName, certificate)
    }
  }
}

/**
 * Verifies the user before a passkey is used or made: the fingerprint, or the
 * phone's screen lock (the system's prompt). `cancelled` when the user went
 * back; `failed` with why for anything else.
 */
fun verifyUser(activity: FragmentActivity, title: String, subtitle: String, verified: () -> Unit, cancelled: () -> Unit, failed: (String) -> Unit) {
  val callback = object : BiometricPrompt.AuthenticationCallback() {
    override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) = verified()

    override fun onAuthenticationError(code: Int, message: CharSequence) {
      if (code == BiometricPrompt.ERROR_USER_CANCELED || code == BiometricPrompt.ERROR_NEGATIVE_BUTTON || code == BiometricPrompt.ERROR_CANCELED) cancelled() else failed(message.toString())
    }
  }
  val info = BiometricPrompt.PromptInfo.Builder()
    .setTitle(title)
    .setSubtitle(subtitle)
    .setAllowedAuthenticators(BIOMETRIC_STRONG or DEVICE_CREDENTIAL)
    .build()
  BiometricPrompt(activity, ContextCompat.getMainExecutor(activity), callback).authenticate(info)
}