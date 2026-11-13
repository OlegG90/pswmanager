//! Auto-Type (#272): the entries that fit a window, and what is typed for one.

use super::*;

impl Vault {
    /// The entries Auto-Type offers for `window` (#272), by title: those with
    /// Auto-Type on that fit the page a browser shows as in KeePassXC (never
    /// by a browser's title, which the page chooses), or that fit another
    /// app's window title.
    pub fn auto_type_matches(&self, window: &autotype::Window) -> Vec<EntryChoice> {
        let mut matches: Vec<EntryChoice> = self
            .visible_entries()
            .filter(|e| autotype::enabled(e))
            .filter(|e| match &window.page {
                Some(page) => page.fits_entry(e),
                None => !window.browser && autotype::fits_title(e, &window.title),
            })
            .map(|e| EntryChoice::of(&e, e.get(fields::USERNAME)))
            .collect();
        matches.sort_by_cached_key(|c| (c.title.to_lowercase(), c.username.to_lowercase()));
        matches
    }

    /// What typing `typed` of entry `id` into a window titled `title` takes:
    /// only an entry in use, with Auto-Type on.
    pub fn auto_type_steps(&self, id: &str, title: &str, typed: autotype::Typed) -> Result<Vec<autotype::Step>, String> {
        let entry = self.entry(id).filter(|e| kind(e) == Kind::Entry).ok_or(NOT_FOUND)?;
        if !autotype::enabled(&entry) {
            return Err("Auto-Type is off for this entry".into());
        }
        let field = |name: &str| Zeroizing::new(entry.get(name).unwrap_or_default().to_string());
        autotype::steps(&typed.sequence(&entry, title), field, || self.totp(id).map(|code| code.map(|c| c.code)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::tests::*;

    #[test]
    fn auto_type_finds_a_browser_s_entries_by_site_and_an_app_s_by_title() {
        let vault = Vault::from_database(sample());
        let window = |title: &str, browser: bool, site: Option<&str>| autotype::Window { title: title.into(), browser, page: site.and_then(browser::Page::of_address) };
        let titles = |w: autotype::Window| vault.auto_type_matches(&w).into_iter().map(|c| c.title).collect::<Vec<_>>();
        assert_eq!(titles(window("Inbox — Mail client", false, None)), ["Mail"]);
        assert_eq!(titles(window("Sign in", true, Some("www.example.com"))), ["Mail"]);
        assert!(titles(window("Mail — phishing page", true, Some("evil.example"))).is_empty(), "not by a browser's title");
        assert!(titles(window("Mail", true, None)).is_empty(), "a browser whose site is unknown");
        assert!(titles(window("Deleted", false, None)).is_empty(), "not from the recycle bin");

        let mail = in_use(&vault).into_iter().find(|e| e.title == "Mail").unwrap().id;
        let steps = vault.auto_type_steps(&mail, "Mail", autotype::Typed::Sequence).unwrap();
        assert_eq!(steps.len(), 4);
        assert_eq!(steps[2], autotype::Step::Text(Zeroizing::new("s3cret".into())));
        let pin = vault.auto_type_steps(&mail, "Mail", autotype::Typed::Password).unwrap();
        assert_eq!(pin, [autotype::Step::Text(Zeroizing::new("s3cret".into()))]);
        assert!(vault.auto_type_steps(&mail, "Mail", autotype::Typed::Totp).is_err(), "no TOTP secret");
    }
}
