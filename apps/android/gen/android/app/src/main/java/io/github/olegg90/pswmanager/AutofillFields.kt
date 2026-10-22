package io.github.olegg90.pswmanager

import android.app.assist.AssistStructure
import android.app.assist.AssistStructure.ViewNode
import android.text.InputType
import android.view.View
import android.view.autofill.AutofillId

/**
 * The login fields of a screen another app shows (#236): its user name and
 * password fields, the app's package, and the web domain when the screen is a
 * page in a browser. Either field may be missing: a sign-in in two steps asks
 * for the user name first.
 */
class AutofillFields(val packageName: String, val webDomain: String?, val username: AutofillId?, val password: AutofillId?) {
  /** The fields to fill, the user name first. */
  val ids: List<AutofillId> get() = listOfNotNull(username, password)

  private enum class Kind { USERNAME, PASSWORD, TEXT, OTHER }

  companion object {
    /**
     * The login fields of `structure`, or null when it has none. A field says
     * what it is by its autofill hints, its HTML attributes or its input type,
     * else by words in its id or hint; a text field just before the first
     * password is taken for the user name when nothing says so.
     */
    fun of(structure: AssistStructure): AutofillFields? {
      var webDomain: String? = null
      var username: AutofillId? = null
      var password: AutofillId? = null
      var lastText: AutofillId? = null
      fun visit(node: ViewNode) {
        if (webDomain == null) node.webDomain?.takeIf { it.isNotEmpty() }?.let { webDomain = it }
        val id = node.autofillId
        if (id != null && node.autofillType == View.AUTOFILL_TYPE_TEXT && node.visibility == View.VISIBLE) {
          when (kind(node)) {
            Kind.PASSWORD -> if (password == null) password = id
            Kind.USERNAME -> if (username == null) username = id
            Kind.TEXT -> if (password == null) lastText = id
            Kind.OTHER -> {}
          }
        }
        for (i in 0 until node.childCount) visit(node.getChildAt(i))
      }
      for (i in 0 until structure.windowNodeCount) visit(structure.getWindowNodeAt(i).rootViewNode)
      val user = username ?: lastText.takeIf { password != null }
      if (user == null && password == null) return null
      return AutofillFields(structure.activityComponent.packageName, webDomain, user, password)
    }

    private val PASSWORD_HINTS = setOf(View.AUTOFILL_HINT_PASSWORD, "current-password", "new-password")
    private val USERNAME_HINTS = setOf(View.AUTOFILL_HINT_USERNAME, View.AUTOFILL_HINT_EMAIL_ADDRESS, "email", "username")
    private val PASSWORD_WORDS = listOf("password", "passwd", "pwd", "пароль")
    private val USERNAME_WORDS = listOf("user", "login", "email", "e-mail", "account", "логін", "пошта")

    private fun kind(node: ViewNode): Kind {
      val hints = node.autofillHints?.map { it.lowercase() }.orEmpty()
      if (hints.any { it in PASSWORD_HINTS }) return Kind.PASSWORD
      if (hints.any { it in USERNAME_HINTS }) return Kind.USERNAME
      // A page's input: its type, then its name and id.
      val html = node.htmlInfo?.takeIf { it.tag.equals("input", ignoreCase = true) }
      val attributes = html?.attributes.orEmpty().associate { it.first.lowercase() to it.second.orEmpty().lowercase() }
      when (attributes["type"]) {
        "password" -> return Kind.PASSWORD
        "email" -> return Kind.USERNAME
        "hidden", "checkbox", "radio", "submit", "button", "search" -> return Kind.OTHER
      }
      val type = node.inputType
      if (type and InputType.TYPE_MASK_CLASS == InputType.TYPE_CLASS_TEXT) {
        when (type and InputType.TYPE_MASK_VARIATION) {
          InputType.TYPE_TEXT_VARIATION_PASSWORD, InputType.TYPE_TEXT_VARIATION_WEB_PASSWORD, InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD -> return Kind.PASSWORD
          InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS, InputType.TYPE_TEXT_VARIATION_WEB_EMAIL_ADDRESS -> return Kind.USERNAME
        }
      }
      val words = listOfNotNull(node.idEntry, node.hint, attributes["name"], attributes["id"], attributes["autocomplete"]).joinToString(" ").lowercase()
      if (PASSWORD_WORDS.any { it in words }) return Kind.PASSWORD
      if (USERNAME_WORDS.any { it in words }) return Kind.USERNAME
      val editable = html != null || type and InputType.TYPE_MASK_CLASS == InputType.TYPE_CLASS_TEXT
      return if (editable) Kind.TEXT else Kind.OTHER
    }
  }
}
