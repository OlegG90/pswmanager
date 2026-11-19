package io.github.olegg90.pswmanager

import android.view.autofill.AutofillId

/**
 * A card's fields on a form (#412): its number, and those of the code, the
 * expiry (one field, or its month and year), and the cardholder's name, as
 * the form has them. Read by [AutofillFields]; filled by [AutofillCardActivity].
 * The expiry's fields keep what kind of field each is ([CardField]): a page
 * often asks for the month and the year in lists.
 */
class CardFields(
  val number: AutofillId,
  val code: AutofillId?,
  val expiry: CardField?,
  val month: CardField?,
  val year: CardField?,
  val name: AutofillId?,
) {
  /** Every field the card fills, the number first. */
  val ids: List<AutofillId> get() = listOfNotNull(number, code, expiry?.id, month?.id, year?.id, name)
}

/**
 * One of the expiry's fields: its id, its autofill type (`View.AUTOFILL_TYPE_TEXT`,
 * `_LIST` or `_DATE`), and a list's options as it shows them (empty otherwise).
 */
class CardField(val id: AutofillId, val type: Int, val options: List<String>)
