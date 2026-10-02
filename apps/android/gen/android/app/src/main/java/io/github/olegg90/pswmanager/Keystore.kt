package io.github.olegg90.pswmanager

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Secrets kept between runs (a store's refresh token, the icon cache's key):
 * encrypted with an AES key in the Android Keystore, which never leaves it,
 * and kept in the app's private preferences. The key needs no fingerprint, so
 * the background upload ([UploadWorker]) can use the tokens too. Used by
 * [SecretsPlugin] and, from Rust through JNI, by the background upload.
 */
object Keystore {
  private const val KEY_STORE = "AndroidKeyStore"
  private const val ALIAS = "pswmanager-secrets"
  private const val CIPHER = "AES/GCM/NoPadding"
  private const val IV_SIZE = 12

  private fun prefs(context: Context) = context.getSharedPreferences("secrets", Context.MODE_PRIVATE)

  private fun key(): SecretKey {
    val keyStore = KeyStore.getInstance(KEY_STORE).apply { load(null) }
    (keyStore.getKey(ALIAS, null) as SecretKey?)?.let { return it }
    val spec = KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
      .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
      .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
      .build()
    return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEY_STORE).apply { init(spec) }.generateKey()
  }

  @JvmStatic
  fun write(context: Context, name: String, secret: String) {
    val cipher = Cipher.getInstance(CIPHER).apply { init(Cipher.ENCRYPT_MODE, key()) }
    val sealed = cipher.iv + cipher.doFinal(secret.toByteArray(Charsets.UTF_8))
    if (!prefs(context).edit().putString(name, Base64.encodeToString(sealed, Base64.NO_WRAP)).commit()) {
      throw Exception("Cannot keep the secret")
    }
  }

  /** The secret, or null when there is none (or it cannot be read any more). */
  @JvmStatic
  fun read(context: Context, name: String): String? {
    val stored = prefs(context).getString(name, null) ?: return null
    return try {
      val sealed = Base64.decode(stored, Base64.NO_WRAP)
      val cipher = Cipher.getInstance(CIPHER)
      cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, sealed, 0, IV_SIZE))
      String(cipher.doFinal(sealed, IV_SIZE, sealed.size - IV_SIZE), Charsets.UTF_8)
    } catch (e: Exception) {
      null
    }
  }

  @JvmStatic
  fun delete(context: Context, name: String) {
    prefs(context).edit().remove(name).commit()
  }
}
