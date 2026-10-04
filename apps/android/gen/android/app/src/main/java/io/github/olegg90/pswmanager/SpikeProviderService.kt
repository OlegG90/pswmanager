package io.github.olegg90.pswmanager

import android.app.Activity
import android.app.PendingIntent
import android.content.Intent
import android.os.Bundle
import android.os.CancellationSignal
import android.os.OutcomeReceiver
import android.os.Process
import android.os.SystemClock
import android.util.Log
import androidx.annotation.RequiresApi
import androidx.credentials.CreatePublicKeyCredentialRequest
import androidx.credentials.exceptions.ClearCredentialException
import androidx.credentials.exceptions.CreateCredentialCancellationException
import androidx.credentials.exceptions.CreateCredentialException
import androidx.credentials.exceptions.GetCredentialCancellationException
import androidx.credentials.exceptions.GetCredentialException
import androidx.credentials.provider.AuthenticationAction
import androidx.credentials.provider.BeginCreateCredentialRequest
import androidx.credentials.provider.BeginCreateCredentialResponse
import androidx.credentials.provider.BeginCreatePublicKeyCredentialRequest
import androidx.credentials.provider.BeginGetCredentialRequest
import androidx.credentials.provider.BeginGetCredentialResponse
import androidx.credentials.provider.BeginGetPublicKeyCredentialOption
import androidx.credentials.provider.CallingAppInfo
import androidx.credentials.provider.CreateEntry
import androidx.credentials.provider.CredentialProviderService
import androidx.credentials.provider.PendingIntentHandler
import androidx.credentials.provider.ProviderClearCredentialStateRequest
import org.json.JSONObject

private const val TAG = "PswmProviderSpike"

/** Member names of a JSON object, nested one level: never values. */
private fun keys(json: String): String = try {
  val o = JSONObject(json)
  o.keys().asSequence().joinToString(", ") { k ->
    val v = o.get(k)
    if (v is JSONObject) "$k{${v.keys().asSequence().joinToString(",")}}" else k
  }
} catch (e: Exception) {
  "unparsable: ${e.message}"
}

private fun describe(app: CallingAppInfo?, allowlist: String): String {
  if (app == null) return "caller: none"
  val origin = try { app.getOrigin(allowlist) } catch (e: Exception) { "error ${e.message}" }
  return "caller: ${app.packageName}, originPopulated=${app.isOriginPopulated()}, privileged origin=${origin != null} (${origin?.length ?: 0} chars)"
}

/**
 * SPIKE (#159), never merged: does Android call PswManager as a credential
 * provider? Logs (tag PswmProviderSpike) only the shape of requests, never
 * values, and how fast the begin phase answers in a cold process.
 */
@RequiresApi(34)
class SpikeProviderService : CredentialProviderService() {
  private val allowlist by lazy { resources.openRawResource(R.raw.privileged_allowlist).bufferedReader().readText() }

  override fun onCreate() {
    super.onCreate()
    val sinceStart = SystemClock.elapsedRealtime() - Process.getStartElapsedRealtime()
    Log.i(TAG, "service created; process started ${sinceStart} ms ago")
  }

  override fun onBeginCreateCredentialRequest(
    request: BeginCreateCredentialRequest,
    cancellationSignal: CancellationSignal,
    callback: OutcomeReceiver<BeginCreateCredentialResponse, CreateCredentialException>,
  ) {
    val t = SystemClock.elapsedRealtime()
    Log.i(TAG, "beginCreate: ${request.javaClass.simpleName}; ${describe(request.callingAppInfo, allowlist)}")
    if (request is BeginCreatePublicKeyCredentialRequest) {
      Log.i(TAG, "  requestJson keys: ${keys(request.requestJson)}; clientDataHash=${request.clientDataHash?.size}")
    }
    val entry = CreateEntry(accountName = "PswManager (spike)", pendingIntent = intent("create"))
    callback.onResult(BeginCreateCredentialResponse(createEntries = listOf(entry)))
    Log.i(TAG, "  answered in ${SystemClock.elapsedRealtime() - t} ms")
  }

  override fun onBeginGetCredentialRequest(
    request: BeginGetCredentialRequest,
    cancellationSignal: CancellationSignal,
    callback: OutcomeReceiver<BeginGetCredentialResponse, GetCredentialException>,
  ) {
    val t = SystemClock.elapsedRealtime()
    Log.i(TAG, "beginGet: ${request.beginGetCredentialOptions.map { it.javaClass.simpleName }}; ${describe(request.callingAppInfo, allowlist)}")
    request.beginGetCredentialOptions.filterIsInstance<BeginGetPublicKeyCredentialOption>().forEach {
      Log.i(TAG, "  requestJson keys: ${keys(it.requestJson)}; clientDataHash=${it.clientDataHash?.size}")
    }
    val unlock = AuthenticationAction(title = "Unlock PswManager (spike)", pendingIntent = intent("unlock"))
    callback.onResult(BeginGetCredentialResponse(authenticationActions = listOf(unlock)))
    Log.i(TAG, "  answered in ${SystemClock.elapsedRealtime() - t} ms")
  }

  override fun onClearCredentialStateRequest(
    request: ProviderClearCredentialStateRequest,
    cancellationSignal: CancellationSignal,
    callback: OutcomeReceiver<Void?, ClearCredentialException>,
  ) {
    Log.i(TAG, "clear state")
    callback.onResult(null)
  }

  private fun intent(action: String): PendingIntent {
    val intent = Intent(this, SpikeProviderActivity::class.java).setAction(action)
    return PendingIntent.getActivity(this, action.hashCode(), intent, PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
  }
}

/** SPIKE (#159): the selection phase; logs what it gets and cancels. */
@RequiresApi(34)
class SpikeProviderActivity : Activity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    val allowlist = resources.openRawResource(R.raw.privileged_allowlist).bufferedReader().readText()
    val result = Intent()
    when (intent.action) {
      "create" -> {
        val request = PendingIntentHandler.retrieveProviderCreateCredentialRequest(intent)
        Log.i(TAG, "selected create: ${request?.callingRequest?.javaClass?.simpleName}; ${describe(request?.callingAppInfo, allowlist)}")
        (request?.callingRequest as? CreatePublicKeyCredentialRequest)?.let {
          Log.i(TAG, "  requestJson keys: ${keys(it.requestJson)}; clientDataHash=${it.clientDataHash?.size}")
        }
        PendingIntentHandler.setCreateCredentialException(result, CreateCredentialCancellationException("Spike"))
      }
      "unlock" -> {
        val request = PendingIntentHandler.retrieveBeginGetCredentialRequest(intent)
        Log.i(TAG, "selected unlock: ${request?.beginGetCredentialOptions?.map { it.javaClass.simpleName }}; ${describe(request?.callingAppInfo, allowlist)}")
        PendingIntentHandler.setGetCredentialException(result, GetCredentialCancellationException("Spike"))
      }
    }
    setResult(RESULT_OK, result)
    finish()
  }
}
