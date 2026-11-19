//! The cards the phone's autofill offers, fills and saves (#412): which
//! entries in use are cards, each offered by its title and redacted summary,
//! the fields of the one picked, and a card typed into a form kept as a new
//! entry or into one of them. What makes an entry a card is [crate::card]'s.

use super::*;
use crate::card;
use crate::edit::{FieldData, IconChoice};

/// A card typed into a form, to keep: its values as the form had them.
pub struct TypedCard<'a> {
    pub name: &'a str,
    pub number: &'a str,
    /// As typed or picked: `12/30`, `12/2030`; empty when the form had none.
    pub expiry: &'a str,
    pub code: &'a str,
}

/// Where a typed card can go: a card in use, `same` when it keeps the same
/// number (it is offered first). Never the number or the code.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CardSaveChoice {
    #[serde(flatten)]
    pub card: CardChoice,
    pub same: bool,
}

/// The starter template's names (`edit.rs`), which a new card's fields take.
const NEW_NUMBER: &str = "Card number";
const NEW_CODE: &str = "CVV";
const NEW_NAME: &str = "Cardholder";
const NEW_EXPIRY: &str = "Expiry";
/// The starter template's icon for a card (KeePass's standard icon 66).
const CARD_ICON: usize = 66;

/// A card as an autofill offer lists it: never the full number or the code.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CardChoice {
    /// The entry's id.
    pub id: String,
    pub title: String,
    /// The brand and the last four digits ("Visa •• 1111").
    pub summary: String,
}

impl Vault {
    /// The entries in use that are cards, for an autofill offer: titled and
    /// summarized, never the full number or the code.
    pub fn cards(&self) -> Vec<CardChoice> {
        self.visible_entries()
            .filter(|e| kind(e) == Kind::Entry)
            .filter_map(|e| {
                let offer = crate::card::offer(&e)?;
                Some(CardChoice { id: e.id().uuid().to_string(), title: e.get(fields::TITLE).unwrap_or_default().to_string(), summary: offer.summary })
            })
            .collect()
    }

    /// Card `id`'s fields for a form; the number and the code are wiped when
    /// dropped. Refused for an entry that is not a card in use.
    pub fn card(&self, id: &str) -> Result<crate::card::Fill, String> {
        let entry = self.entry(id).filter(|e| kind(e) == Kind::Entry).ok_or(NOT_FOUND)?;
        crate::card::fill(&entry).ok_or_else(|| "This entry is not a card".into())
    }

    /// The cards a card typed with `number` can go into, the one keeping the
    /// same number first.
    pub fn cards_for_save(&self, number: &str) -> Vec<CardSaveChoice> {
        let typed = card::digits(number);
        let mut choices: Vec<CardSaveChoice> = self
            .visible_entries()
            .filter(|e| kind(e) == Kind::Entry)
            .filter_map(|e| {
                let kept = card::number(&e)?;
                let choice = CardChoice { id: e.id().uuid().to_string(), title: e.get(fields::TITLE).unwrap_or_default().to_string(), summary: card::summary(kept) };
                Some(CardSaveChoice { card: choice, same: !typed.is_empty() && card::digits(kept) == typed })
            })
            .collect();
        choices.sort_by_key(|c| !c.same);
        choices
    }

    /// Keeps a card typed into a form: into card `id`, its values going to
    /// the fields it keeps them in (whatever their names) and the old version
    /// to history, or as a new entry titled by its brand and last four, with
    /// the starter template's names and icon. The number and the code are
    /// protected. Answers the entry's id and title.
    pub fn save_card(&mut self, id: Option<&str>, typed: &TypedCard) -> Result<(String, String), String> {
        if !card::is_number(typed.number) {
            return Err("This is not a card number".into());
        }
        let (base, mut data) = match id {
            Some(id) => {
                self.card(id)?;
                let data = self.edit_data(id).ok_or(NOT_FOUND)?;
                (Some(data.clone()), data)
            }
            None => {
                let mut data = EntryData::default();
                data.title = card::summary(typed.number);
                data.icon = IconChoice::Builtin { id: CARD_ICON };
                (None, data)
            }
        };
        put(&mut data.fields, card::NUMBER, NEW_NUMBER, typed.number, true);
        put(&mut data.fields, card::CODE, NEW_CODE, typed.code, true);
        put(&mut data.fields, card::NAME, NEW_NAME, typed.name, false);
        if let Some((month, year)) = card::month_year(typed.expiry) {
            let short = format!("{month:02}/{:02}", year % 100);
            // Where the entry keeps it: a field, else its own expiry date (as
            // Keepass2Android's template does), else a new field.
            if find(&data.fields, card::EXPIRY).is_none() && data.expires.is_some() {
                data.expires = Some(last_day(month, year)?);
            } else {
                put(&mut data.fields, card::EXPIRY, NEW_EXPIRY, &short, false);
            }
        }
        let title = data.title.clone();
        let (id, _) = self.save_entry(id, base.as_ref(), &data, false, &[])?;
        Ok((id, title))
    }
}

/// The first of `fields` named one of `names` (in their order), without regard to case.
fn find(fields: &[FieldData], names: &[&str]) -> Option<usize> {
    names.iter().find_map(|wanted| {
        let wanted = wanted.to_lowercase();
        fields.iter().position(|f| f.name.trim().to_lowercase() == wanted)
    })
}

/// Sets `value` in the field named one of `names`, else in a new field
/// `new`; nothing for an empty value (the form did not have it).
fn put(fields: &mut Vec<FieldData>, names: &[&str], new: &str, value: &str, protected: bool) {
    if value.trim().is_empty() {
        return;
    }
    match find(fields, names) {
        Some(at) => fields[at].value = value.to_string(),
        None => fields.push(FieldData { name: new.to_string(), value: value.to_string(), protected }),
    }
}

/// The last day of `month` in `year`, as an entry's expiry (UTC, RFC 3339).
fn last_day(month: u32, year: i32) -> Result<String, String> {
    let next = if month == 12 { chrono::NaiveDate::from_ymd_opt(year + 1, 1, 1) } else { chrono::NaiveDate::from_ymd_opt(year, month + 1, 1) };
    let last = next.and_then(|d| d.pred_opt()).ok_or("The expiry is not a date")?;
    Ok(format!("{}T00:00:00Z", last.format("%Y-%m-%d")))
}
