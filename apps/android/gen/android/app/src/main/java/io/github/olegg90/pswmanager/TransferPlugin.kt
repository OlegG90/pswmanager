package io.github.olegg90.pswmanager

import android.app.Activity
import androidx.appcompat.app.AppCompatActivity
import androidx.credentials.providerevents.ProviderEventsManager
import androidx.credentials.providerevents.exception.ImportCredentialsCancellationException
import androidx.credentials.providerevents.exception.ImportCredentialsNoExportOptionException
import androidx.credentials.providerevents.transfer.ClearExportRequest
import androidx.credentials.providerevents.transfer.CredentialTypes
import androidx.credentials.providerevents.transfer.ExportEntry
import androidx.credentials.providerevents.transfer.ImportCredentialsRequest
import androidx.credentials.providerevents.transfer.KnownExtensions
import androidx.credentials.providerevents.transfer.RegisterExportRequest
import androidx.core.content.ContextCompat
import androidx.core.graphics.drawable.toBitmap
import androidx.lifecycle.lifecycleScope
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

@InvokeArg
class ExportArgs {
  /** The database, as the importing app lists it. */
  lateinit var name: String
}

/**
 * Importing from another password manager (#152) through Android's
 * Credential Transfer: the system lists the apps that can export, the user
 * picks one and confirms there, and the export (FIDO CXF JSON) comes back to
 * Rust, which reads it into entries. A credential provider is not needed to
 * import (tried in #151).
 *
 * Errors told apart (by Rust): `transfer:cancelled` (the user went back),
 * `transfer:none` (no app on this phone can export).
 */
@TauriPlugin
class TransferPlugin(private val activity: Activity) : Plugin(activity) {
  /** The export of the app the user picks: `{ json }`. */
  @Command
  fun importCredentials(invoke: Invoke) {
    val request = ImportCredentialsRequest(credentialTypes = TYPES, knownExtensions = setOf(KnownExtensions.KNOWN_EXTENSION_SHARED))
    // Rust waits for the answer: every way out settles the invoke.
    val scope = (activity as? AppCompatActivity)?.lifecycleScope ?: return invoke.reject("Importing needs the app's own window")
    scope.launch {
      try {
        val response = ProviderEventsManager.create(activity).importCredentials(activity, request)
        invoke.resolve(JSObject().put("json", response.response.responseJson))
      } catch (e: ImportCredentialsCancellationException) {
        invoke.reject("transfer:cancelled")
      } catch (e: CancellationException) {
        // The window went away while the other app was open.
        invoke.reject("transfer:cancelled")
        throw e
      } catch (e: ImportCredentialsNoExportOptionException) {
        invoke.reject("transfer:none")
      } catch (e: Exception) {
        invoke.reject(e.message ?: e.toString())
      }
    }
  }

  /**
   * Offers the database for export to other password managers (#153): Android's
   * Credential Transfer then lists PswManager among the apps to import from, and
   * starts [ExportActivity] when it is picked. Registering again replaces it.
   */
  @Command
  fun registerExport(invoke: Invoke) {
    val args = invoke.parseArgs(ExportArgs::class.java)
    val scope = (activity as? AppCompatActivity)?.lifecycleScope ?: return invoke.reject("Needs the app's own window")
    scope.launch {
      try {
        val icon = ContextCompat.getDrawable(activity, R.mipmap.ic_launcher)!!.toBitmap(ICON, ICON)
        val entry = ExportEntry(ExportRegistration.id(activity), args.name, "PswManager", icon, EXPORTED)
        ProviderEventsManager.create(activity).registerExport(RegisterExportRequest.create(activity, listOf(entry)))
        ExportRegistration.registered(activity, args.name)
        invoke.resolve(JSObject())
      } catch (e: CancellationException) {
        invoke.reject("transfer:cancelled")
        throw e
      } catch (e: Exception) {
        invoke.reject(e.message ?: e.toString())
      }
    }
  }

  /** The database is forgotten: PswManager is no longer offered for export. */
  @Command
  fun clearExport(invoke: Invoke) {
    val scope = (activity as? AppCompatActivity)?.lifecycleScope ?: return invoke.reject("Needs the app's own window")
    scope.launch {
      try {
        ProviderEventsManager.create(activity).clearExport(ClearExportRequest())
        ExportRegistration.cleared(activity)
        invoke.resolve(JSObject())
      } catch (e: CancellationException) {
        invoke.reject("transfer:cancelled")
        throw e
      } catch (e: Exception) {
        invoke.reject(e.message ?: e.toString())
      }
    }
  }

  companion object {
    /** The icon's side for the system's list, in pixels (it scales it to 32). */
    private const val ICON = 96

    /** What the core writes ([crates/core/src/cxf.rs] `export`). */
    private val EXPORTED = setOf(
      CredentialTypes.CREDENTIAL_TYPE_BASIC_AUTH, CredentialTypes.CREDENTIAL_TYPE_PUBLIC_KEY,
      CredentialTypes.CREDENTIAL_TYPE_TOTP, CredentialTypes.CREDENTIAL_TYPE_NOTE,
      CredentialTypes.CREDENTIAL_TYPE_CUSTOM_FIELDS,
    )

    /** What the core reads into entries (crates/core/src/cxf.rs). */
    private val TYPES = setOf(
      CredentialTypes.CREDENTIAL_TYPE_BASIC_AUTH, CredentialTypes.CREDENTIAL_TYPE_PUBLIC_KEY,
      CredentialTypes.CREDENTIAL_TYPE_TOTP, CredentialTypes.CREDENTIAL_TYPE_NOTE,
      CredentialTypes.CREDENTIAL_TYPE_CUSTOM_FIELDS, CredentialTypes.CREDENTIAL_TYPE_GENERATED_PASSWORD,
      CredentialTypes.CREDENTIAL_TYPE_ADDRESS, CredentialTypes.CREDENTIAL_TYPE_CREDIT_CARD,
      CredentialTypes.CREDENTIAL_TYPE_WIFI, CredentialTypes.CREDENTIAL_TYPE_API_KEY,
      CredentialTypes.CREDENTIAL_TYPE_SSH_KEY, CredentialTypes.CREDENTIAL_TYPE_PERSON_NAME,
      CredentialTypes.CREDENTIAL_TYPE_PASSPORT, CredentialTypes.CREDENTIAL_TYPE_DRIVERS_LICENSE,
      CredentialTypes.CREDENTIAL_TYPE_IDENTITY_DOCUMENT,
    )
  }
}
