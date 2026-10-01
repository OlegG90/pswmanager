package io.github.olegg90.pswmanager

import android.app.Activity
import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
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
class SecretName {
  lateinit var name: String
}

@InvokeArg
class SecretValue {
  lateinit var name: String
  lateinit var secret: String
}

/**
 * Secrets kept between runs (a store's refresh token, the icon cache's key):
 * encrypted with an AES key in the Android Keystore, which never leaves it,
 * and kept in the app's private preferences. The key needs no fingerprint, so
 * a sync in the background can use the tokens too.
 */
@TauriPlugin
class SecretsPlugin(private val activity: Activity) : Plugin(activity) {
  private val prefs get() = activity.getSharedPreferences("secrets", Context.MODE_PRIVATE)

  private fun key(): SecretKey {
    val keyStore = KeyStore.getInstance(KEY_STORE).apply { load(null) }
    (keyStore.getKey(ALIAS, null) as SecretKey?)?.let { return it }
    val spec = KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
      .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
      .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
      .build()
    return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEY_STORE).apply { init(spec) }.generateKey()
  }

  @Command
  fun write(invoke: Invoke) = answer(invoke) {
    val args = invoke.parseArgs(SecretValue::class.java)
    val cipher = Cipher.getInstance(CIPHER).apply { init(Cipher.ENCRYPT_MODE, key()) }
    val sealed = cipher.iv + cipher.doFinal(args.secret.toByteArray(Charsets.UTF_8))
    if (!prefs.edit().putString(args.name, Base64.encodeToString(sealed, Base64.NO_WRAP)).commit()) {
      throw Exception("Cannot keep the secret")
    }
    JSObject()
  }

  /** The secret, or `secret: null` when there is none (or it cannot be read any more). */
  @Command
  fun read(invoke: Invoke) = answer(invoke) {
    val name = invoke.parseArgs(SecretName::class.java).name
    val stored = prefs.getString(name, null)
    val secret = stored?.let {
      try {
        val sealed = Base64.decode(it, Base64.NO_WRAP)
        val cipher = Cipher.getInstance(CIPHER)
        cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, sealed, 0, IV_SIZE))
        String(cipher.doFinal(sealed, IV_SIZE, sealed.size - IV_SIZE), Charsets.UTF_8)
      } catch (e: Exception) {
        null
      }
    }
    JSObject().put("secret", secret)
  }

  @Command
  fun delete(invoke: Invoke) = answer(invoke) {
    prefs.edit().remove(invoke.parseArgs(SecretName::class.java).name).commit()
    JSObject()
  }

  private fun answer(invoke: Invoke, work: () -> JSObject) {
    try {
      invoke.resolve(work())
    } catch (e: Exception) {
      invoke.reject(e.message ?: e.toString())
    }
  }

  private companion object {
    const val KEY_STORE = "AndroidKeyStore"
    const val ALIAS = "pswmanager-secrets"
    const val CIPHER = "AES/GCM/NoPadding"
    const val IV_SIZE = 12
  }
}
