package io.github.olegg90.pswmanager

import android.app.assist.AssistStructure
import android.app.assist.AssistStructure.ViewNode
import android.text.InputType
import android.view.View
import android.view.autofill.AutofillId

/**
 * The login fields of a screen another app shows (#236): its user name and
 * password fields, the app's package, and the web domain (with its scheme)
 * of the page they are on when the screen is a page in a browser. Either
 * field may be missing: a sign-in in two steps asks for the user name first.
 * Read from a screen, the fields also hold their text: what the user typed
 * in, when Android asks to save (it puts the latest values into the screen).
 */
class AutofillFields(
  val packageName: String,
  val webDomain: String?,
  val webScheme: String?,
  val username: AutofillId?,
  val password: AutofillId?,
  val usernameText: String? = null,
  /** The password to save: on a form that changes it, the new one. */
  val passwordText: String? = null,
) {
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
      var username: Field? = null
      var password: Field? = null
      // On a form that changes the password (current, new, again), the one
      // to save is the new one: the field that says so, else the last.
      var newPassword: Field? = null
      var lastPassword: Field? = null
      var lastText: Field? = null
      // A field is on the page of its nearest node with a web domain: an
      // iframe from another site is a page of its own.
      fun visit(node: ViewNode, page: Page?) {
        val here = node.webDomain?.takeIf { it.isNotEmpty() }?.let { Page(it, node.webScheme) } ?: page
        val id = node.autofillId
        if (id != null && node.autofillType == View.AUTOFILL_TYPE_TEXT && node.visibility == View.VISIBLE) {
          val field = Field(id, here, node.autofillValue?.takeIf { it.isText }?.textValue?.toString())
          when (kind(node)) {
            Kind.PASSWORD -> {
              if (password == null) password = field
              if (newPassword == null && isNewPassword(node)) newPassword = field
              lastPassword = field
            }
            Kind.USERNAME -> if (username == null) username = field
            Kind.TEXT -> if (password == null) lastText = field
            Kind.OTHER -> {}
          }
        }
        for (i in 0 until node.childCount) visit(node.getChildAt(i), here)
      }
      for (i in 0 until structure.windowNodeCount) visit(structure.getWindowNodeAt(i).rootViewNode, null)
      val pass = password
      val user = (username ?: lastText.takeIf { pass != null })?.takeIf { pass == null || it.page == pass.page }
      val page = (pass ?: user ?: return null).page
      val saved = (newPassword ?: lastPassword)?.takeIf { it.page == page } ?: pass
      return AutofillFields(structure.activityComponent.packageName, page?.domain, page?.scheme, user?.id, pass?.id, user?.text, saved?.text)
    }

    /** The page a field is on: its web domain and scheme. */
    private data class Page(val domain: String, val scheme: String?)

    private class Field(val id: AutofillId, val page: Page?, val text: String?)

    // Lower case, as the hints are compared (`emailAddress` among them).
    private val PASSWORD_HINTS = setOf(View.AUTOFILL_HINT_PASSWORD, "current-password", "new-password").map { it.lowercase() }.toSet()
    private val USERNAME_HINTS = setOf(View.AUTOFILL_HINT_USERNAME, View.AUTOFILL_HINT_EMAIL_ADDRESS, "email", "username").map { it.lowercase() }.toSet()
    private val PASSWORD_WORDS = listOf("password", "passwd", "pwd", "пароль")
    private val USERNAME_WORDS = listOf("user", "login", "email", "e-mail", "логін", "пошта")

    /** A password field for a new password: by its autofill hint or its `autocomplete`. */
    private fun isNewPassword(node: ViewNode): Boolean =
      node.autofillHints.orEmpty().any { it.equals("new-password", ignoreCase = true) } ||
        node.htmlInfo?.attributes.orEmpty().any { it.first.equals("autocomplete", ignoreCase = true) && it.second.equals("new-password", ignoreCase = true) }

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
