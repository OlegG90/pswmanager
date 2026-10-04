package io.github.olegg90.pswmanager

import android.os.Bundle
import android.util.Log
import android.widget.ScrollView
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import androidx.credentials.providerevents.ProviderEventsManager
import androidx.credentials.providerevents.exception.ImportCredentialsException
import androidx.credentials.providerevents.transfer.CredentialTypes
import androidx.credentials.providerevents.transfer.ImportCredentialsRequest
import androidx.credentials.providerevents.transfer.KnownExtensions
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject

/**
 * SPIKE (#151), never merged: can PswManager, which is not a credential
 * provider, import through Android's Credential Transfer? Started with
 * `adb shell am start -n io.github.olegg90.pswmanager/.TransferSpikeActivity`.
 * Shows and logs (tag PswmSpike) the exporter and the SHAPE of the CXF JSON:
 * every string value is replaced by its length, except type names.
 */
class TransferSpikeActivity : AppCompatActivity() {
  private lateinit var out: TextView

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    out = TextView(this).apply { setPadding(32, 96, 32, 32); textSize = 12f }
    setContentView(ScrollView(this).apply { addView(out) })
    say("Starting import…")
    val request = ImportCredentialsRequest(
      credentialTypes = setOf(
        CredentialTypes.CREDENTIAL_TYPE_BASIC_AUTH, CredentialTypes.CREDENTIAL_TYPE_PUBLIC_KEY,
        CredentialTypes.CREDENTIAL_TYPE_TOTP, CredentialTypes.CREDENTIAL_TYPE_NOTE,
        CredentialTypes.CREDENTIAL_TYPE_CUSTOM_FIELDS, CredentialTypes.CREDENTIAL_TYPE_ADDRESS,
        CredentialTypes.CREDENTIAL_TYPE_CREDIT_CARD, CredentialTypes.CREDENTIAL_TYPE_WIFI,
        CredentialTypes.CREDENTIAL_TYPE_API_KEY, CredentialTypes.CREDENTIAL_TYPE_SSH_KEY,
        CredentialTypes.CREDENTIAL_TYPE_GENERATED_PASSWORD, CredentialTypes.CREDENTIAL_TYPE_FILE,
      ),
      knownExtensions = setOf(KnownExtensions.KNOWN_EXTENSION_SHARED),
    )
    lifecycleScope.launch {
      try {
        val response = ProviderEventsManager.create(this@TransferSpikeActivity).importCredentials(this@TransferSpikeActivity, request)
        val json = response.response.responseJson
        say("Exporter: ${response.callingAppInfo.packageName}\nJSON length: ${json.length}\n${summary(JSONObject(json))}")
      } catch (e: ImportCredentialsException) {
        say("Failed: ${e.javaClass.simpleName}: ${e.message}")
      } catch (e: Exception) {
        say("Failed: ${e.javaClass.name}: ${e.message}")
      }
    }
  }

  private fun say(text: String) {
    out.text = text
    text.lines().forEach { Log.i("PswmSpike", it) }
  }

  /** Counts and member names only: no values. */
  private fun summary(root: JSONObject): String {
    val lines = mutableListOf<String>()
    lines += "header keys: ${root.keys().asSequence().toList()} version=${root.optJSONObject("version")} exporter=${root.optString("exporterRpId")} / ${root.optString("exporterDisplayName")}"
    val accounts = root.optJSONArray("accounts") ?: JSONArray()
    lines += "accounts: ${accounts.length()}"
    val itemKeys = sortedSetOf<String>()
    val scopeKeys = sortedSetOf<String>()
    val appKeys = sortedSetOf<String>()
    val types = sortedMapOf<String, Int>()
    val members = sortedMapOf<String, MutableSet<String>>()
    var items = 0
    var collections = 0
    var withTags = 0
    var credentialCounts = sortedMapOf<Int, Int>()
    for (a in 0 until accounts.length()) {
      val account = accounts.getJSONObject(a)
      lines += "account keys: ${account.keys().asSequence().toList()}"
      collections += account.optJSONArray("collections")?.length() ?: 0
      val list = account.optJSONArray("items") ?: JSONArray()
      items += list.length()
      for (i in 0 until list.length()) {
        val item = list.getJSONObject(i)
        itemKeys += item.keys().asSequence()
        if ((item.optJSONArray("tags")?.length() ?: 0) > 0) withTags++
        item.optJSONObject("scope")?.let { s ->
          scopeKeys += s.keys().asSequence()
          val apps = s.optJSONArray("androidApps") ?: JSONArray()
          for (k in 0 until apps.length()) appKeys += apps.getJSONObject(k).keys().asSequence()
        }
        val creds = item.optJSONArray("credentials") ?: JSONArray()
        credentialCounts[creds.length()] = (credentialCounts[creds.length()] ?: 0) + 1
        for (c in 0 until creds.length()) {
          val cred = creds.getJSONObject(c)
          val type = cred.optString("type")
          types[type] = (types[type] ?: 0) + 1
          val set = members.getOrPut(type) { sortedSetOf() }
          cred.keys().forEach { k ->
            val v = cred.get(k)
            set += if (v is JSONObject) "$k{${v.keys().asSequence().joinToString(",")}}" else "$k:${v.javaClass.simpleName}"
          }
        }
      }
    }
    lines += "items: $items, collections: $collections, items with tags: $withTags"
    lines += "credentials per item (count -> items): $credentialCounts"
    lines += "item keys: $itemKeys"
    lines += "scope keys: $scopeKeys; androidApp keys: $appKeys"
    lines += "credential types: $types"
    members.forEach { (type, set) -> lines += "  $type members: $set" }
    return lines.joinToString("\n")
  }

  /** The JSON with every string but type names replaced by `<n chars>`. */
  private fun shape(value: Any?, key: String = ""): Any? = when (value) {
    is JSONObject -> JSONObject().also { o -> value.keys().forEach { k -> o.put(k, shape(value.get(k), k)) } }
    is JSONArray -> JSONArray().also { a -> for (i in 0 until value.length()) a.put(shape(value.get(i), key)) }
    is String -> if (key in KEEP) value else "<${value.length} chars>"
    else -> value
  }

  companion object {
    private val KEEP = setOf("type", "fieldType", "exporterRpId", "exporterDisplayName", "algorithm", "hashAlg")
  }
}
