//! What the window reads of the open database: the listing, an entry's
//! details and history, one field or file when asked for it, the reports
//! over all of them, and an export. Secrets stay behind, but for the one
//! field asked for and an export, which holds them all.

use super::*;

impl Vault {
    /// Reused, weak and old passwords among the entries the user works with.
    pub fn health(&self) -> crate::health::Health {
        crate::health::check(self.visible_entries(), keepass::db::Times::now())
    }

    /// The entries in use as a CXF export for another password manager (see
    /// [crate::cxf::export]), one account named after the database. Every
    /// secret is in it, in clear.
    pub fn export(&self) -> Zeroizing<String> {
        let name = self.db.meta.database_name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "PswManager".into());
        crate::cxf::export(self.visible_entries(), &name, chrono::Utc::now().timestamp())
    }

    /// The entries in use as a file for another password manager (#240, see
    /// [crate::file_export]), named after the database. Every secret is in
    /// it, in clear.
    pub fn export_file(&self, format: crate::file_export::Format) -> crate::file_export::Export {
        use crate::file_export::{self, Export, Format};
        let mut export = match format {
            Format::Csv => file_export::csv(self.visible_entries()),
            Format::Cxf => {
                let with_files = self.visible_entries().filter(|e| e.attachments().next().is_some()).count();
                let entries = self.visible_entries().count();
                Export { name: String::new(), content: self.export(), entries, left_out: file_export::cxf_left_out(with_files) }
            }
        };
        export.name = file_export::file_name(&self.database_name(), format);
        export
    }

    /// The database's name, empty when it has none.
    pub fn database_name(&self) -> String {
        self.db.meta.database_name.clone().unwrap_or_default()
    }

    /// The entries the user works with that are alike in `criteria` (#267).
    pub fn similar(&self, criteria: crate::similar::Criteria) -> Vec<crate::similar::Similar> {
        crate::similar::find(self.visible_entries(), criteria)
    }

    /// Every entry, the recycle bin's and the templates too, each marked with its [Kind].
    pub fn listing(&self) -> Listing {
        let mut custom_icons = BTreeMap::new();
        let mut entries: Vec<EntrySummary> = self
            .db
            .iter_all_entries()
            .map(|e| {
                let summary = summary(&e);
                if let (Some(key), Some(icon)) = (&summary.custom_icon, e.custom_icon()) {
                    // Entries often share one icon: encode it once.
                    if !custom_icons.contains_key(key) {
                        if let Some(url) = icons::data_url(&icon.data) {
                            custom_icons.insert(key.clone(), url);
                        }
                    }
                }
                summary
            })
            .collect();
        // The PC's list sorts again in the order the user chose (#390).
        entries.sort_by_cached_key(|e| (e.title.to_lowercase(), e.id.clone()));
        // An icon that is not an image the webview can show falls back to the site's.
        for entry in &mut entries {
            if entry.custom_icon.as_ref().is_some_and(|k| !custom_icons.contains_key(k)) {
                entry.custom_icon = None;
            }
        }
        let in_import_groups = crate::cxf::in_import_groups(&self.db, &self.hidden_groups());
        Listing { entries, custom_icons, database: self.settings(), in_import_groups }
    }

    pub fn detail(&self, id: &str) -> Option<EntryDetail> {
        let entry = self.entry(id)?;
        Some(detail_of(&entry))
    }

    /// The entry's older versions, newest first: when each was saved and what
    /// changed from it to the next newer one (names only, never values).
    pub fn history(&self, id: &str) -> Option<Vec<Version>> {
        let entry = self.entry(id)?;
        let versions: Vec<EntryRef<'_>> = (0..versions_of(&entry)).filter_map(|i| entry.historical(i)).collect();
        let listed = versions.iter().enumerate().map(|(i, version)| {
            let newer = if i == 0 { &entry } else { &versions[i - 1] };
            Version { modified: version.times.last_modification.map(edit::time_text), changed: changes(version, newer) }
        });
        Some(listed.collect())
    }

    /// An older version (`index` in the history, newest first), read only,
    /// with the fields whose value differs from the entry's now.
    pub fn version(&self, id: &str, index: usize) -> Option<VersionDetail> {
        let entry = self.entry(id)?;
        let version = entry.historical(index)?;
        let names: BTreeSet<&String> = version.fields.keys().chain(entry.fields.keys()).collect();
        let differs = names
            .into_iter()
            .filter(|name| version.fields.get(*name) != entry.fields.get(*name))
            .map(|name| {
                let now = entry.fields.get(name);
                let protected = now.is_some_and(|value| is_secret(name, value));
                Difference { name: name.clone(), current: now.filter(|_| !protected).map(|value| value.get().clone()), protected }
            })
            .collect();
        Some(VersionDetail { detail: detail_of(&version), differs })
    }

    /// The content of one of the entry's files (in an older version with
    /// `version`), for saving it to disk.
    pub fn attachment(&self, id: &str, version: Option<usize>, name: &str) -> Option<Zeroizing<Vec<u8>>> {
        let entry = self.entry(id)?;
        let file = |at: &EntryRef<'_>| at.attachment_by_name(name).map(|a| Zeroizing::new(a.data.get().clone()));
        match version {
            Some(index) => file(&entry.historical(index)?),
            None => file(&entry),
        }
    }

    /// One field's value, protected or not (`Password`, `UserName`, `URL` or
    /// an additional attribute), in an older version with `version`.
    pub fn field_in(&self, id: &str, version: Option<usize>, name: &str) -> Option<Zeroizing<String>> {
        let entry = self.entry(id)?;
        let value = |at: &EntryRef<'_>| at.fields.get(name).map(|v| Zeroizing::new(v.get().clone()));
        match version {
            Some(index) => value(&entry.historical(index)?),
            None => value(&entry),
        }
    }

    /// One of the entry's own field values.
    pub fn field(&self, id: &str, name: &str) -> Option<Zeroizing<String>> {
        self.field_in(id, None, name)
    }

    /// The address the entry's URL, or the further one `name` (`KP2A_URL_n`),
    /// opens in the browser, if it is one.
    pub fn link(&self, id: &str, name: Option<&str>) -> Option<url::Url> {
        let name = name.unwrap_or(fields::URL);
        icons::link(&self.field(id, name).filter(|_| holds_url(name))?)
    }

    /// The entry's current TOTP code, or `None` when it has no secret.
    pub fn totp(&self, id: &str) -> Result<Option<otp::Code>, String> {
        let Some(value) = self.entry(id).and_then(|e| otp::find(&e.fields)) else { return Ok(None) };
        otp::Totp::parse(&value).map(|totp| Some(totp.code_now()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::tests::*;

    #[test]
    fn listing_marks_the_recycle_bin_and_hides_secrets() {
        let vault = Vault::from_database(sample());
        let listing = vault.listing();
        let titles: Vec<(&str, Kind)> = listing.entries.iter().map(|e| (e.title.as_str(), e.kind)).collect();
        assert_eq!(titles, [("alpha router", Kind::Entry), ("Deleted", Kind::Trash), ("Mail", Kind::Entry)]);
        let mail = &listing.entries[2];
        assert_eq!(mail.group, ["Work"]);
        assert_eq!(mail.host.as_deref(), Some("example.com"));
        assert!(mail.has_password);
        let json = serde_json::to_string(&listing.entries).unwrap();
        assert!(!json.contains("s3cret") && !json.contains("pin-1234"), "{json}");
    }

    #[test]
    fn detail_masks_protected_fields() {
        let vault = Vault::from_database(sample());
        let id = in_use(&vault)[1].id.clone();
        let detail = vault.detail(&id).unwrap();
        let modified = vault.db.entry(parse_id(&id).unwrap()).unwrap().times.last_modification.unwrap();
        assert_eq!(detail.summary.modified, Some(edit::time_text(modified)));
        assert!(detail.summary.modified.as_deref().is_some_and(|t| t.ends_with('Z')));
        assert_eq!(
            detail.fields,
            [
                Field { name: "PIN".into(), value: None, protected: true, link: false },
                Field { name: "Recovery phone".into(), value: Some("+0 000".into()), protected: false, link: false },
            ]
        );
        assert_eq!(vault.field(&id, fields::PASSWORD).unwrap().as_str(), "s3cret");
        assert_eq!(vault.field(&id, "PIN").unwrap().as_str(), "pin-1234");
        assert!(vault.field(&id, "Nope").is_none());
    }

    #[test]
    fn listing_marks_templates_2fa_passkeys_and_expiry() {
        let mut db = sample();
        let mut root = db.root_mut();
        let mut templates = root.add_group();
        templates.name = "Templates".into();
        let templates_id = templates.id();
        templates.add_entry().edit(|e| e.set_unprotected(fields::TITLE, "Card"));
        root.add_entry().edit(|e| {
            e.set_unprotected(fields::TITLE, "Shop");
            e.set_protected(fields::OTP, "otpauth://totp/Shop?secret=JBSWY3DP");
            e.set_protected(edit::passkey::PRIVATE_KEY, "key");
            e.times.expires = Some(true);
            e.times.expiry = chrono::NaiveDate::from_ymd_opt(2030, 1, 2).unwrap().and_hms_opt(3, 4, 5).unwrap();
        });
        db.meta.entry_templates_group = Some(templates_id.uuid());
        let listing = Vault::from_database(db).listing();
        let by_title = |title: &str| listing.entries.iter().find(|e| e.title == title).unwrap();
        assert_eq!(by_title("Card").kind, Kind::Template);
        let shop = by_title("Shop");
        assert!(shop.otp && shop.passkey);
        assert_eq!(shop.expires.as_deref(), Some("2030-01-02T03:04:05Z"));
        let mail = by_title("Mail");
        assert!(!mail.otp && !mail.passkey && mail.expires.is_none());
        assert!(mail.modified.as_deref().is_some_and(|t| t.ends_with('Z')), "the list sorts by it (#390)");
        let json = serde_json::to_string(&listing.entries).unwrap();
        assert!(!json.contains("JBSWY3DP") && !json.contains("\"key\""), "{json}");
    }

    /// Protected values share one keystream in file order; reading them in any
    /// other order gives wrong secrets, not an error. The fixtures are made by
    /// `tools/sic2kdbx/make_fixtures.py`.
    #[test]
    fn reads_the_secrets_sic2kdbx_writes() {
        let vault = Vault::open(&fixture_path("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        let router = vault.listing().entries.into_iter().find(|e| e.title == "Router").unwrap();
        assert_eq!(router.tags, ["NET", "Old", "Favorite"]);
        assert_eq!(vault.field(&router.id, fields::PASSWORD).unwrap().as_str(), "new-pass");
        assert_eq!(vault.field(&router.id, "PIN").unwrap().as_str(), "1234");
        assert_eq!(vault.field(&router.id, fields::OTP).unwrap().as_str(), "otpauth://totp/Router?secret=JBSWY3DP");
        let entry = vault.db.entry(EntryId::from(Uuid::parse_str(&router.id).unwrap())).unwrap();
        let old = &entry.history.as_ref().unwrap().get_entries()[0];
        assert_eq!(old.get_password(), Some("old-pass"));
    }

    #[test]
    fn protected_standard_fields_and_totp_stay_in_the_backend() {
        let mut db = Database::new();
        db.root_mut().add_entry().edit(|e| {
            e.set_unprotected(fields::TITLE, "Bank");
            e.set_protected(fields::USERNAME, "client-42");
            e.set_protected(fields::URL, "bank.example.com");
            e.set_unprotected(fields::OTP, "otpauth://totp/Bank?secret=JBSWY3DP");
        });
        let vault = Vault::from_database(db);
        let listing = vault.listing();
        let json = serde_json::to_string(&listing.entries).unwrap();
        assert!(!json.contains("client-42") && !json.contains("bank.example") && !json.contains("JBSWY3DP"), "{json}");
        let id = &listing.entries[0].id;
        let detail = vault.detail(id).unwrap();
        let names: Vec<(&str, bool)> = detail.fields.iter().map(|f| (f.name.as_str(), f.protected)).collect();
        assert_eq!(names, [("otp", true), ("URL", true), ("UserName", true)]);
        assert!(detail.fields.iter().all(|f| f.value.is_none()));
        assert_eq!(vault.field(id, fields::USERNAME).unwrap().as_str(), "client-42");
        assert_eq!(vault.link(id, None).unwrap().as_str(), "https://bank.example.com/");
    }

    #[test]
    fn urls_and_further_urls_open_in_the_browser() {
        let mut db = Database::new();
        db.root_mut().add_entry().edit(|e| {
            e.set_unprotected(fields::TITLE, "NAS");
            e.set_unprotected(fields::URL, "nas.example.com:5001");
            e.set_unprotected("KP2A_URL_1", "192.168.1.10:5000");
            e.set_unprotected("KP2A_URL_2", "androidapp://com.example.nas");
            e.set_unprotected("Admin page", "https://nas.example.com/admin");
        });
        let vault = Vault::from_database(db);
        let id = &vault.listing().entries[0].id;
        let detail = vault.detail(id).unwrap();
        assert!(detail.summary.link);
        let links: Vec<(&str, bool)> = detail.fields.iter().map(|f| (f.name.as_str(), f.link)).collect();
        assert_eq!(links, [("Admin page", false), ("KP2A_URL_1", true), ("KP2A_URL_2", false)]);
        assert_eq!(vault.link(id, None).unwrap().as_str(), "https://nas.example.com:5001/");
        assert_eq!(vault.link(id, Some("KP2A_URL_1")).unwrap().as_str(), "https://192.168.1.10:5000/");
        // Only the URL fields open, whatever another field holds.
        assert!(vault.link(id, Some("KP2A_URL_2")).is_none() && vault.link(id, Some("Admin page")).is_none());
    }

    #[test]
    fn an_entry_icon_is_used_only_when_it_is_an_image() {
        let mut db = Database::new();
        let mut root = db.root_mut();
        root.add_entry().edit(|e| {
            e.set_unprotected(fields::TITLE, "a");
            e.set_unprotected(fields::URL, "example.com");
            e.set_icon_custom_new(b"\x89PNG\r\n\x1a\n....".to_vec());
        });
        root.add_entry().edit(|e| {
            e.set_unprotected(fields::TITLE, "b");
            e.set_unprotected(fields::URL, "example.org");
            e.set_icon_custom_new(b"not an image".to_vec());
        });
        let listing = Vault::from_database(db).listing();
        let (a, b) = (&listing.entries[0], &listing.entries[1]);
        let key = a.custom_icon.as_ref().unwrap();
        assert!(listing.custom_icons[key].starts_with("data:image/png;base64,"));
        assert_eq!(listing.custom_icons.len(), 1);
        // No usable icon of its own: the site's icon (or the default) is shown instead.
        assert_eq!((b.custom_icon.as_deref(), b.host.as_deref()), (None, Some("example.org")));
    }

    #[test]
    fn totp_codes_come_from_the_otp_field() {
        let dir = tempfile::tempdir().unwrap();
        let vault = fixture("sic2kdbx.kdbx", dir.path());
        let router = vault.listing().entries.iter().find(|e| e.title == "Router").unwrap().id.clone();
        let code = vault.totp(&router).unwrap().unwrap();
        assert_eq!((code.code.len(), code.period), (6, 30));
        let mail = vault.listing().entries.iter().find(|e| e.title == "Mail").unwrap().id.clone();
        assert!(vault.totp(&mail).unwrap().is_none());
    }
}
