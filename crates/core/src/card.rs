//! Cards as an autofill category (#412): whether an entry is a card, its
//! brand, the redacted summary an offer shows, and the fields a form takes.
//! The full number and the CVV stay here: an offer carries the last four only,
//! and the one picked answer is wiped once it is handed over.

use crate::modules;
use crate::vault::{EntryChoice, LoginError, LoginPlace, Vault};
use keepass::db::EntryRef;
use zeroize::Zeroizing;

/// The field names a card entry keeps, as the importers write them.
pub const NUMBER: &str = "Card number";
pub const CODE: &str = "Card code";
pub const NAME: &str = "Card cardholder name";
pub const EXPIRY: &str = "Card expiry";

/// Whether the entry is a card: it keeps a number that passes Luhn.
pub fn qualifies(entry: &EntryRef<'_>) -> bool {
    entry.get(NUMBER).is_some_and(|number| luhn(number) && digits(number).len() >= 12)
}

/// The card's brand, from its number's first digits.
pub fn brand(number: &str) -> &'static str {
    let two = number.chars().filter(|c| c.is_ascii_digit()).take(2).collect::<String>().parse::<u32>().ok();
    match (number.chars().find(|c| c.is_ascii_digit()), two) {
        (Some('4'), _) => "Visa",
        (Some('3'), Some(34 | 37)) => "Amex",
        (_, Some(51..=55)) => "Mastercard",
        _ => "Card",
    }
}

/// The digits of `number`, without spaces or other separators.
fn digits(number: &str) -> String {
    number.chars().filter(|c| c.is_ascii_digit()).collect()
}

/// Whether `number` (digits, maybe with separators) passes the Luhn check.
pub fn luhn(number: &str) -> bool {
    let digits: Vec<u32> = number.chars().filter_map(|c| c.to_digit(10)).collect();
    if digits.len() < 2 {
        return false;
    }
    let sum: u32 = digits.iter().rev().enumerate().map(|(i, digit)| {
        if i % 2 == 1 {
            let doubled = digit * 2;
            if doubled > 9 { doubled - 9 } else { doubled }
        } else {
            *digit
        }
    }).sum();
    sum.is_multiple_of(10)
}

/// The offer's redacted summary: brand and last four, never the full number.
pub fn summary(number: &str) -> String {
    let digits = digits(number);
    let last4 = &digits[digits.len().saturating_sub(4)..];
    format!("{} •• {}", brand(number), last4)
}

/// A card's redacted offer.
pub struct Offer {
    pub summary: String,
}

/// A card's fields for a form, from memory, wiped on drop.
pub struct Fill {
    pub name: String,
    pub number: Zeroizing<String>,
    pub expiry: String,
    pub code: Zeroizing<String>,
}

/// The redacted offer for `entry`, when it is a card.
pub fn offer(entry: &EntryRef<'_>) -> Option<Offer> {
    qualifies(entry).then(|| Offer { summary: summary(entry.get(NUMBER).unwrap_or("")) })
}

/// The card's fields, when `entry` is a card.
pub fn fill(entry: &EntryRef<'_>) -> Option<Fill> {
    if !qualifies(entry) {
        return None;
    }
    Some(Fill {
        name: entry.get(NAME).unwrap_or("").to_string(),
        number: Zeroizing::new(entry.get(NUMBER).unwrap_or("").to_string()),
        expiry: entry.get(EXPIRY).unwrap_or("").to_string(),
        code: Zeroizing::new(entry.get(CODE).unwrap_or("").to_string()),
    })
}

/// The card category, the second [modules::CategoryFiller] (#412). It owns the
/// card decisions; the [crate::autofill] router owns the policy around them.
pub(crate) struct CardFiller;

impl modules::CategoryFiller for CardFiller {
    fn id(&self) -> &'static str {
        "card"
    }

    fn label(&self) -> &'static str {
        "Cards"
    }

    fn qualifies(&self, entry: EntryRef<'_>) -> bool {
        qualifies(&entry)
    }

    fn card_offer(&self, entry: EntryRef<'_>) -> Option<Offer> {
        offer(&entry)
    }

    fn card_fill(&self, entry: EntryRef<'_>) -> Option<Fill> {
        fill(&entry)
    }

    // Cards fill no login: these are never called for the card category.
    fn offers(&self, _vault: &Vault, _place: &LoginPlace) -> Vec<EntryChoice> {
        Vec::new()
    }

    fn choices(&self, _vault: &Vault, _place: &LoginPlace) -> Vec<EntryChoice> {
        Vec::new()
    }

    fn candidates(&self, _vault: &Vault, _query: &str) -> Vec<EntryChoice> {
        Vec::new()
    }

    fn fills(&self, _vault: &Vault, _id: &str, _place: &LoginPlace) -> Result<(Zeroizing<String>, Zeroizing<String>), LoginError> {
        Err("Cards fill no login".into())
    }

    fn saves(&self, _vault: &mut Vault, _id: Option<&str>, _place: &LoginPlace, _confirmed: bool, _title: &str, _username: &str, _password: &str) -> Result<String, LoginError> {
        Err("Cards save no login".into())
    }

    fn links(&self, _vault: &mut Vault, _id: &str, _place: &LoginPlace, _confirmed: bool) -> Result<(), LoginError> {
        Err("Cards link no login".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use keepass::Database;

    /// An entry with the card fields `number` (and, optionally, `code`).
    fn entry(number: &str, code: &str) -> EntryRef<'static> {
        let db: &'static mut Database = Box::leak(Box::new(Database::new()));
        {
            let mut root = db.root_mut();
            let mut entry = root.add_entry();
            entry.set_protected(NUMBER, number);
            entry.set_protected(CODE, code);
            entry.set_unprotected(NAME, "Erin Example");
            entry.set_unprotected(EXPIRY, "12/30");
        }
        db.iter_all_entries().next().unwrap()
    }

    #[test]
    fn a_number_that_passes_luhn_is_a_card() {
        assert!(luhn("4111111111111111"));
        assert!(luhn("5555 5555 5555 4444"));
        assert!(!luhn("4111111111111112"));
        assert!(qualifies(&entry("4111111111111111", "123")));
        assert!(!qualifies(&entry("4111111111111112", "123")));
    }

    #[test]
    fn the_brand_and_summary_are_redacted() {
        assert_eq!(brand("4111111111111111"), "Visa");
        assert_eq!(brand("5555555555554444"), "Mastercard");
        assert_eq!(brand("378282246310005"), "Amex");
        assert_eq!(summary("4111111111111111"), "Visa •• 1111");
        assert_eq!(summary("5555 5555 5555 4444"), "Mastercard •• 4444");
        assert!(!summary("4111111111111111").contains("4111111111111111"), "the full number never appears");
    }

    #[test]
    fn the_fields_are_read_for_a_card_only() {
        let fields = fill(&entry("4111111111111111", "123")).unwrap();
        assert_eq!((fields.name.as_str(), fields.expiry.as_str()), ("Erin Example", "12/30"));
        assert_eq!(fields.number.as_str(), "4111111111111111");
        assert_eq!(fields.code.as_str(), "123");
        assert!(fill(&entry("4111111111111112", "123")).is_none());
    }
}
