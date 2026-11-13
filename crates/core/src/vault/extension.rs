//! What the browser extension asks of the open database (#273): its
//! associations, a site's logins, the groups, and saving a login.

use super::*;

impl Vault {
    /// How the browser extension knows this database (#273, [browser::database_hash]).
    pub fn browser_hash(&self) -> String {
        browser::database_hash(&self.db)
    }

    /// The identification key the extension was associated with as `name`.
    pub fn browser_key(&self, name: &str) -> Option<String> {
        browser::key(&self.db, name)
    }

    /// The extensions associated with this database, by name.
    pub fn browser_associations(&self) -> Vec<browser::Association> {
        browser::associations(&self.db)
    }

    /// Keeps an extension's identification `key` as `name` (replacing one of
    /// that name) and saves the file; KeePassXC reads it too.
    pub fn browser_associate(&mut self, name: &str, key: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("The connection needs a name".into());
        }
        self.change(|db, _| {
            browser::associate(db, name, key);
            Ok(())
        })
    }

    /// Removes the association `name` and saves the file.
    pub fn browser_forget(&mut self, name: &str) -> Result<(), String> {
        self.change(|db, _| {
            browser::forget(db, name);
            Ok(())
        })
    }

    /// The logins the extension gets for a page at `url`: the entries in use
    /// that fit it as in KeePassXC ([browser::Page]) and that KeePassXC's
    /// settings do not keep from it.
    pub fn browser_logins(&self, url: &str) -> Vec<browser::Login> {
        let Some(page) = browser::Page::of_tab(url) else { return Vec::new() };
        self.visible_entries().filter(|e| page.fits_entry(e) && browser::offered(e, page.host())).map(|e| browser::Login::of(&e)).collect()
    }

    /// The current TOTP code of entry `id` (any UUID form), when it is in use,
    /// not hidden from the extension ([browser::hidden]) and has one.
    pub fn browser_totp(&self, id: &str) -> Option<String> {
        self.entry(id).filter(|e| kind(e) == Kind::Entry && !browser::hidden(e))?;
        self.totp(id).ok().flatten().map(|c| c.code)
    }

    /// The database's groups, for the extension to pick where a login goes.
    pub fn browser_groups(&self) -> browser::Group {
        browser::groups(&self.db, &self.hidden_groups())
    }

    /// Creates the group at `path` (`Parent/Child`; the parts already there
    /// are used) and saves the file; its name and UUID.
    pub fn browser_create_group(&mut self, path: &str) -> Result<(String, String), String> {
        let path = browser::split_path(path);
        let Some(name) = path.last().cloned() else { return Err("The group needs a name".into()) };
        let id = self.change(|db, hidden| edit::group_at(db, &path, hidden))?;
        Ok((name, id.uuid().simple().to_string()))
    }

    /// Keeps a login the extension offers for a page at `url`: the user name
    /// and password of entry `id`, which must be in use, fit the page and be
    /// one the extension may have there ([browser::offered]), or a
    /// new entry titled after the page's host with the page's address, in
    /// the group with UUID `group` (none or unknown: [browser::NEW_LOGINS_GROUP]).
    /// Saved like an edit (the old version goes to history). Returns its id.
    pub fn browser_save_login(&mut self, id: Option<&str>, url: &str, username: &str, password: &str, group: Option<&str>) -> Result<String, String> {
        let page = browser::Page::of_tab(url).ok_or("Logins are saved for web pages only")?;
        if password.is_empty() {
            return Err("There is no password to save".into());
        }
        let (base, mut data) = match id {
            Some(id) => {
                self.browser_entry(id, &page)?;
                let data = self.edit_data(id).ok_or(NOT_FOUND)?;
                (Some(data.clone()), data)
            }
            None => {
                let mut data = EntryData::default();
                data.title = page.host().to_string();
                data.url = url.to_string();
                data.group = group
                    .and_then(|g| browser::group_by_uuid(&self.db, g))
                    .map_or_else(|| vec![browser::NEW_LOGINS_GROUP.to_string()], |g| edit::path_of(&self.db, g));
                (None, data)
            }
        };
        data.username = username.to_string();
        data.password = password.to_string();
        self.save_entry(id, base.as_ref(), &data, false, &[]).map(|(id, _)| id)
    }

    /// The title and user name of entry `id`, for the user to confirm that a
    /// login the extension offers for a page at `url` replaces its own; an
    /// error when [Vault::browser_save_login] would refuse it, so the user is
    /// not asked about an entry the extension may not have.
    pub fn browser_replaced(&self, id: &str, url: &str) -> Result<(String, String), String> {
        let page = browser::Page::of_tab(url).ok_or("Logins are saved for web pages only")?;
        let entry = self.browser_entry(id, &page)?;
        let text = |name: &str| entry.get(name).unwrap_or_default().to_string();
        Ok((text(fields::TITLE), text(fields::USERNAME)))
    }

    /// Entry `id`, when it is in use, fits `page` and the extension may have it there.
    fn browser_entry(&self, id: &str, page: &browser::Page) -> Result<EntryRef<'_>, String> {
        let entry = self.entry(id).filter(|e| kind(e) == Kind::Entry).ok_or(NOT_FOUND)?;
        if !page.fits_entry(&entry) || !browser::offered(&entry, page.host()) {
            return Err("This login is not for the page".into());
        }
        Ok(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::tests::*;

    #[test]
    fn the_browser_extension_gets_a_site_s_logins_and_saves_new_ones() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("browser.kdbx");
        Vault::create(&path, "Browser", Some("pw"), None).unwrap();
        let mut vault = Vault::open(&path, Some("pw"), None).unwrap();
        vault.browser_associate("Chrome", "a2V5").unwrap();
        assert_eq!(Vault::open(&path, Some("pw"), None).unwrap().browser_key("Chrome").as_deref(), Some("a2V5"), "saved to the file");

        let id = vault.browser_save_login(None, "https://mail.example/login", "me", "pw-1", None).unwrap();
        let logins = vault.browser_logins("https://www.mail.example/inbox");
        assert_eq!(logins.len(), 1);
        assert_eq!((logins[0].login.as_str(), logins[0].password.as_str(), logins[0].group.as_str()), ("me", "pw-1", browser::NEW_LOGINS_GROUP));
        assert!(vault.browser_logins("http://mail.example/").is_empty(), "https pages only");
        assert!(vault.browser_logins("https://evil.example/").is_empty());

        // Updated by the UUID the extension has, only for its own site.
        let uuid = logins[0].uuid.clone();
        assert_eq!(vault.browser_replaced(&uuid, "https://mail.example/").unwrap(), ("mail.example".to_string(), "me".to_string()));
        vault.browser_save_login(Some(&uuid), "https://mail.example/", "me", "pw-2", None).unwrap();
        assert_eq!(vault.browser_logins("https://mail.example/")[0].password, "pw-2");
        assert!(vault.browser_replaced(&uuid, "https://evil.example/").is_err(), "not asked about");
        assert!(vault.browser_save_login(Some(&uuid), "https://evil.example/", "me", "pw-3", None).is_err());
        assert_eq!(vault.history(&id).unwrap().len(), 1, "the old password is in the history");

        // Hidden from the extension, or denied to the page's host: neither updated nor its code read.
        let entry = parse_id(&uuid).unwrap();
        vault.db.entry_mut(entry).unwrap().custom_data.insert("KeePassXC-Browser Settings".into(), mark(r#"{"Deny":["mail.example"]}"#));
        assert!(vault.browser_save_login(Some(&uuid), "https://mail.example/", "me", "pw-3", None).is_err(), "denied");
        vault.db.entry_mut(entry).unwrap().custom_data.clear();
        vault.db.entry_mut(entry).unwrap().set_protected("otp", "otpauth://totp/x?secret=JBSWY3DPEHPK3PXP");
        assert!(vault.browser_totp(&uuid).is_some());
        vault.db.entry_mut(entry).unwrap().custom_data.insert("BrowserHideEntry".into(), mark("true"));
        assert!(vault.browser_save_login(Some(&uuid), "https://mail.example/", "me", "pw-3", None).is_err(), "hidden");
        assert!(vault.browser_replaced(&uuid, "https://mail.example/").is_err(), "hidden: not asked about");
        assert_eq!(vault.browser_totp(&uuid), None);
        vault.db.entry_mut(entry).unwrap().custom_data.clear();

        let (name, group) = vault.browser_create_group("Web/Shops").unwrap();
        assert_eq!(name, "Shops");
        vault.browser_save_login(None, "https://shop.example/", "me", "pw", Some(&group)).unwrap();
        assert_eq!(vault.browser_logins("https://shop.example/")[0].group, "Shops");
        assert_eq!(vault.browser_groups().children.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), [browser::NEW_LOGINS_GROUP, "Web"]);
    }
}
