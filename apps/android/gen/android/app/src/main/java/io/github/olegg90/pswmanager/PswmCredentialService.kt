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
import androidx.credentials.provider.BeginGetCredentialRequest
import androidx.credentials.provider.BeginGetCredentialResponse
import androidx.credentials.provider.BeginGetPublicKeyCredentialOption
import androidx.credentials.provider.CredentialProviderService
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

  const val STALE = "stale:"
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
 * picked). Making passkeys comes in #163.
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
    callback.onResult(passkeys(this, request))
  }

  override fun onBeginCreateCredentialRequest(
    request: BeginCreateCredentialRequest,
    cancellationSignal: CancellationSignal,
    callback: OutcomeReceiver<BeginCreateCredentialResponse, CreateCredentialException>,
  ) {
    callback.onError(CreateCredentialUnsupportedException("PswManager does not make passkeys yet"))
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

    /** The passkeys the database (unlocked) has for each passkey the request asks for. */
    fun passkeys(context: Context, request: BeginGetCredentialRequest): BeginGetCredentialResponse {
      val entries = request.beginGetCredentialOptions.filterIsInstance<BeginGetPublicKeyCredentialOption>().flatMap { option ->
        val found = JSONArray(ProviderBridge.passkeys(option.requestJson) ?: "[]")
        (0 until found.length()).map { i ->
          val passkey = found.getJSONObject(i)
          val title = passkey.optString("title")
          PublicKeyCredentialEntry(
            context = context,
            username = passkey.optString("username").ifEmpty { title },
            pendingIntent = signIntent(context, passkey.getString("id"), title),
            beginGetPublicKeyCredentialOption = option,
            displayName = title,
          )
        }
      }
      return BeginGetCredentialResponse(credentialEntries = entries)
    }

    /** Opens [PasskeyActivity] for entry `id`; each entry its own request code. */
    private fun signIntent(context: Context, id: String, title: String): PendingIntent {
      val intent = Intent(context, PasskeyActivity::class.java)
        .putExtra(PasskeyActivity.EXTRA_ENTRY, id)
        .putExtra(PasskeyActivity.EXTRA_TITLE, title)
      return PendingIntent.getActivity(context, id.hashCode(), intent, PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }

    /** Google's list of browsers trusted to speak for a site (kept with the app, updated with releases). */
    fun allowlist(context: Context): String = context.resources.openRawResource(R.raw.privileged_allowlist).bufferedReader().use { it.readText() }
  }
}
