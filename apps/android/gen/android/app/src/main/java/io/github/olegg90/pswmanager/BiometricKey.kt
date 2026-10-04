package io.github.olegg90.pswmanager

import android.content.Context
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import android.util.Base64
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricManager.Authenticators.BIOMETRIC_STRONG
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * The database's key sealed for biometric unlock (`docs/spec-android.md`,
 * *Unlock*): encrypted with an AES key in the Android Keystore that only a
 * strong biometric (fingerprint or face) unlocks, for each use, and that a
 * newly enrolled fingerprint or face makes unusable for good. The sealed key
 * is kept in the app's private preferences. Used by the app
 * ([BiometricPlugin]) and by the credential provider's unlock
 * ([UnlockActivity]).
 */
object BiometricKey {
  /** The Keystore key is gone (a new fingerprint or face): the sealed key was deleted with it. */
  class Invalidated : Exception("biometric:invalidated")

  /** This phone has a strong biometric enrolled. */
  fun available(context: Context): Boolean =
    BiometricManager.from(context).canAuthenticate(BIOMETRIC_STRONG) == BiometricManager.BIOMETRIC_SUCCESS

  fun isStored(context: Context): Boolean = prefs(context).contains(SEALED)

  /** A cipher that seals, once a biometric confirms it ([prompt]). */
  fun encrypting(context: Context): Cipher = try {
    encryptingWith(existingKey() ?: newKey())
  } catch (e: KeyPermanentlyInvalidatedException) {
    forget(context)
    encryptingWith(newKey())
  }

  /** Keeps `secret`, sealed with `cipher` (from [encrypting], confirmed). */
  fun seal(context: Context, cipher: Cipher, secret: String) {
    val sealed = cipher.iv + cipher.doFinal(secret.toByteArray(Charsets.UTF_8))
    if (!prefs(context).edit().putString(SEALED, Base64.encodeToString(sealed, Base64.NO_WRAP)).commit()) {
      throw Exception("Cannot keep the key")
    }
  }

  /** A cipher that opens the sealed key once a biometric confirms it; null when none is stored. */
  fun decrypting(context: Context): Cipher? {
    val sealed = sealed(context) ?: return null
    return try {
      val key = existingKey() ?: throw KeyPermanentlyInvalidatedException()
      Cipher.getInstance(CIPHER).apply { init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, sealed, 0, IV_SIZE)) }
    } catch (e: KeyPermanentlyInvalidatedException) {
      forget(context)
      throw Invalidated()
    }
  }

  /** The sealed key, opened with `cipher` (from [decrypting], confirmed). */
  fun open(context: Context, cipher: Cipher): String {
    val sealed = sealed(context) ?: throw Exception("biometric:none")
    return String(cipher.doFinal(sealed, IV_SIZE, sealed.size - IV_SIZE), Charsets.UTF_8)
  }

  /** Deletes the sealed key and the Keystore key. */
  fun forget(context: Context) {
    prefs(context).edit().remove(SEALED).commit()
    keyStore().deleteEntry(ALIAS)
  }

  /**
   * Asks for a strong biometric to use `cipher`; `cancelled` when the user
   * chose the master password instead.
   */
  fun prompt(
    activity: FragmentActivity,
    title: String,
    cipher: Cipher,
    succeeded: (Cipher) -> Unit,
    failed: (cancelled: Boolean, message: String) -> Unit,
  ) {
    activity.runOnUiThread {
      val callback = object : BiometricPrompt.AuthenticationCallback() {
        override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
          succeeded(result.cryptoObject!!.cipher!!)
        }

        override fun onAuthenticationError(code: Int, message: CharSequence) {
          val cancelled = code == BiometricPrompt.ERROR_NEGATIVE_BUTTON || code == BiometricPrompt.ERROR_USER_CANCELED ||
            code == BiometricPrompt.ERROR_CANCELED
          failed(cancelled, message.toString())
        }
      }
      val info = BiometricPrompt.PromptInfo.Builder()
        .setTitle(title)
        .setNegativeButtonText("Use the master password")
        .setAllowedAuthenticators(BIOMETRIC_STRONG)
        .build()
      BiometricPrompt(activity, ContextCompat.getMainExecutor(activity), callback).authenticate(info, BiometricPrompt.CryptoObject(cipher))
    }
  }

  private fun prefs(context: Context) = context.getSharedPreferences("biometric", Context.MODE_PRIVATE)

  private fun sealed(context: Context): ByteArray? = prefs(context).getString(SEALED, null)?.let { Base64.decode(it, Base64.NO_WRAP) }

  private fun keyStore() = KeyStore.getInstance(KEY_STORE).apply { load(null) }

  private fun existingKey(): SecretKey? = keyStore().getKey(ALIAS, null) as SecretKey?

  private fun encryptingWith(key: SecretKey): Cipher = Cipher.getInstance(CIPHER).apply { init(Cipher.ENCRYPT_MODE, key) }

  private fun newKey(): SecretKey {
    val spec = KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
      .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
      .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
      .setUserAuthenticationRequired(true)
      .setInvalidatedByBiometricEnrollment(true)
      .apply {
        // Every use asks for a strong biometric.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
          setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG)
        }
      }
      .build()
    return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEY_STORE).apply { init(spec) }.generateKey()
  }

  private const val KEY_STORE = "AndroidKeyStore"
  private const val ALIAS = "pswmanager-biometric"
  private const val CIPHER = "AES/GCM/NoPadding"
  private const val IV_SIZE = 12
  private const val SEALED = "databaseKey"
}
