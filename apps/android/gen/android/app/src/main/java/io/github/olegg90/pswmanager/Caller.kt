package io.github.olegg90.pswmanager

import android.content.Context
import android.content.pm.PackageManager
import android.content.pm.Signature
import androidx.annotation.RequiresApi
import androidx.appcompat.app.AlertDialog
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
  /** The client data hash the request gives: a browser's only (an app's client data is made in Rust). */
  fun clientDataHash(given: ByteArray?): ByteArray? = given.takeIf { origin != null }

  /** Who asks, for the user: the site a browser speaks for, or the app. */
  val label: String get() = origin ?: packageName

  companion object {
    /** Throws when the list of privileged browsers cannot be read: who asks is then unknown. */
    @RequiresApi(34)
    fun of(context: Context, app: CallingAppInfo): Caller {
      val origin = app.getOrigin(ProviderBridge.allowlist(context))
      val signing = app.signingInfoCompat
      val certificate = current(signing.hasMultipleSigners, signing.apkContentsSigners.toList(), signing.signingCertificateHistory.toList())
      return Caller(origin, app.packageName, certificate)
    }

    /**
     * Who an autofill request is for (#236): the page's site when a privileged
     * browser shows it over https, the app itself, or no one (null: nothing is
     * filled or saved there). Rust decides ([ProviderBridge.autofillAsker]);
     * this gathers what it decides on.
     */
    fun forAutofill(context: Context, fields: AutofillFields): Caller? {
      val certificate = signingCertificate(context, fields.packageName) ?: return null
      val asker = Answer.of(ProviderBridge.autofillAsker(ProviderBridge.allowlist(context), fields.packageName, certificate, fields.webDomain, fields.webScheme))
      return (asker as? Answer.Ok)?.let { Caller(it.text, fields.packageName, certificate) }
    }

    /** An installed app's current signing certificate. */
    private fun signingCertificate(context: Context, packageName: String): ByteArray? = try {
      val signing = context.packageManager.getPackageInfo(packageName, PackageManager.GET_SIGNING_CERTIFICATES).signingInfo ?: return null
      current(signing.hasMultipleSigners(), signing.apkContentsSigners.orEmpty().toList(), signing.signingCertificateHistory.orEmpty().toList())
    } catch (e: PackageManager.NameNotFoundException) {
      android.util.Log.w("PswmAutofill", "Cannot see the app asking ($packageName)")
      null
    }

    /** An app's current signing certificate: the first of several signers, else the last after a rotation. */
    private fun current(multipleSigners: Boolean, signers: List<Signature>, history: List<Signature>): ByteArray? =
      (if (multipleSigners) signers.firstOrNull() else history.lastOrNull())?.toByteArray()
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

/**
 * Asks the user to confirm the app before a login (`entry` names it) is used or changed for it,
 * when the entry is for the app's package but keeps no certificate (made in Keepass2Android, say).
 * Any app can take a package name; only its signing certificate tells the real one apart, so it is
 * shown, and the entry keeps it once confirmed: an app signed otherwise does not get the login then.
 * The same when the user picked an entry to link to the app (`linking`, #270).
 */
fun confirmApp(activity: FragmentActivity, packageName: String, entry: String, certificate: String, confirmed: () -> Unit, cancelled: () -> Unit, linking: Boolean = false) {
  val login = entry.ifEmpty { "This login" }
  val why = if (linking) "$login is to be linked to $packageName" else "$login is for $packageName, but PswManager does not know this app's signing certificate yet"
  AlertDialog.Builder(activity)
    .setTitle("Trust this app?")
    .setMessage(
      "$why. " +
        "Any app can take that name; only the certificate tells them apart:\n\n$certificate\n\n" +
        "Go on only if this is the app you meant, from a store you trust. The entry then keeps this certificate, and an app signed otherwise will not get or change this login.",
    )
    .setPositiveButton("Trust") { _, _ -> confirmed() }
    .setNegativeButton("Cancel") { _, _ -> cancelled() }
    .setOnCancelListener { cancelled() }
    .show()
}