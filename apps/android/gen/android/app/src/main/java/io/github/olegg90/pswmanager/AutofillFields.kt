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
 * A form for a card has its [card] fields too (#412), offered the cards.
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
  /** A card's fields, when the screen has a card number field. */
  val card: CardFields? = null,
  /** What the card's fields hold, when Android asks to save (#412). */
  val cardTyped: TypedCard? = null,
) {
  /** The login fields to fill, the user name first. */
  val loginIds: List<AutofillId> get() = listOfNotNull(username, password)

  /** Every field an offer may fill (the login's, then the card's): what unlocking covers. */
  val ids: List<AutofillId> get() = loginIds + card?.ids.orEmpty()

  /** The same fields without the card's: cards are turned off. */
  fun withoutCard(): AutofillFields = AutofillFields(packageName, webDomain, webScheme, username, password, usernameText, passwordText)

  private enum class Kind { USERNAME, PASSWORD, TEXT, OTHER }

  private enum class CardKind { NUMBER, CODE, EXPIRY, MONTH, YEAR, NAME }

  companion object {
    /**
     * The login fields of `structure`, or null when it has none. A field says
     * what it is by its autofill hints, its HTML attributes or its input type,
     * else by words in its id or hint; a text field just before the first
     * password is taken for the user name when nothing says so.
     *
     * The structure is another app's and may hold anything, including nulls
     * the platform types say never occur: a screen that cannot be read has
     * no login fields, rather than stopping the autofill service.
     */
    fun of(structure: AssistStructure?): AutofillFields? =
      try {
        structure?.let(::read)
      } catch (e: RuntimeException) {
        android.util.Log.w("PswmAutofill", "Unreadable screen: ${e.javaClass.simpleName}")
        null
      }

    private fun read(structure: AssistStructure): AutofillFields? {
      // The text fields in order, each with what it is for a login, read once
      // the card's fields are known.
      val texts = mutableListOf<Text>()
      // A card's fields, the first of each kind; only with a number field are they a card.
      val card = mutableMapOf<CardKind, Field>()
      // A field is on the page of its nearest node with a web domain: an
      // iframe from another site is a page of its own.
      fun visit(node: ViewNode?, page: Page?) {
        if (node == null) return
        val here = node.webDomain?.takeIf { it.isNotEmpty() }?.let { Page(it, node.webScheme) } ?: page
        val id = node.autofillId
        val type = node.autofillType
        // The expiry is often lists (a page's month and year) or a date: read
        // for a card only. Everything else is text. A page's list counts
        // whatever type the browser gives it.
        val select = node.htmlInfo?.tag.equals("select", ignoreCase = true)
        if (id != null && node.visibility == View.VISIBLE && (type == View.AUTOFILL_TYPE_LIST || type == View.AUTOFILL_TYPE_DATE || select)) {
          val options = node.autofillOptions.orEmpty().map { it.toString() }
          cardKind(node, options)?.takeIf { it in EXPIRY_KINDS }?.let { card.putIfAbsent(it, Field(id, here, chosen(node, options), type, options)) }
        }
        if (id != null && type == View.AUTOFILL_TYPE_TEXT && node.visibility == View.VISIBLE) {
          val field = Field(id, here, node.autofillValue?.takeIf { it.isText }?.textValue?.toString())
          cardKind(node, emptyList())?.let { card.putIfAbsent(it, field) }
          texts += Text(field, kind(node), isNewPassword(node))
        }
        for (i in 0 until node.childCount) visit(node.getChildAt(i), here)
      }
      for (i in 0 until structure.windowNodeCount) visit(structure.getWindowNodeAt(i)?.rootViewNode, null)
      // The card's fields on the number's page (a payment form may be a frame of its own).
      val number = card[CardKind.NUMBER]
      val onCard = { kind: CardKind -> card[kind]?.takeIf { it.page == number?.page } }
      val expiry = { kind: CardKind -> onCard(kind)?.let { CardField(it.id, it.type, it.options) } }
      val cardFields = number?.let {
        CardFields(it.id, onCard(CardKind.CODE)?.id, expiry(CardKind.EXPIRY), expiry(CardKind.MONTH), expiry(CardKind.YEAR), onCard(CardKind.NAME)?.id)
      }
      // A card's field is not a login's (a code is often a password field);
      // without a card number on the screen, a field that only sounds like a
      // card's ("Card number or user name") stays a login's.
      val cardIds = cardFields?.ids.orEmpty().toSet()
      var username: Field? = null
      var password: Field? = null
      // On a form that changes the password (current, new, again), the one
      // to save is the new one: the field that says so, else the last.
      var newPassword: Field? = null
      var lastPassword: Field? = null
      var lastText: Field? = null
      for ((field, kind, isNew) in texts.filter { it.field.id !in cardIds }) {
        when (kind) {
          Kind.PASSWORD -> {
            if (password == null) password = field
            if (newPassword == null && isNew) newPassword = field
            lastPassword = field
          }
          Kind.USERNAME -> if (username == null) username = field
          Kind.TEXT -> if (password == null) lastText = field
          Kind.OTHER -> {}
        }
      }
      val pass = password
      val user = (username ?: lastText.takeIf { pass != null })?.takeIf { pass == null || it.page == pass.page }
      val page = (pass ?: user ?: number ?: return null).page
      val saved = (newPassword ?: lastPassword)?.takeIf { it.page == page } ?: pass
      // What the card's fields hold, for a save: the number typed is what makes it one.
      val typedCard = number?.text?.takeIf { it.isNotBlank() }?.let {
        val text = { kind: CardKind -> onCard(kind)?.text.orEmpty() }
        TypedCard(text(CardKind.NAME), it, CardValues.typedExpiry(onCard(CardKind.EXPIRY)?.text, onCard(CardKind.MONTH)?.text, onCard(CardKind.YEAR)?.text), text(CardKind.CODE))
      }
      return AutofillFields(structure.activityComponent.packageName, page?.domain, page?.scheme, user?.id, pass?.id, user?.text, saved?.text, cardFields, typedCard)
    }

    /** What a list or date field holds, as text: the option chosen, or the date as `MM/YYYY`. */
    private fun chosen(node: ViewNode, options: List<String>): String? {
      val value = node.autofillValue ?: return null
      return when {
        value.isList -> options.getOrNull(value.listValue)
        value.isDate -> java.time.Instant.ofEpochMilli(value.dateValue).atZone(java.time.ZoneOffset.UTC).let { "%02d/%d".format(it.monthValue, it.year) }
        value.isText -> value.textValue.toString()
        else -> null
      }
    }

    /** The card hints Android and `autocomplete` give, lower case, by what they ask for. */
    private val CARD_HINTS = mapOf(
      CardKind.NUMBER to setOf(View.AUTOFILL_HINT_CREDIT_CARD_NUMBER, "cc-number"),
      CardKind.CODE to setOf(View.AUTOFILL_HINT_CREDIT_CARD_SECURITY_CODE, "cc-csc"),
      CardKind.EXPIRY to setOf(View.AUTOFILL_HINT_CREDIT_CARD_EXPIRATION_DATE, "cc-exp"),
      CardKind.MONTH to setOf(View.AUTOFILL_HINT_CREDIT_CARD_EXPIRATION_MONTH, "cc-exp-month"),
      CardKind.YEAR to setOf(View.AUTOFILL_HINT_CREDIT_CARD_EXPIRATION_YEAR, "cc-exp-year"),
      CardKind.NAME to setOf("cc-name", "creditcardholdername"),
    ).mapValues { (_, hints) -> hints.map { it.lowercase() }.toSet() }

    /** Words in a field's id, name or hint that say which card value it takes, the most specific first. */
    private val CARD_WORDS = listOf(
      CardKind.MONTH to listOf("exp-month", "expmonth", "exp_month", "expirymonth", "expiry-month", "expiry_month", "expiresmonth", "expires-month", "expires_month", "ccmonth", "cc-month", "cc_month"),
      CardKind.YEAR to listOf("exp-year", "expyear", "exp_year", "expiryyear", "expiry-year", "expiry_year", "expiresyear", "expires-year", "expires_year", "ccyear", "cc-year", "cc_year"),
      CardKind.NUMBER to listOf("cardnumber", "card-number", "card_number", "ccnumber", "cc-number", "cc_number", "номер картки", "номер карты"),
      CardKind.CODE to listOf("cvv", "cvc", "csc", "securitycode", "security-code", "security_code"),
      CardKind.EXPIRY to listOf("expiry", "expiration", "exp-date", "expdate", "exp_date", "mm/yy", "термін дії"),
      CardKind.NAME to listOf("cardholder", "card-holder", "card_holder", "nameoncard", "name-on-card", "cc-name", "cardname", "card-name", "card_name", "власник картки"),
    )

    /** The card values a list or a date field is read for: the expiry's. */
    private val EXPIRY_KINDS = setOf(CardKind.EXPIRY, CardKind.MONTH, CardKind.YEAR)

    /**
     * What card value a field takes, by its hints or `autocomplete`, else words
     * in its id, name or hint, else (a list) by its `options` alone: months or
     * years. Null for none.
     */
    private fun cardKind(node: ViewNode, options: List<String>): CardKind? {
      val attributes = node.htmlInfo?.attributes.orEmpty().filter { it.first != null }.associate { it.first.lowercase() to it.second.orEmpty().lowercase() }
      val hints = node.autofillHints?.mapNotNull { it?.lowercase() }.orEmpty() + listOfNotNull(attributes["autocomplete"])
      CARD_HINTS.entries.firstOrNull { (_, known) -> hints.any { it in known } }?.let { return it.key }
      val words = listOfNotNull(node.idEntry, node.hint, attributes["name"], attributes["id"]).joinToString(" ").lowercase()
      CARD_WORDS.firstOrNull { (_, list) -> list.any { it in words } }?.let { return it.first }
      return when (CardValues.listKind(options)) {
        CardValues.ListOf.MONTH -> CardKind.MONTH
        CardValues.ListOf.YEAR -> CardKind.YEAR
        null -> null
      }
    }

    /** The page a field is on: its web domain and scheme. */
    private data class Page(val domain: String, val scheme: String?)

    /** A text field read, with what it is for a login and whether it takes a new password. */
    private data class Text(val field: Field, val kind: Kind, val isNew: Boolean)

    /** A field read: on which page, its text, and for an expiry's field its autofill type and a list's options. */
    private class Field(val id: AutofillId, val page: Page?, val text: String?, val type: Int = View.AUTOFILL_TYPE_TEXT, val options: List<String> = emptyList())

    // Lower case, as the hints are compared (`emailAddress` among them).
    private val PASSWORD_HINTS = setOf(View.AUTOFILL_HINT_PASSWORD, "current-password", "new-password").map { it.lowercase() }.toSet()
    private val USERNAME_HINTS = setOf(View.AUTOFILL_HINT_USERNAME, View.AUTOFILL_HINT_EMAIL_ADDRESS, "email", "username").map { it.lowercase() }.toSet()
    private val PASSWORD_WORDS = listOf("password", "passwd", "pwd", "пароль")
    private val USERNAME_WORDS = listOf("user", "login", "email", "e-mail", "логін", "пошта")

    /** A password field for a new password: by its autofill hint or its `autocomplete`. */
    private fun isNewPassword(node: ViewNode): Boolean =
      node.autofillHints.orEmpty().any { it?.equals("new-password", ignoreCase = true) == true } ||
        node.htmlInfo?.attributes.orEmpty().any { it.first.equals("autocomplete", ignoreCase = true) && it.second.equals("new-password", ignoreCase = true) }

    private fun kind(node: ViewNode): Kind {
      val hints = node.autofillHints?.mapNotNull { it?.lowercase() }.orEmpty()
      if (hints.any { it in PASSWORD_HINTS }) return Kind.PASSWORD
      if (hints.any { it in USERNAME_HINTS }) return Kind.USERNAME
      // A page's input: its type, then its name and id.
      val html = node.htmlInfo?.takeIf { it.tag.equals("input", ignoreCase = true) }
      val attributes = html?.attributes.orEmpty().filter { it.first != null }.associate { it.first.lowercase() to it.second.orEmpty().lowercase() }
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
