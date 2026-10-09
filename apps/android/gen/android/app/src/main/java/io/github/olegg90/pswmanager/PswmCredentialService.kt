package io.github.olegg90.pswmanager

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.CancellationSignal
import android.os.OutcomeReceiver
import androidx.annotation.RequiresApi
import androidx.credentials.exceptions.ClearCredentialException
import androidx.credentials.exceptions.CreateCredentialException
import androidx.credentials.exceptions.CreateCredentialUnsupportedException
import androidx.credentials.exceptions.GetCredentialException
import androidx.credentials.provider.AuthenticationAction
import androidx.credentials.provider.BeginCreateCredentialRequest
import androidx.credentials.provider.BeginCreateCredentialResponse
import androidx.credentials.provider.BeginCreatePasswordCredentialRequest
import androidx.credentials.provider.BeginCreatePublicKeyCredentialRequest
import androidx.credentials.provider.BeginGetCredentialRequest
import androidx.credentials.provider.BeginGetCredentialResponse
import androidx.credentials.provider.BeginGetPasswordOption
import androidx.credentials.provider.BeginGetPublicKeyCredentialOption
import androidx.credentials.provider.CreateEntry
import androidx.credentials.provider.CredentialProviderService
import androidx.credentials.provider.PasswordCredentialEntry
import androidx.credentials.provider.ProviderClearCredentialStateRequest
import androidx.credentials.provider.PublicKeyCredentialEntry
import org.json.JSONArray
import java.io.File

/** What Rust does for the credential provider (`src/provider.rs`). */
object ProviderBridge {
  init {
    System.loadLibrary("pswm_android_lib")
  }

  external fun isUnlocked(): Boolean

  /** The setting allows the fingerprint and the master password is not due. */
  external fun fingerprintAllowed(state: String): Boolean

  /**
   * Unlocks with the master password, or the sealed key (`sealed`); null, or
   * why not ([STALE] first when the sealed key no longer opens the database).
   */
  external fun unlock(context: Context, state: String, password: String?, sealed: String?): String?

  /** The passkeys for a sign-in request, as JSON (`[{id, title, username}]`); null while locked. */
  external fun passkeys(request: String): String?

  /**
   * Signs in with the passkey of entry `id`: the answer's JSON, or [FAILED]
   * and why. A privileged browser gives `origin` and `clientDataHash`; an app
   * its package and signing certificate.
   */
  external fun sign(id: String, request: String, origin: String?, clientDataHash: ByteArray?, packageName: String, certificate: ByteArray?): String

  /**
   * Where a new passkey for a creation request can go, as JSON (`{site,
   * excluded, entries: [{id, title, username}], cloud}`), or [FAILED] and why.
   */
  external fun newPasskeyChoices(state: String, request: String): String

  /** Makes a passkey into entry `id` (a new one when null) and saves: the answer's JSON, or [FAILED] and why. */
  external fun makePasskey(id: String?, request: String, origin: String?, clientDataHash: ByteArray?, packageName: String, certificate: ByteArray?): String

  /** The logins for the app (or the site a browser speaks for), as JSON (`[{id, title, username}]`); null while locked. */
  external fun logins(origin: String?, packageName: String): String?

  /** Entry `id`'s login as JSON (`{username, password}`); null while locked or when gone. */
  external fun login(id: String): String?

  /** The entries a login offered can update, as JSON (`{entries, cloud}`), or [FAILED] and why. */
  external fun loginChoices(state: String, origin: String?, packageName: String): String

  /** Saves a login into entry `id` (a new one titled `title` when null): empty, or [FAILED] and why. */
  external fun saveLogin(id: String?, origin: String?, packageName: String, title: String, username: String, password: String): String

  /**
   * The database's entries in use as CXF JSON for another password manager
   * ([ExportActivity]), the user checked with the master password or the
   * sealed key; [FAILED] and why (then [STALE] for a stale sealed key).
   */
  external fun export(context: Context, state: String, password: String?, sealed: String?): String?

  const val STALE = "stale:"

  /** The entry (and its title) a picked passkey or login is for, in the activity's intent. */
  const val EXTRA_ENTRY = "entry"
  const val EXTRA_TITLE = "title"
  const val FAILED = "failed:"

  /** The app's state file, as Tauri keeps it (its data folder, `app_data_dir`; `STATE_FILE` in app.rs). */
  fun state(context: Context): String = File(context.dataDir, "pswm.json").absolutePath
}

/**
 * PswManager as a credential provider (stage A5, Android 14+): Android asks
 * every enabled provider at once and needs a quick answer, given here from
 * memory only. While the database is locked the only answer is *Unlock
 * PswManager* ([UnlockActivity]); nothing about entries is told. Unlocked,
 * it offers the site's passkeys ([PasskeyActivity] signs in with the one
 * picked); asked to make one, it offers to keep it ([CreatePasskeyActivity]).
 * Apps that use Credential Manager for passwords get their logins
 * ([PasswordActivity]) and can save new ones ([CreatePasswordActivity]).
 */
@RequiresApi(34)
class PswmCredentialService : CredentialProviderService() {
  override fun onBeginGetCredentialRequest(
    request: BeginGetCredentialRequest,
    cancellationSignal: CancellationSignal,
    callback: OutcomeReceiver<BeginGetCredentialResponse, GetCredentialException>,
  ) {
    if (!ProviderBridge.isUnlocked()) {
      val unlock = AuthenticationAction(title = "Unlock PswManager", pendingIntent = unlockIntent(this))
      callback.onResult(BeginGetCredentialResponse(authenticationActions = listOf(unlock)))
      return
    }
    callback.onResult(credentials(this, request))
  }

  override fun onBeginCreateCredentialRequest(
    request: BeginCreateCredentialRequest,
    cancellationSignal: CancellationSignal,
    callback: OutcomeReceiver<BeginCreateCredentialResponse, CreateCredentialException>,
  ) {
    // Locked or not: the activity unlocks first when it must.
    val activity = when (request) {
      is BeginCreatePublicKeyCredentialRequest -> CreatePasskeyActivity::class.java
      is BeginCreatePasswordCredentialRequest -> CreatePasswordActivity::class.java
      else -> return callback.onError(CreateCredentialUnsupportedException("PswManager keeps passkeys and passwords only"))
    }
    val save = PendingIntent.getActivity(this, 3, Intent(this, activity), PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    callback.onResult(BeginCreateCredentialResponse(createEntries = listOf(CreateEntry(accountName = "PswManager", pendingIntent = save))))
  }

  override fun onClearCredentialStateRequest(
    request: ProviderClearCredentialStateRequest,
    cancellationSignal: CancellationSignal,
    callback: OutcomeReceiver<Void?, ClearCredentialException>,
  ) {
    callback.onResult(null)
  }

  companion object {
    /** Opens [UnlockActivity]; mutable, so Android can add the request to it. */
    fun unlockIntent(context: Context): PendingIntent {
      val intent = Intent(context, UnlockActivity::class.java)
      return PendingIntent.getActivity(context, 1, intent, PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }

    /** What the database (unlocked) has for the request: the site's passkeys, the app's (or site's) logins. */
    fun credentials(context: Context, request: BeginGetCredentialRequest): BeginGetCredentialResponse {
      val passkeys = request.beginGetCredentialOptions.filterIsInstance<BeginGetPublicKeyCredentialOption>().flatMap { option ->
        val found = JSONArray(ProviderBridge.passkeys(option.requestJson) ?: "[]")
        (0 until found.length()).map { i ->
          val passkey = found.getJSONObject(i)
          val title = passkey.optString("title")
          PublicKeyCredentialEntry(
            context = context,
            username = passkey.optString("username").ifEmpty { title },
            pendingIntent = entryIntent(context, PasskeyActivity::class.java, passkey.getString("id"), title),
            beginGetPublicKeyCredentialOption = option,
            displayName = title,
          )
        }
      }
      val passwordOptions = request.beginGetCredentialOptions.filterIsInstance<BeginGetPasswordOption>()
      val caller = request.callingAppInfo?.takeIf { passwordOptions.isNotEmpty() }?.let { app ->
        try {
          Caller.of(context, app)
        } catch (e: Exception) {
          android.util.Log.w("PswmProvider", "No logins offered: who asks is unknown (${e.javaClass.simpleName})")
          null
        }
      }
      val found = JSONArray(caller?.let { ProviderBridge.logins(it.origin, it.packageName) } ?: "[]")
      val logins = passwordOptions.flatMap { option ->
        (0 until found.length()).map { i ->
          val login = found.getJSONObject(i)
          val title = login.optString("title")
          PasswordCredentialEntry(
            context = context,
            username = login.optString("username").ifEmpty { title },
            pendingIntent = entryIntent(context, PasswordActivity::class.java, login.getString("id"), title),
            beginGetPasswordOption = option,
            displayName = title,
          )
        }
      }
      return BeginGetCredentialResponse(credentialEntries = passkeys + logins)
    }

    /** Opens `activity` for entry `id`: the activity and the entry's id make the intent its own (extras do not). */
    private fun entryIntent(context: Context, activity: Class<*>, id: String, title: String): PendingIntent {
      val intent = Intent(context, activity)
        .setIdentifier(id)
        .putExtra(ProviderBridge.EXTRA_ENTRY, id)
        .putExtra(ProviderBridge.EXTRA_TITLE, title)
      return PendingIntent.getActivity(context, 2, intent, PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }

    @Volatile private var allowlist: String? = null

    /** Google's list of browsers trusted to speak for a site (kept with the app, updated with releases), read once. */
    fun allowlist(context: Context): String =
      allowlist ?: context.resources.openRawResource(R.raw.privileged_allowlist).bufferedReader().use { it.readText() }.also { allowlist = it }
  }
}
