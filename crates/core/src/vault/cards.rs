//! The cards the phone's autofill offers and fills (#412): which entries in
//! use are cards, each offered by its title and redacted summary, and the
//! fields of the one picked. What makes an entry a card is [crate::card]'s.

use super::*;

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
}
