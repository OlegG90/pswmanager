package io.github.olegg90.pswmanager

import android.app.Activity
import androidx.fragment.app.FragmentActivity
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import javax.crypto.Cipher

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
 * Biometric unlock for the app: the sealed key ([BiometricKey]) stored,
 * retrieved and forgotten for Rust.
 *
 * Errors told apart (by Rust): `biometric:cancelled` (the user chose the
 * master password), `biometric:invalidated` (the Keystore key is gone; the
 * sealed key is deleted with it), `biometric:none` (nothing is stored).
 */
@TauriPlugin
class BiometricPlugin(private val activity: Activity) : Plugin(activity) {
  /** Whether this phone can (a strong biometric is enrolled) and whether a key is stored. */
  @Command
  fun status(invoke: Invoke) {
    invoke.resolve(JSObject().put("available", BiometricKey.available(activity)).put("stored", BiometricKey.isStored(activity)))
  }

  /** Seals `secret` after a fingerprint or face confirms it. */
  @Command
  fun store(invoke: Invoke) {
    val args = invoke.parseArgs(StoreArgs::class.java)
    val cipher = try {
      BiometricKey.encrypting(activity)
    } catch (e: Exception) {
      return invoke.reject(e.message ?: e.toString())
    }
    prompt(invoke, args.title, cipher) { ready ->
      BiometricKey.seal(activity, ready, args.secret)
      JSObject()
    }
  }

  /** The sealed secret, after a fingerprint or face. */
  @Command
  fun retrieve(invoke: Invoke) {
    val title = invoke.parseArgs(PromptArgs::class.java).title
    val cipher = try {
      BiometricKey.decrypting(activity) ?: return invoke.reject("biometric:none")
    } catch (e: Exception) {
      return invoke.reject(e.message ?: e.toString())
    }
    prompt(invoke, title, cipher) { ready -> JSObject().put("secret", BiometricKey.open(activity, ready)) }
  }

  /** Deletes the sealed key and the Keystore key. */
  @Command
  fun forget(invoke: Invoke) {
    try {
      BiometricKey.forget(activity)
      invoke.resolve(JSObject())
    } catch (e: Exception) {
      invoke.reject(e.message ?: e.toString())
    }
  }

  private fun prompt(invoke: Invoke, title: String, cipher: Cipher, done: (Cipher) -> JSObject) {
    BiometricKey.prompt(activity as FragmentActivity, title, cipher, { ready ->
      try {
        invoke.resolve(done(ready))
      } catch (e: Exception) {
        invoke.reject(e.message ?: e.toString())
      }
    }) { cancelled, message -> invoke.reject(if (cancelled) "biometric:cancelled" else message) }
  }
}
