package io.github.olegg90.pswmanager

import android.app.Activity
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class SecretName {
  lateinit var name: String
}

@InvokeArg
class SecretValue {
  lateinit var name: String
  lateinit var secret: String
}

/** The app's secrets, kept by [Keystore]. */
@TauriPlugin
class SecretsPlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun write(invoke: Invoke) = answer(invoke) {
    val args = invoke.parseArgs(SecretValue::class.java)
    Keystore.write(activity, args.name, args.secret)
    JSObject()
  }

  /** The secret, or `secret: null` when there is none (or it cannot be read any more). */
  @Command
  fun read(invoke: Invoke) = answer(invoke) {
    JSObject().put("secret", Keystore.read(activity, invoke.parseArgs(SecretName::class.java).name))
  }

  @Command
  fun delete(invoke: Invoke) = answer(invoke) {
    Keystore.delete(activity, invoke.parseArgs(SecretName::class.java).name)
    JSObject()
  }

  private fun answer(invoke: Invoke, work: () -> JSObject) {
    try {
      invoke.resolve(work())
    } catch (e: Exception) {
      invoke.reject(e.message ?: e.toString())
    }
  }
}
