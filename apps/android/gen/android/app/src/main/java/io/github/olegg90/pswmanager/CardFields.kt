package io.github.olegg90.pswmanager

import android.view.autofill.AutofillId

/**
 * A card's fields on a form (#412): its number, and those of the code, the
 * expiry (one field, or its month and year), and the cardholder's name, as
 * the form has them. Read by [AutofillFields]; filled by [AutofillCardActivity].
 */
class CardFields(
  val number: AutofillId,
  val code: AutofillId?,
  val expiry: AutofillId?,
  val month: AutofillId?,
  val year: AutofillId?,
  val name: AutofillId?,
) {
  /** Every field the card fills, the number first. */
  val ids: List<AutofillId> get() = listOfNotNull(number, code, expiry, month, year, name)
}
