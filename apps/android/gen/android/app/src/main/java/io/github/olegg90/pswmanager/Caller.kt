package io.github.olegg90.pswmanager

import android.content.Context
import android.content.pm.PackageManager
import androidx.annotation.RequiresApi
import androidx.appcompat.app.AlertDialog
import androidx.biometric.BiometricManager.Authenticators.BIOMETRIC_STRONG
import androidx.biometric.BiometricManager.Authenticators.DEVICE_CREDENTIAL
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.credentials.provider.CallingAppInfo
import androidx.fragment.app.FragmentActivity
import org.json.JSONObject
import java.security.MessageDigest

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
      // The app's current signing certificate (the last after a rotation).
      val signing = app.signingInfoCompat
      val certificate = (if (signing.hasMultipleSigners) signing.apkContentsSigners.firstOrNull() else signing.signingCertificateHistory.lastOrNull())?.toByteArray()
      return Caller(origin, app.packageName, certificate)
    }

    /**
     * Who an autofill request is for (#236): the page's web domain when a
     * browser on Google's list of privileged apps shows it over https (any app
     * can put a domain in its views, so only those speak for a site), else the
     * app by its package and signing certificate. Such a browser speaks for a
     * page only: one over plain http (or with no scheme said), its own screens
     * and a page without a domain are no one's, so nothing is filled or saved
     * there, rather than as the browser app's. Null when there is no one.
     */
    fun forAutofill(context: Context, fields: AutofillFields): Caller? {
      val certificate = signingCertificate(context, fields.packageName) ?: return null
      if (!privilegedBrowser(context, fields.packageName, certificate)) return Caller(null, fields.packageName, certificate)
      val domain = fields.webDomain?.takeIf { fields.webScheme.equals("https", ignoreCase = true) } ?: return null
      return Caller("https://$domain", fields.packageName, certificate)
    }

    /** An installed app's current signing certificate (the last after a rotation). */
    private fun signingCertificate(context: Context, packageName: String): ByteArray? = try {
      val signing = context.packageManager.getPackageInfo(packageName, PackageManager.GET_SIGNING_CERTIFICATES).signingInfo ?: return null
      (if (signing.hasMultipleSigners()) signing.apkContentsSigners.firstOrNull() else signing.signingCertificateHistory.lastOrNull())?.toByteArray()
    } catch (e: PackageManager.NameNotFoundException) {
      android.util.Log.w("PswmAutofill", "Cannot see the app asking ($packageName)")
      null
    }

    /** Whether the app is a browser on Google's list, signed with one of the certificates listed for it. */
    private fun privilegedBrowser(context: Context, packageName: String, certificate: ByteArray): Boolean {
      val fingerprint = MessageDigest.getInstance("SHA-256").digest(certificate).joinToString(":") { "%02X".format(it) }
      val apps = try {
        JSONObject(ProviderBridge.allowlist(context)).getJSONArray("apps")
      } catch (e: Exception) {
        return false
      }
      return (0 until apps.length()).map { apps.getJSONObject(it).optJSONObject("info") }.any { info ->
        val signatures = info?.optJSONArray("signatures")
        info?.optString("package_name") == packageName && signatures != null &&
          (0 until signatures.length()).any { signatures.getJSONObject(it).optString("cert_fingerprint_sha256").equals(fingerprint, ignoreCase = true) }
      }
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