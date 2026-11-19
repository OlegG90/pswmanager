//! Cards as an autofill category (#412): whether an entry is a card, its
//! brand, the redacted summary an offer shows, and the fields a form takes.
//! The full number and the CVV stay here: an offer carries the last four only,
//! and the one picked answer is wiped once it is handed over.

use crate::modules;
use keepass::db::EntryRef;
use zeroize::Zeroizing;

/// The names a card's number is kept under, the most specific first: the
/// importers' and PswManager's starter template (`Card number`), Keepass2Android's
/// template (`Number`, named in the language its UI had when the template was
/// made: `Номер`, `Nummer`), and the autofill hint K2A saves under (`cc-number`).
/// A generic name like `Number` counts only when its value passes Luhn.
pub const NUMBER: &[&str] = &["Card number", "Kartennummer", "Номер картки", "Номер карты", "cc-number", "Number", "Nummer", "Номер"];
/// The card's security code: the importers' `Card code`, the starter
/// template's and K2A's `CVV` (German K2A: `Sicherheitscode`), the hint `cc-csc`.
pub const CODE: &[&str] = &["Card code", "CVV", "CVC", "CVV2", "Security code", "Sicherheitscode", "cc-csc"];
/// The cardholder: the importers' `Card cardholder name`, the starter
/// template's `Cardholder`, K2A's `Card holder` (`Власник картки`,
/// `Владелец карты`, `Inhaber`), the hint `cc-name`.
pub const NAME: &[&str] = &[
    "Card cardholder name", "Cardholder", "Card holder", "Cardholder name", "Karteninhaber", "Власник картки", "Владелец карты", "Inhaber", "cc-name",
];
/// The expiry as text: the importers' `Card expiry`, the starter template's
/// `Expiry`, the hint `cc-exp`, and the names other apps (SafeInCloud's
/// cards, kept as they were by `sic2kdbx`) give it. With none of them, [fill]
/// takes a field whose value looks like one ([looks_like_expiry]), then the
/// entry's own expiry date, where K2A's template keeps it.
pub const EXPIRY: &[&str] = &[
    "Card expiry", "Expiry", "Expiry date", "Expiration date", "Expiration", "Expires", "Exp", "Valid thru", "Valid through", "Valid until",
    "Ablaufdatum", "Gültig bis", "Термін дії", "Строк дії", "Дійсна до", "Діє до", "Срок действия", "Действует до", "cc-exp",
];

/// Whether `value` reads as a card's expiry: a month and a year, `12/30`,
/// `1/2030`, `12-30` or `12.30`, and nothing else.
pub fn looks_like_expiry(value: &str) -> bool {
    static EXPIRY_VALUE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"^\s*(0?[1-9]|1[0-2])\s*[/.\-]\s*(\d{2}|\d{4})\s*$").unwrap());
    EXPIRY_VALUE.is_match(value)
}

/// The value of the first of `names` the entry keeps, matched without regard
/// to case or surrounding spaces, as another app may have written it. An
/// empty field (a template's left blank) counts as none, so the next name,
/// or another source, is tried.
fn field<'e>(entry: &'e EntryRef<'_>, names: &[&str]) -> Option<&'e str> {
    names.iter().find_map(|wanted| {
        let wanted = wanted.to_lowercase();
        entry.fields.iter().find(|(name, value)| name.trim().to_lowercase() == wanted && !value.get().trim().is_empty()).map(|(_, value)| value.get().as_str())
    })
}

/// The card number the entry keeps: the first of [NUMBER] whose value
/// passes Luhn and is long enough, so a generic `Number` of another kind
/// (an ID card's) is not taken for one.
fn number<'e>(entry: &'e EntryRef<'_>) -> Option<&'e str> {
    NUMBER.iter().filter_map(|name| field(entry, &[name])).find(|number| luhn(number) && digits(number).len() >= 12)
}

/// Whether the entry is a card: it keeps a number that passes Luhn.
pub fn qualifies(entry: &EntryRef<'_>) -> bool {
    number(entry).is_some()
}

/// The card's brand, from its number's first digits.
pub fn brand(number: &str) -> &'static str {
    let digits = digits(number);
    let first = |n: usize| digits.get(..n).and_then(|d| d.parse::<u32>().ok());
    match (first(1), first(2), first(4)) {
        (Some(4), _, _) => "Visa",
        (_, Some(34 | 37), _) => "Amex",
        (_, Some(51..=55), _) | (_, _, Some(2221..=2720)) => "Mastercard",
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
    pub name: Zeroizing<String>,
    pub number: Zeroizing<String>,
    pub expiry: Zeroizing<String>,
    pub code: Zeroizing<String>,
}

/// The redacted offer for `entry`, when it is a card.
pub fn offer(entry: &EntryRef<'_>) -> Option<Offer> {
    number(entry).map(|number| Offer { summary: summary(number) })
}

/// The card's fields, when `entry` is a card. With no expiry field, the
/// entry's own expiry date (KeePass's *Expires*, where Keepass2Android's
/// template keeps it) gives it, as `MM/YY`.
pub fn fill(entry: &EntryRef<'_>) -> Option<Fill> {
    let number = number(entry)?;
    // By its name, else the one field whose value reads as an expiry, else the
    // entry's own expiry date.
    let expiry = field(entry, EXPIRY)
        .or_else(|| entry.fields.iter().map(|(_, value)| value.get().as_str()).find(|value| looks_like_expiry(value)))
        .map(str::to_string)
        .or_else(|| (entry.times.expires == Some(true)).then(|| entry.times.expiry.format("%m/%y").to_string()))
        .unwrap_or_default();
    Some(Fill {
        name: Zeroizing::new(field(entry, NAME).unwrap_or("").to_string()),
        number: Zeroizing::new(number.to_string()),
        expiry: Zeroizing::new(expiry),
        code: Zeroizing::new(field(entry, CODE).unwrap_or("").to_string()),
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

    fn hint(&self) -> &'static str {
        "Fills in card details; the offer shows the brand and last four only, never the full number or CVV."
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use keepass::Database;

    /// An entry with `fields`, and an expiry date when `expires` names one.
    fn with(fields: &[(&str, &str)], expires: Option<&str>) -> EntryRef<'static> {
        let db: &'static mut Database = Box::leak(Box::new(Database::new()));
        {
            let mut root = db.root_mut();
            let mut entry = root.add_entry();
            for &(name, value) in fields {
                entry.set_protected(name, value);
            }
            if let Some(at) = expires {
                entry.times.expires = Some(true);
                entry.times.expiry = chrono::NaiveDateTime::parse_from_str(at, "%Y-%m-%d %H:%M").unwrap();
            }
        }
        db.iter_all_entries().next().unwrap()
    }

    /// An entry as the importers write a card, with `number` and `code`.
    fn entry(number: &str, code: &str) -> EntryRef<'static> {
        with(&[("Card number", number), ("Card code", code), ("Card cardholder name", "Erin Example"), ("Card expiry", "12/30")], None)
    }

    #[test]
    fn the_starter_template_card_is_read_whole() {
        let card = with(&[("Cardholder", "Erin Example"), ("Card number", "4111 1111 1111 1111"), ("Expiry", "12/30"), ("CVV", "123"), ("PIN", "0000")], None);
        let fields = fill(&card).unwrap();
        assert_eq!((fields.name.as_str(), fields.expiry.as_str(), fields.code.as_str()), ("Erin Example", "12/30", "123"));
        assert_eq!(offer(&card).unwrap().summary, "Visa •• 1111");
    }

    #[test]
    fn a_keepass2android_card_is_read_in_its_language_with_the_entrys_expiry() {
        // English, Ukrainian and German K2A templates: the expiry is the entry's own date.
        for (number, code, holder) in [("Number", "CVV", "Card holder"), ("Номер", "CVV", "Власник картки"), ("Nummer", "Sicherheitscode", "Inhaber")] {
            let card = with(&[(number, "5555555555554444"), (code, "321"), ("PIN", "0000"), (holder, "Erin Example")], Some("2030-12-31 23:59"));
            let fields = fill(&card).unwrap_or_else(|| panic!("{number} is a card"));
            assert_eq!(fields.number.as_str(), "5555555555554444");
            assert_eq!((fields.name.as_str(), fields.expiry.as_str(), fields.code.as_str()), ("Erin Example", "12/30", "321"), "{number}");
        }
        // As K2A saves a card from a form, under the autofill hints; any case.
        let saved = with(&[("cc-number", "4111111111111111"), ("CC-CSC", "123")], None);
        assert_eq!(fill(&saved).unwrap().code.as_str(), "123");
    }

    #[test]
    fn the_expiry_is_found_under_other_apps_names_or_by_its_value() {
        // SafeInCloud's card, as sic2kdbx keeps its field names.
        let named = with(&[("Number", "4111111111111111"), ("Owner", "Erin Example"), ("Expires", "07/31"), ("CVV", "123")], None);
        assert_eq!(fill(&named).unwrap().expiry.as_str(), "07/31");
        let unnamed = with(&[("Number", "4111111111111111"), ("Bank", "Example Bank"), ("Some date", "1/2031"), ("CVV", "123")], None);
        assert_eq!(fill(&unnamed).unwrap().expiry.as_str(), "1/2031");
        // A field named so wins over a value that only looks like one; the entry's date comes last.
        let both = with(&[("Number", "4111111111111111"), ("Valid thru", "12/30"), ("Note", "3/29")], Some("2032-01-01 00:00"));
        assert_eq!(fill(&both).unwrap().expiry.as_str(), "12/30");
        let dated = with(&[("Number", "4111111111111111"), ("Note", "not a date")], Some("2032-01-01 00:00"));
        assert_eq!(fill(&dated).unwrap().expiry.as_str(), "01/32");
        // The starter template's Expiry left blank, the entry's own date set.
        let template = with(&[("Cardholder", "Erin Example"), ("Card number", "4111111111111111"), ("Expiry", ""), ("CVV", "123"), ("PIN", "")], Some("2030-12-31 00:00"));
        assert_eq!(fill(&template).unwrap().expiry.as_str(), "12/30");
        for value in ["12/30", "1/2030", "07-31", " 12.30 "] {
            assert!(looks_like_expiry(value), "{value}");
        }
        for value in ["13/30", "4111111111111111", "12/3", "123", "12/30 and more"] {
            assert!(!looks_like_expiry(value), "{value}");
        }
    }

    #[test]
    fn a_generic_number_that_fails_luhn_is_no_card() {
        // K2A's ID card template keeps its number under `Number` too.
        assert!(!qualifies(&with(&[("Number", "AB1234567")], None)));
        assert!(!qualifies(&with(&[("Number", "123456789012")], None)));
        // A card number elsewhere still wins over an ID's generic one.
        let both = with(&[("Number", "123456789012"), ("Card number", "4111111111111111")], None);
        assert_eq!(fill(&both).unwrap().number.as_str(), "4111111111111111");
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
        assert_eq!(brand("2223 0031 2200 3222"), "Mastercard");
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
