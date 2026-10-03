package io.github.olegg90.pswmanager

import android.app.Activity
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
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

@InvokeArg
class StoreArgs {
  /** The database's key (what Rust makes of it), sealed here. */
  lateinit var secret: String
  lateinit var title: String
}

@InvokeArg
class PromptArgs {
  lateinit var title: String
}

/**
 * Biometric unlock (`docs/spec-android.md`, *Unlock*): the database's key is
 * encrypted with an AES key in the Android Keystore that only a strong
 * biometric (fingerprint or face) unlocks, for each use, and that a newly
 * enrolled fingerprint or face makes unusable for good. The sealed key is kept
 * in the app's private preferences.
 *
 * Errors told apart (by Rust): `biometric:cancelled` (the user chose the
 * master password), `biometric:invalidated` (the Keystore key is gone; the
 * sealed key is deleted with it), `biometric:none` (nothing is stored).
 */
@TauriPlugin
class BiometricPlugin(private val activity: Activity) : Plugin(activity) {
  private val prefs get() = activity.getSharedPreferences("biometric", Context.MODE_PRIVATE)

  /** Whether this phone can (a strong biometric is enrolled) and whether a key is stored. */
  @Command
  fun status(invoke: Invoke) {
    val available = BiometricManager.from(activity).canAuthenticate(BIOMETRIC_STRONG) == BiometricManager.BIOMETRIC_SUCCESS
    invoke.resolve(JSObject().put("available", available).put("stored", prefs.contains(SEALED)))
  }

  /** Seals `secret` after a fingerprint or face confirms it. */
  @Command
  fun store(invoke: Invoke) {
    val args = invoke.parseArgs(StoreArgs::class.java)
    val cipher = try {
      try {
        encrypting()
      } catch (e: KeyPermanentlyInvalidatedException) {
        forgetKey()
        encrypting()
      }
    } catch (e: Exception) {
      invoke.reject(e.message ?: e.toString())
      return
    }
    prompt(invoke, args.title, cipher) { ready ->
      val sealed = ready.iv + ready.doFinal(args.secret.toByteArray(Charsets.UTF_8))
      if (!prefs.edit().putString(SEALED, Base64.encodeToString(sealed, Base64.NO_WRAP)).commit()) {
        throw Exception("Cannot keep the key")
      }
      JSObject()
    }
  }

  /** The sealed secret, after a fingerprint or face. */
  @Command
  fun retrieve(invoke: Invoke) {
    val title = invoke.parseArgs(PromptArgs::class.java).title
    val stored = prefs.getString(SEALED, null) ?: return invoke.reject("biometric:none")
    val sealed = Base64.decode(stored, Base64.NO_WRAP)
    val cipher = try {
      val key = existingKey() ?: throw KeyPermanentlyInvalidatedException()
      Cipher.getInstance(CIPHER).apply { init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, sealed, 0, IV_SIZE)) }
    } catch (e: KeyPermanentlyInvalidatedException) {
      forgetKey()
      return invoke.reject("biometric:invalidated")
    } catch (e: Exception) {
      return invoke.reject(e.message ?: e.toString())
    }
    prompt(invoke, title, cipher) { ready ->
      JSObject().put("secret", String(ready.doFinal(sealed, IV_SIZE, sealed.size - IV_SIZE), Charsets.UTF_8))
    }
  }

  /** Deletes the sealed key and the Keystore key. */
  @Command
  fun forget(invoke: Invoke) {
    try {
      forgetKey()
      invoke.resolve(JSObject())
    } catch (e: Exception) {
      invoke.reject(e.message ?: e.toString())
    }
  }

  private fun prompt(invoke: Invoke, title: String, cipher: Cipher, done: (Cipher) -> JSObject) {
    activity.runOnUiThread {
      val callback = object : BiometricPrompt.AuthenticationCallback() {
        override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
          try {
            invoke.resolve(done(result.cryptoObject!!.cipher!!))
          } catch (e: Exception) {
            invoke.reject(e.message ?: e.toString())
          }
        }

        override fun onAuthenticationError(code: Int, message: CharSequence) {
          val cancelled = code == BiometricPrompt.ERROR_NEGATIVE_BUTTON || code == BiometricPrompt.ERROR_USER_CANCELED ||
            code == BiometricPrompt.ERROR_CANCELED
          invoke.reject(if (cancelled) "biometric:cancelled" else message.toString())
        }
      }
      val info = BiometricPrompt.PromptInfo.Builder()
        .setTitle(title)
        .setNegativeButtonText("Use the master password")
        .setAllowedAuthenticators(BIOMETRIC_STRONG)
        .build()
      BiometricPrompt(activity as FragmentActivity, ContextCompat.getMainExecutor(activity), callback)
        .authenticate(info, BiometricPrompt.CryptoObject(cipher))
    }
  }

  private fun keyStore() = KeyStore.getInstance(KEY_STORE).apply { load(null) }

  private fun existingKey(): SecretKey? = keyStore().getKey(ALIAS, null) as SecretKey?

  private fun encrypting(): Cipher {
    val key = existingKey() ?: newKey()
    return Cipher.getInstance(CIPHER).apply { init(Cipher.ENCRYPT_MODE, key) }
  }

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

  private fun forgetKey() {
    prefs.edit().remove(SEALED).commit()
    keyStore().deleteEntry(ALIAS)
  }

  private companion object {
    const val KEY_STORE = "AndroidKeyStore"
    const val ALIAS = "pswmanager-biometric"
    const val CIPHER = "AES/GCM/NoPadding"
    const val IV_SIZE = 12
    const val SEALED = "databaseKey"
  }
}
