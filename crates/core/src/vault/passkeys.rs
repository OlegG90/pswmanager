//! Passkeys: the entries that hold one for a site or app, making a new one,
//! and signing in with one.

use super::*;

impl Vault {
    /// The passkeys for the site or app `rp_id`, among the entries the user
    /// works with: only those with a credential id in `allowed` when the
    /// request names some (WebAuthn's `allowCredentials`).
    pub fn passkeys_for(&self, rp_id: &str, allowed: &[String]) -> Vec<EntryChoice> {
        self.passkeys(rp_id, allowed, |_| true)
    }

    /// [Vault::passkeys_for] for the browser extension: not those hidden from
    /// it ([browser::hidden]), as KeePassXC leaves them out.
    pub fn browser_passkeys(&self, rp_id: &str, allowed: &[String]) -> Vec<EntryChoice> {
        self.passkeys(rp_id, allowed, |e| !browser::hidden(e))
    }

    /// The passkeys for `rp_id`, among the entries in use that `offered` keeps.
    fn passkeys(&self, rp_id: &str, allowed: &[String], offered: impl Fn(&EntryRef<'_>) -> bool) -> Vec<EntryChoice> {
        // Ids compared as bytes: exporters write base64url or plain base64, padded or not.
        let restricted = !allowed.is_empty();
        let allowed: Vec<Vec<u8>> = allowed.iter().filter_map(|id| crate::webauthn::credential_id_bytes(id)).collect();
        self.visible_entries()
            .filter(|e| offered(e))
            .filter(|e| e.get(edit::passkey::RELYING_PARTY).is_some_and(|rp| rp.eq_ignore_ascii_case(rp_id)))
            .filter(|e| !restricted || e.get(edit::passkey::CREDENTIAL_ID).and_then(crate::webauthn::credential_id_bytes).is_some_and(|id| allowed.contains(&id)))
            .map(|e| EntryChoice::of(&e, e.get(edit::passkey::USERNAME).filter(|u| !u.is_empty()).or(e.get(fields::USERNAME))))
            .collect()
    }

    /// The entries a new passkey for the site `rp_id` can go into: the
    /// user's entries whose URL is the site's (or under it) and that hold no
    /// passkey yet (KeePassXC keeps one per entry).
    pub fn entries_for_new_passkey(&self, rp_id: &str) -> Vec<EntryChoice> {
        self.entries_for_passkey(rp_id, |_| true)
    }

    /// [Vault::entries_for_new_passkey] for the browser extension: not those
    /// hidden from it ([browser::hidden]).
    pub fn browser_entries_for_new_passkey(&self, rp_id: &str) -> Vec<EntryChoice> {
        self.entries_for_passkey(rp_id, |e| !browser::hidden(e))
    }

    /// The entries a new passkey for `rp_id` can go into, among those `offered` keeps.
    fn entries_for_passkey(&self, rp_id: &str, offered: impl Fn(&EntryRef<'_>) -> bool) -> Vec<EntryChoice> {
        self.visible_entries()
            .filter(|e| offered(e))
            .filter(|e| !e.fields.keys().any(|name| name.starts_with(edit::passkey::PREFIX)))
            .filter(|e| e.get(fields::URL).and_then(icons::host_of).is_some_and(|host| crate::webauthn::on_site(&host, rp_id)))
            .map(|e| EntryChoice::of(&e, e.get(fields::USERNAME)))
            .collect()
    }

    /// Keeps a passkey just made (see [crate::webauthn::make]) in entry `id`,
    /// or in a new entry for the site (its name, user name and address), and
    /// saves the file. The entry gets KeePassXC's tag. Returns its id.
    pub fn add_passkey(&mut self, id: Option<&str>, made: &crate::webauthn::Made) -> Result<String, String> {
        self.keep_passkey(id, made, Vec::new())
    }

    /// [Vault::add_passkey] for the browser extension (#285): a new entry goes
    /// into the group the extension names (`Parent/Child`, made where
    /// missing), else KeePassXC's [browser::NEW_PASSKEYS_GROUP].
    pub fn add_browser_passkey(&mut self, id: Option<&str>, made: &crate::webauthn::Made, group: Option<&str>) -> Result<String, String> {
        let path = group.map(browser::split_path).filter(|p| !p.is_empty()).unwrap_or_else(|| vec![browser::NEW_PASSKEYS_GROUP.to_string()]);
        self.keep_passkey(id, made, path)
    }

    /// [Vault::add_passkey], a new entry in the group at `path` (empty: the top).
    fn keep_passkey(&mut self, id: Option<&str>, made: &crate::webauthn::Made, path: Vec<String>) -> Result<String, String> {
        let (base, mut data) = match id {
            Some(id) => {
                let data = self.edit_data(id).ok_or(NOT_FOUND)?;
                if data.fields.iter().any(|f| f.name.starts_with(edit::passkey::PREFIX)) {
                    return Err("This entry has a passkey already".into());
                }
                (Some(data.clone()), data)
            }
            None => {
                let mut data = EntryData::default();
                data.title = made.rp_name.clone();
                data.username = made.user_name.clone();
                data.url = format!("https://{}", made.rp_id);
                data.group = path;
                (None, data)
            }
        };
        data.fields.extend(made.fields.iter().cloned());
        if !data.tags.iter().any(|t| t == edit::passkey::TAG) {
            data.tags.push(edit::passkey::TAG.to_string());
        }
        self.save_entry(id, base.as_ref(), &data, false, &[]).map(|(id, _)| id)
    }

    /// Signs in with the passkey of entry `id` (WebAuthn's request options,
    /// `request_json`), after the user was verified: the answer for the site
    /// or app (see [crate::webauthn::sign]).
    pub fn sign_with_passkey(&self, id: &str, request_json: &str, caller: &crate::webauthn::Caller) -> Result<String, String> {
        let entry = self.entry(id).filter(|e| kind(e) == Kind::Entry).ok_or(NOT_FOUND)?;
        let need = |name: &str| entry.get(name).filter(|v| !v.is_empty()).ok_or_else(|| "This entry's passkey is incomplete".to_string());
        // KeePassXC writes "1"; missing means on, as there.
        let flag = |name: &str| entry.get(name).is_none_or(|v| v == "1" || v.eq_ignore_ascii_case("true"));
        let stored = crate::webauthn::Stored {
            rp_id: need(edit::passkey::RELYING_PARTY)?,
            credential_id: need(edit::passkey::CREDENTIAL_ID)?,
            user_handle: need(edit::passkey::USER_HANDLE)?,
            private_key_pem: need(edit::passkey::PRIVATE_KEY)?,
            backup_eligible: flag(edit::passkey::FLAG_BE),
            backed_up: flag(edit::passkey::FLAG_BS),
        };
        crate::webauthn::sign(request_json, &stored, caller)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::tests::*;

    #[test]
    fn offers_a_sites_passkeys_and_signs_in_with_one() {
        let caller = crate::webauthn::Caller::App { origin: "android:apk-key-hash:x".into(), package: "com.example".into() };
        let made = crate::webauthn::make(
            r#"{"rp": {"id": "example.com"}, "user": {"id": "dQ", "name": "alice"}, "challenge": "Yw"}"#,
            &caller,
        )
        .unwrap();
        let mut db = sample();
        let mut data = EntryData::default();
        data.title = "Example".into();
        data.fields = made.fields.clone();
        let id = edit::apply(&mut db, None, &data, &HashSet::new()).unwrap().uuid().to_string();
        let vault = Vault::from_database(db);
        let offered = vault.passkeys_for("example.com", &[]);
        assert_eq!((offered[0].id.as_str(), offered[0].title.as_str(), offered[0].username.as_str()), (id.as_str(), "Example", "alice"));
        assert!(vault.passkeys_for("other.com", &[]).is_empty());
        assert_eq!(vault.passkeys_for("Example.COM", &[]).len(), 1);
        // A site that names only ids nothing can read gets none, not all.
        assert!(vault.passkeys_for("example.com", &["!!!".into()]).is_empty());
        assert!(vault.passkeys_for("example.com", &["c29tZXRoaW5nLWVsc2U".into()]).is_empty());
        let credential_id = made.fields.iter().find(|f| f.name == edit::passkey::CREDENTIAL_ID).unwrap().value.clone();
        assert_eq!(vault.passkeys_for("example.com", std::slice::from_ref(&credential_id)).len(), 1);
        assert_eq!(vault.browser_passkeys("example.com", &[]).len(), 1);

        let response = vault.sign_with_passkey(&id, r#"{"rpId": "example.com", "challenge": "c2lnbg"}"#, &caller).unwrap();
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["id"], credential_id.as_str());
        assert_eq!(json["response"]["userHandle"], "dQ");
        assert!(vault.sign_with_passkey(&id, r#"{"rpId": "other.com", "challenge": "Yw"}"#, &caller).is_err());

        // Hidden from the browser extension: offered everywhere else.
        let mut vault = vault;
        vault.db.entry_mut(parse_id(&id).unwrap()).unwrap().custom_data.insert("BrowserHideEntry".into(), mark("true"));
        assert!(vault.browser_passkeys("example.com", &[]).is_empty());
        assert_eq!(vault.passkeys_for("example.com", &[]).len(), 1);
    }

    #[test]
    fn a_page_s_passkey_goes_to_keepassxc_s_group_and_signs_with_client_data_made_here() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let page = crate::webauthn::Caller::Page { origin: "https://shop.example".into() };
        let made = crate::webauthn::make(r#"{"rp": {"id": "shop.example", "name": "Shop"}, "user": {"id": "dQ", "name": "bob"}, "challenge": "Yw"}"#, &page).unwrap();
        let id = vault.add_browser_passkey(None, &made, None).unwrap();
        assert_eq!(vault.edit_data(&id).unwrap().group, [browser::NEW_PASSKEYS_GROUP]);
        let named = vault.add_browser_passkey(None, &made, Some("Web/Passkeys")).unwrap();
        assert_eq!(vault.edit_data(&named).unwrap().group, ["Web", "Passkeys"], "the group the extension names");
        let decode = |text: &serde_json::Value| base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, text.as_str().unwrap()).unwrap();
        let made_json: serde_json::Value = serde_json::from_str(&made.response).unwrap();
        assert!(made_json["response"]["attestationObject"].is_string() && made_json["response"]["publicKey"].is_string());
        let created: serde_json::Value = serde_json::from_slice(&decode(&made_json["response"]["clientDataJSON"])).unwrap();
        assert_eq!((created["type"].as_str(), created.get("androidPackageName")), (Some("webauthn.create"), None));

        let response = vault.sign_with_passkey(&id, r#"{"rpId": "shop.example", "challenge": "c2lnbg"}"#, &page).unwrap();
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        let client_data: serde_json::Value = serde_json::from_slice(&decode(&json["response"]["clientDataJSON"])).unwrap();
        assert_eq!(client_data, serde_json::json!({ "type": "webauthn.get", "challenge": "c2lnbg", "origin": "https://shop.example", "crossOrigin": false }));
    }

    #[test]
    fn a_new_passkey_goes_into_the_sites_entry_or_a_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let mut data = EntryData::default();
        data.title = "Shop".into();
        data.username = "bob".into();
        data.url = "https://www.shop.example/login".into();
        let (shop, _) = vault.save_entry(None, None, &data, false, &[]).unwrap();
        let caller = crate::webauthn::Caller::App { origin: "android:apk-key-hash:x".into(), package: "com.example".into() };
        let request = r#"{"rp": {"id": "shop.example", "name": "Shop"}, "user": {"id": "dQ", "name": "bob"}, "challenge": "Yw"}"#;

        // The site's entry is offered, until it has a passkey.
        let offered = vault.entries_for_new_passkey("shop.example");
        assert_eq!(offered.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), [shop.as_str()]);
        vault.db.entry_mut(parse_id(&shop).unwrap()).unwrap().custom_data.insert("BrowserHideEntry".into(), mark("true"));
        assert!(vault.browser_entries_for_new_passkey("shop.example").is_empty(), "hidden from the extension");
        vault.db.entry_mut(parse_id(&shop).unwrap()).unwrap().custom_data.clear();
        let made = crate::webauthn::make(request, &caller).unwrap();
        assert_eq!(vault.add_passkey(Some(&shop), &made).unwrap(), shop);
        assert!(vault.entries_for_new_passkey("shop.example").is_empty());
        assert!(vault.add_passkey(Some(&shop), &made).is_err());
        let detail = vault.edit_data(&shop).unwrap();
        assert!(detail.tags.contains(&edit::passkey::TAG.to_string()));
        assert_eq!(detail.username, "bob");

        // A second one goes into a new entry for the site, and is offered when signing in.
        let made = crate::webauthn::make(request, &caller).unwrap();
        let new = vault.add_passkey(None, &made).unwrap();
        let entry = vault.edit_data(&new).unwrap();
        assert_eq!((entry.title.as_str(), entry.url.as_str(), entry.username.as_str()), ("Shop", "https://shop.example", "bob"));
        let reopened = Vault::open(&dir.path().join("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        assert_eq!(reopened.passkeys_for("shop.example", &[]).len(), 2);
    }
}
