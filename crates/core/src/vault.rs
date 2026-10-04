//! The unlocked database. Everything decrypted stays here: the frontend gets
//! summaries without secrets, a secret only when asked for one field, and a
//! whole entry only while it is being edited.

use crate::dbfile::{DbFile, KeyChange, OpenError, Read, SaveError, Snapshot};
use crate::edit::{self, EntryData, Taking, NOT_FOUND};
use crate::sync::Outcome;
use crate::{encryption, icons, otp};
use keepass::db::{fields, EntryId, EntryRef, GroupId, Value};
use keepass::{Database, DatabaseKey};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::File;
use std::path::Path;
use uuid::Uuid;
use zeroize::Zeroizing;

/// An entry offered to a site or app: for its passkey, or its login.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryChoice {
    /// The entry's id.
    pub id: String,
    pub title: String,
    pub username: String,
}

impl EntryChoice {
    fn of(entry: &EntryRef<'_>, username: Option<&str>) -> EntryChoice {
        EntryChoice {
            id: entry.id().uuid().to_string(),
            title: entry.get(fields::TITLE).unwrap_or_default().to_string(),
            username: username.unwrap_or_default().to_string(),
        }
    }
}

/// Where a login is for: an Android app (its package), or a site (the host
/// of the page a browser speaks for).
#[derive(Debug, Clone, PartialEq)]
pub enum LoginPlace {
    App(String),
    Site(String),
}

impl LoginPlace {
    /// The site a browser speaks for (its page's `origin`), else the app (`package`).
    pub fn of(origin: Option<&str>, package: &str) -> Result<LoginPlace, String> {
        match origin {
            Some(origin) => crate::webauthn::origin_host(origin).map(LoginPlace::Site).ok_or_else(|| "The browser's page is not a site".into()),
            None => Ok(LoginPlace::App(package.to_string())),
        }
    }

    /// Whether the entry is for this place: an app named among its URLs
    /// (`androidapp://<package>`, as Keepass2Android keeps it; packages are
    /// case-sensitive), or a site whose host is the entry's (a `www.` aside).
    /// Not a site's other hosts: a password goes to its own host only.
    fn matches(&self, entry: &EntryRef<'_>) -> bool {
        match self {
            LoginPlace::App(package) => {
                let wanted = format!("androidapp://{package}");
                entry.fields.iter().any(|(name, value)| (name == fields::URL || name.starts_with("KP2A_URL")) && value.get().trim() == wanted)
            }
            LoginPlace::Site(host) => {
                let bare = |h: &str| h.trim_start_matches("www.").to_ascii_lowercase();
                entry.get(fields::URL).and_then(icons::host_of).is_some_and(|site| bare(&site) == bare(host))
            }
        }
    }

    /// The URL a new entry for this place gets.
    fn url(&self) -> String {
        match self {
            LoginPlace::App(package) => format!("androidapp://{package}"),
            LoginPlace::Site(host) => format!("https://{host}"),
        }
    }
}

pub struct Vault {
    db: Database,
    /// The file it came from; `None` only in tests that never save.
    file: Option<DbFile>,
    /// `db` holds changes the file lacks (kept from an older file that was
    /// read back); the next [Vault::change] writes them even if it changes nothing.
    unsaved: bool,
}

/// What the list shows about an entry. No password, no protected field.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntrySummary {
    pub id: String,
    pub title: String,
    pub username: String,
    pub url: String,
    /// The site the entry's URL points to, for its icon.
    pub host: Option<String>,
    /// Group names from the top, without the root group.
    pub group: Vec<String>,
    pub tags: Vec<String>,
    pub notes: String,
    /// Key into [Listing::custom_icons] when the entry has its own icon.
    pub custom_icon: Option<String>,
    /// The KeePass standard icon chosen for it (not the default key, 0).
    pub icon: Option<usize>,
    pub has_password: bool,
    /// Where the entry is: in use, a template, or in the recycle bin.
    pub kind: Kind,
    /// It has a TOTP secret.
    pub otp: bool,
    /// It has a passkey KeePassXC stored.
    pub passkey: bool,
    /// When it expires (UTC, RFC 3339), if it does.
    pub expires: Option<String>,
}

/// Where an entry is, for the sidebar's groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Entry,
    Template,
    /// In the recycle bin (a template there too).
    Trash,
}

/// An additional attribute. A protected one comes without its value.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    pub name: String,
    pub value: Option<String>,
    pub protected: bool,
}

/// A file attached to an entry: its name and size, never its content.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub name: String,
    pub size: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryDetail {
    #[serde(flatten)]
    pub summary: EntrySummary,
    pub fields: Vec<Field>,
    pub attachments: Vec<Attachment>,
    /// When the entry was last changed (UTC, RFC 3339), if the file says.
    pub modified: Option<String>,
    /// How many older versions its history keeps.
    pub versions: usize,
}

/// An older version of an entry, as its history lists it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    /// When it was saved (UTC, RFC 3339).
    pub modified: Option<String>,
    /// What changed from it to the next newer version: names, never values.
    pub changed: Vec<String>,
}

/// An older version shown read only, with how it differs from the entry now.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionDetail {
    #[serde(flatten)]
    pub detail: EntryDetail,
    pub differs: Vec<Difference>,
}

/// A field whose value in a version is not the entry's current one.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Difference {
    pub name: String,
    /// The current value, when it is not a secret (and the entry has the field).
    pub current: Option<String>,
    /// The current value is a secret: the window only says it differs.
    pub protected: bool,
}

/// An entry the editor saved, and the listing after it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub id: String,
    pub listing: Listing,
    /// Fields another device also changed; this edit replaced them.
    pub conflicts: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub entries: Vec<EntrySummary>,
    /// The database's own icons used by the entries, as `data:` URLs.
    pub custom_icons: BTreeMap<String, String>,
    /// The name, description and default user name the file keeps.
    pub database: DatabaseSettings,
}

/// The settings kept in the database file itself (see [edit::Setting]);
/// empty when the file has none.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseSettings {
    pub name: String,
    pub description: String,
    pub default_username: String,
    /// Old versions kept per entry; -1 for no limit.
    pub history_max_items: isize,
    /// Bytes of old versions kept per entry; -1 for no limit.
    pub history_max_size: isize,
    /// The cipher and key derivation, from the file's header.
    pub encryption: encryption::Encryption,
}

impl Vault {
    /// Opens a KDBX file with a password, a key file, or both.
    pub fn open(path: &Path, password: Option<&str>, key_file: Option<&Path>) -> Result<Vault, String> {
        Self::open_with_key(path, key(password, key_file)?)
    }

    /// [Vault::open] with the key already made ([key_reading]).
    pub fn open_with_key(path: &Path, key: DatabaseKey) -> Result<Vault, String> {
        let (db, file) = DbFile::open(path, key)?;
        Ok(Vault { db, file: Some(file), unsaved: false })
    }

    /// Creates a new, empty database at `path` (never over a file already
    /// there): KDBX 4 with AES-256 and Argon2id, as KeePassXC makes them, the
    /// recycle bin on, the root group named `name`.
    pub fn create(path: &Path, name: &str, password: Option<&str>, key_file: Option<&Path>) -> Result<(), String> {
        if password.is_none() && key_file.is_none() {
            return Err("A database needs a master password, a key file, or both".into());
        }
        if path.exists() {
            return Err(format!("{} is already there: choose another name", path.display()));
        }
        let key = key(password, key_file)?;
        let mut db = Database::with_config(new_database_config());
        db.meta.database_name = Some(name.to_string());
        db.meta.recyclebin_enabled = Some(true);
        db.meta.history_max_items = Some(10);
        db.root_mut().name = name.to_string();
        let mut bytes = Vec::new();
        db.save(&mut bytes, key.clone()).map_err(|e| format!("Cannot write the database: {e}"))?;
        Database::parse(&bytes, key).map_err(|e| format!("The new database did not open again ({e})"))?;
        crate::store::write_atomically(path, &bytes).map_err(|e| format!("Cannot write {}: {e}", path.display()))
    }

    #[cfg(test)]
    pub fn from_database(db: Database) -> Vault {
        Vault { db, file: None, unsaved: false }
    }

    /// All entries the user works with: not in the recycle bin, not templates.
    fn visible_entries(&self) -> impl Iterator<Item = EntryRef<'_>> {
        let hidden = self.hidden_groups();
        self.db.iter_all_entries().filter(move |e| !is_in(e, &hidden))
    }

    fn hidden_groups(&self) -> HashSet<GroupId> {
        edit::hidden_groups(&self.db)
    }

    /// Reused, weak and old passwords among the entries the user works with.
    pub fn health(&self) -> crate::health::Health {
        crate::health::check(self.visible_entries(), keepass::db::Times::now())
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
        entries.sort_by_cached_key(|e| (e.title.to_lowercase(), e.id.clone()));
        // An icon that is not an image the webview can show falls back to the site's.
        for entry in &mut entries {
            if entry.custom_icon.as_ref().is_some_and(|k| !custom_icons.contains_key(k)) {
                entry.custom_icon = None;
            }
        }
        Listing { entries, custom_icons, database: self.settings() }
    }

    /// The settings kept in the database file, empty where it has none.
    pub fn settings(&self) -> DatabaseSettings {
        let meta = &self.db.meta;
        let or_empty = |value: &Option<String>| value.clone().unwrap_or_default();
        let (max_items, max_size) = edit::history_limits(meta);
        DatabaseSettings {
            name: or_empty(&meta.database_name),
            description: or_empty(&meta.database_description),
            default_username: or_empty(&meta.default_username),
            history_max_items: as_stored(max_items),
            history_max_size: as_stored(max_size),
            encryption: encryption::of(&self.db.config),
        }
    }

    /// Saves the database with another cipher and / or key derivation, as a
    /// change on the file as it is now.
    pub fn set_encryption(&mut self, wanted: &encryption::Encryption) -> Result<(), String> {
        self.change(|db, _| encryption::apply(&mut db.config, wanted))
    }

    /// Saves the database with a new master password and / or key file (at
    /// least one of them), once the current ones (`current`) proved right, as
    /// a change on the file as it is now; the time of the change is kept in the
    /// file (KeePass's `MasterKeyChanged`).
    pub fn change_key(&mut self, current: (Option<&str>, Option<&Path>), password: Option<&str>, key_file: Option<&Path>) -> Result<(), String> {
        let file = self.file()?;
        if !file.has_key(&key(current.0, current.1)?) {
            return Err("The current master password is not right".into());
        }
        if password.is_none() && key_file.is_none() {
            return Err("A database needs a master password, a key file, or both".into());
        }
        let new = key(password, key_file)?;
        self.save_change(Some(&new), |db, _| {
            db.meta.master_key_changed = Some(keepass::db::Times::now());
            Ok(())
        })
        .map_err(|e| e.to_string())
    }

    /// How many old versions these history limits would remove.
    pub fn versions_over_limits(&self, max_items: isize, max_size: isize) -> usize {
        edit::versions_over_limits(&self.db, max_items, max_size)
    }

    /// Sets the history limits and saves the file, every entry's history
    /// trimmed to them.
    pub fn set_history_limits(&mut self, max_items: isize, max_size: isize) -> Result<(), String> {
        // What the lists offer (no history at all, but some room for it), or
        // what the file has already: another client may have set it.
        let (items_now, size_now) = edit::history_limits(&self.db.meta);
        let items_ok = (0..=100).contains(&max_items) || max_items == as_stored(items_now);
        let size_ok = (1 << 20..=64 << 20).contains(&max_size) || max_size == as_stored(size_now);
        if !items_ok || !size_ok {
            return Err("Not a history limit".into());
        }
        self.change(|db, _| {
            edit::set_history_limits(db, max_items, max_size);
            Ok(())
        })
    }

    /// Changes a setting kept in the database file and saves the file.
    pub fn set_setting(&mut self, setting: edit::Setting, value: &str) -> Result<(), String> {
        self.change(|db, _| {
            edit::set_setting(db, setting, value);
            Ok(())
        })
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

    /// Makes an older version the entry's current content and saves the file
    /// (see [edit::restore_version]).
    /// `saved` is when the version shown was saved (see [edit::restore_version]).
    pub fn restore_version(&mut self, id: &str, index: usize, saved: Option<&str>) -> Result<(), String> {
        let id = parse_id(id)?;
        self.change(|db, hidden| edit::restore_version(db, id, index, saved, hidden))
    }

    /// The entry's URL as a web address, if it is one.
    pub fn web_url(&self, id: &str) -> Option<url::Url> {
        icons::web_url(&self.field(id, fields::URL)?)
    }

    /// The passkeys for the site or app `rp_id`, among the entries the user
    /// works with: only those with a credential id in `allowed` when the
    /// request names some (WebAuthn's `allowCredentials`).
    pub fn passkeys_for(&self, rp_id: &str, allowed: &[String]) -> Vec<EntryChoice> {
        // Ids compared as bytes: exporters write base64url or plain base64, padded or not.
        let restricted = !allowed.is_empty();
        let allowed: Vec<Vec<u8>> = allowed.iter().filter_map(|id| crate::webauthn::credential_id_bytes(id)).collect();
        self.visible_entries()
            .filter(|e| e.get(edit::passkey::RELYING_PARTY).is_some_and(|rp| rp.eq_ignore_ascii_case(rp_id)))
            .filter(|e| !restricted || e.get(edit::passkey::CREDENTIAL_ID).and_then(crate::webauthn::credential_id_bytes).is_some_and(|id| allowed.contains(&id)))
            .map(|e| EntryChoice::of(&e, e.get(edit::passkey::USERNAME).filter(|u| !u.is_empty()).or(e.get(fields::USERNAME))))
            .collect()
    }

    /// The entries a new passkey for the site `rp_id` can go into: the
    /// user's entries whose URL is the site's (or under it) and that hold no
    /// passkey yet (KeePassXC keeps one per entry).
    pub fn entries_for_new_passkey(&self, rp_id: &str) -> Vec<EntryChoice> {
        self.visible_entries()
            .filter(|e| !e.fields.keys().any(|name| name.starts_with(edit::passkey::PREFIX)))
            .filter(|e| e.get(fields::URL).and_then(icons::host_of).is_some_and(|host| crate::webauthn::on_site(&host, rp_id)))
            .map(|e| EntryChoice::of(&e, e.get(fields::USERNAME)))
            .collect()
    }

    /// Keeps a passkey just made (see [crate::webauthn::make]) in entry `id`,
    /// or in a new entry for the site (its name, user name and address), and
    /// saves the file. The entry gets KeePassXC's tag. Returns its id.
    pub fn add_passkey(&mut self, id: Option<&str>, made: &crate::webauthn::Made) -> Result<String, String> {
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
                (None, data)
            }
        };
        data.fields.extend(made.fields.iter().cloned());
        if !data.tags.iter().any(|t| t == edit::passkey::TAG) {
            data.tags.push(edit::passkey::TAG.to_string());
        }
        self.save_entry(id, base.as_ref(), &data, false, &[]).map(|(id, _)| id)
    }

    /// The entries with a login for `place` (an app or a site), as Credential
    /// Manager offers them to the app asking.
    pub fn logins_for(&self, place: &LoginPlace) -> Vec<EntryChoice> {
        self.places_entries(place, true)
    }

    /// Entry `id`'s user name and password, to hand to the app asking.
    pub fn login(&self, id: &str) -> Option<(Zeroizing<String>, Zeroizing<String>)> {
        let entry = self.entry(id).filter(|e| kind(e) == Kind::Entry)?;
        let text = |name: &str| Zeroizing::new(entry.get(name).unwrap_or_default().to_string());
        Some((text(fields::USERNAME), text(fields::PASSWORD)))
    }

    /// The entries a login an app offers to save can update: those for `place`.
    pub fn entries_for_login(&self, place: &LoginPlace) -> Vec<EntryChoice> {
        self.places_entries(place, false)
    }

    /// The user's entries for `place`; only those with a password when `with_password`.
    fn places_entries(&self, place: &LoginPlace, with_password: bool) -> Vec<EntryChoice> {
        self.visible_entries()
            .filter(|e| place.matches(e) && (!with_password || e.get(fields::PASSWORD).is_some_and(|p| !p.is_empty())))
            .map(|e| EntryChoice::of(&e, e.get(fields::USERNAME)))
            .collect()
    }

    /// Keeps a login an app offers to save: the password (and the user name
    /// when the entry has none) of entry `id`, or a new entry titled `title`
    /// for `place`; the file is saved (the old version goes to history).
    /// Returns the entry's id.
    pub fn save_login(&mut self, id: Option<&str>, place: &LoginPlace, title: &str, username: &str, password: &str) -> Result<String, String> {
        if password.is_empty() {
            return Err("There is no password to save".into());
        }
        let (base, mut data) = match id {
            Some(id) => {
                let data = self.edit_data(id).ok_or(NOT_FOUND)?;
                (Some(data.clone()), data)
            }
            None => {
                let mut data = EntryData::default();
                data.title = title.to_string();
                data.url = place.url();
                (None, data)
            }
        };
        if data.username.is_empty() {
            data.username = username.to_string();
        }
        data.password = password.to_string();
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

    /// An entry by id, to show or copy from: templates and the recycle bin's too.
    fn entry(&self, id: &str) -> Option<EntryRef<'_>> {
        self.db.entry(parse_id(id).ok()?)
    }

    /// The entry with every value, for the editor: not a template or one in
    /// the recycle bin.
    pub fn edit_data(&self, id: &str) -> Option<EntryData> {
        let entry = self.entry(id).filter(|e| kind(e) != Kind::Trash)?;
        Some(edit::read(&entry, group_path(&entry)))
    }

    /// Creates (`id` is `None`) or changes an entry and saves the file.
    /// `base` is the entry as the editor opened it: only what the editor
    /// changed against it is applied, so changes another device made since
    /// stay. Returns the entry's id and the fields both changed (the
    /// editor's version won). Nothing changes when saving fails.
    /// A `template` (a new one, or one being edited) is kept in the templates'
    /// group, made when missing; the editor's group does not move it.
    /// The entry's `files` change with it, as one edit (see [edit::apply_with_files]).
    pub fn save_entry(
        &mut self,
        id: Option<&str>,
        base: Option<&EntryData>,
        data: &EntryData,
        template: bool,
        files: &[edit::FileEdit],
    ) -> Result<(String, Vec<String>), String> {
        let id = id.map(parse_id).transpose()?;
        let (id, conflicts) = self.change(|db, hidden| {
            let current = id.and_then(|id| db.entry(id));
            let template = template || current.as_ref().is_some_and(|e| is_template(e));
            if template && !files.is_empty() {
                return Err("A template's files are not changed".into());
            }
            let mut data = data.clone();
            if template {
                // Where it is now (the top for a new one): nothing moves until
                // it is put among the templates, by the group's id.
                data.group = current.as_ref().map_or_else(Vec::new, group_path);
            }
            let (saved, conflicts) = match (current, base) {
                (Some(entry), Some(base)) => {
                    let (merged, conflicts) = edit::merge3(&edit::read(&entry, group_path(&entry)), base, &data);
                    (edit::apply_with_files(db, id, &merged, files, hidden)?, conflicts)
                }
                _ => (edit::apply_with_files(db, id, &data, files, hidden)?, Vec::new()),
            };
            if template {
                edit::put_among_templates(db, saved)?;
            }
            Ok((saved, conflicts))
        })?;
        Ok((id.uuid().to_string(), conflicts))
    }

    /// Adds what another password manager exported as new entries under the
    /// group `group` (see [crate::cxf]), and saves the file. Returns how many
    /// were added.
    pub fn import(&mut self, import: &crate::cxf::Import, group: &str) -> Result<usize, String> {
        let under = [group.to_string()];
        self.change(|db, hidden| crate::cxf::add(db, import, &under, hidden))
    }

    /// Gives entries a tag or takes it off (the star is the tag Favorite), and
    /// saves the file.
    pub fn set_tag(&mut self, ids: &[String], tag: &str, on: bool) -> Result<(), String> {
        let ids = parse_ids(ids)?;
        self.change(|db, hidden| edit::set_tag(db, &ids, tag, on, hidden))
    }

    /// Renames a tag in every entry and saves the file.
    pub fn rename_tag(&mut self, from: &str, to: &str) -> Result<(), String> {
        self.change(|db, _| edit::rename_tag(db, from, to))
    }

    /// Takes a tag off every entry and saves the file.
    pub fn remove_tag(&mut self, tag: &str) -> Result<(), String> {
        self.change(|db, _| {
            edit::remove_tag(db, tag);
            Ok(())
        })
    }

    /// Puts entries from the recycle bin back where they were and saves the file.
    pub fn restore(&mut self, ids: &[String]) -> Result<(), String> {
        let ids = parse_ids(ids)?;
        self.change(|db, _| edit::restore(db, &ids))
    }

    /// Removes entries in the recycle bin for good and saves the file.
    pub fn delete_for_good(&mut self, ids: &[String]) -> Result<(), String> {
        let ids = parse_ids(ids)?;
        self.change(|db, _| {
            edit::delete_for_good(db, &ids);
            Ok(())
        })
    }

    /// Removes everything in the recycle bin for good and saves the file.
    pub fn empty_bin(&mut self) -> Result<(), String> {
        self.change(|db, _| {
            edit::empty_bin(db);
            Ok(())
        })
    }

    /// Moves entries (templates too) to the recycle bin and saves the file, as
    /// one change. An entry already gone (deleted or binned elsewhere) needs nothing.
    pub fn delete_entries(&mut self, ids: &[String]) -> Result<(), String> {
        let ids = parse_ids(ids)?;
        self.change(|db, _| {
            for &id in &ids {
                if db.entry(id).is_some_and(|entry| kind(&entry) != Kind::Trash) {
                    edit::recycle(db, id)?;
                }
            }
            Ok(())
        })
    }

    /// The file as it is now, if it changed on disk since it was last read or
    /// written: it becomes the database, and the ids of the entries that
    /// differ are returned (added, changed, moved or gone).
    pub fn reload(&mut self) -> Result<Option<Vec<String>>, OpenError> {
        let Some(since) = self.snapshot() else { return Ok(None) };
        Ok(since.read_changed()?.and_then(|read| self.adopt(&since, read)))
    }

    /// For reading the file again without holding the vault: see [Vault::adopt].
    pub fn snapshot(&self) -> Option<Snapshot> {
        self.file.as_ref().map(DbFile::snapshot)
    }

    /// Takes a changed file read since `since`, keeping this device's changes
    /// it lacks (an older file came back); those are written by the next
    /// change or [Vault::save_pending]. Returns the ids of the entries that
    /// differ from before, or `None` when the file was read or written since.
    /// A file on another key (see [KeyChange]) takes that key, or is written
    /// again with this one.
    pub fn adopt(&mut self, since: &Snapshot, read: Read) -> Option<Vec<String>> {
        let (mut db, older_key) = self.file.as_mut()?.adopt(since, read)?;
        if edit::keep_ours(&mut db, &self.db, Taking::Adopted, older_key) {
            self.unsaved = true;
        }
        let changed = changed_entries(&self.db, &db);
        self.db = db;
        Some(changed)
    }

    /// Takes the remote file `theirs` (opened from `raw`). With `merge` —
    /// this device has changes since the last sync — they are merged into it
    /// and saved ([Outcome::Merged]: the working copy then has what the remote
    /// file lacks); otherwise it replaces the working copy byte for byte
    /// ([Outcome::Downloaded]). Either carries the entries that differ from
    /// before. `None` when the working copy was written since `since`.
    /// A remote file on another key (see [KeyChange]) is never taken byte for
    /// byte: it is written with the key that goes on.
    pub fn take_remote(
        &mut self,
        since: &Snapshot,
        mut theirs: Database,
        key_change: KeyChange,
        raw: &[u8],
        merge: bool,
    ) -> Result<Option<Outcome>, String> {
        if !self.file_mut()?.is_at(since) {
            return Ok(None);
        }
        // A copy on an older key comes from before this device's change of key:
        // it is merged like an older file coming back, whatever changed here.
        let older = key_change.is_older();
        let kept_ours = (merge || older) && edit::keep_ours(&mut theirs, &self.db, Taking::Merged, older);
        // Not the remote file byte for byte: what goes up replaces it, so it is kept first.
        let rewritten = match self.file_mut()?.save_copy(&mut theirs, raw, key_change, kept_ours) {
            Ok(rewritten) => rewritten,
            Err(SaveError::Changed) => return Ok(None),
            Err(SaveError::Failed(message)) => return Err(message),
        };
        let changed = changed_entries(&self.db, &theirs);
        self.db = theirs;
        self.unsaved = false;
        Ok(Some(if rewritten { Outcome::Merged(changed) } else { Outcome::Downloaded(changed) }))
    }

    fn file(&self) -> Result<&DbFile, String> {
        self.file.as_ref().ok_or_else(|| "This database cannot be saved".into())
    }

    fn file_mut(&mut self) -> Result<&mut DbFile, String> {
        self.file.as_mut().ok_or_else(|| "This database cannot be saved".into())
    }

    /// Keeps a key the user gave for a copy that opened with no key known
    /// (another device changed it), to try on the next read of that copy.
    pub fn remember_key(&mut self, key: DatabaseKey) -> Result<(), String> {
        self.file_mut()?.remember_key(key);
        Ok(())
    }

    /// Lets go of a key [Vault::remember_key] kept, when it opened nothing.
    pub fn forget_key(&mut self, key: &DatabaseKey) -> Result<(), String> {
        self.file_mut()?.forget_key(key);
        Ok(())
    }

    /// True when the file is written with `key`.
    pub fn uses_key(&self, key: &DatabaseKey) -> Result<bool, String> {
        Ok(self.file()?.has_key(key))
    }

    /// True when the database holds changes its file lacks (kept from an
    /// older file, not written yet).
    pub fn has_unsaved(&self) -> bool {
        self.unsaved
    }

    /// Writes changes kept from an older file, if any. The file may have been
    /// replaced again meanwhile, on another key: [OpenError::OtherKey].
    pub fn save_pending(&mut self) -> Result<(), OpenError> {
        if self.unsaved {
            self.save_change(None, |_, _| Ok(()))?;
        }
        Ok(())
    }

    /// Makes a change and saves it. The change is made on the file as it is
    /// now — reloaded first, and again if it changes while saving — so a
    /// change from another device is kept, and where both touched an entry
    /// this newer one wins with the other version in the entry's history.
    /// An untouched database does not rewrite the file (and wake the sync
    /// client). Nothing changes when saving fails.
    fn change<R>(&mut self, change: impl Fn(&mut Database, &HashSet<GroupId>) -> Result<R, String>) -> Result<R, String> {
        // A command only shows why: a file on another key is not told apart.
        self.save_change(None, change).map_err(|e| e.to_string())
    }

    /// [Vault::change], the file written with `key` from now on when one is
    /// given, telling a file on another key apart.
    fn save_change<R>(
        &mut self,
        key: Option<&DatabaseKey>,
        change: impl Fn(&mut Database, &HashSet<GroupId>) -> Result<R, String>,
    ) -> Result<R, OpenError> {
        const ATTEMPTS: usize = 3;
        for _ in 0..ATTEMPTS {
            self.reload()?;
            let mut db = self.db.clone();
            let result = change(&mut db, &self.hidden_groups())?;
            if db == self.db && !self.unsaved && key.is_none() {
                return Ok(result);
            }
            let file = self.file_mut()?;
            let saved = match key {
                Some(key) => file.save_with_key(&mut db, key.clone()),
                None => file.save(&mut db),
            };
            match saved {
                Ok(()) => {
                    self.db = db;
                    self.unsaved = false;
                    return Ok(result);
                }
                Err(SaveError::Changed) => continue,
                Err(SaveError::Failed(message)) => return Err(message.into()),
            }
        }
        Err(String::from("The database file keeps changing on disk; try again in a moment").into())
    }

    /// The entry's current TOTP code, or `None` when it has no secret.
    pub fn totp(&self, id: &str) -> Result<Option<otp::Code>, String> {
        let Some(value) = self.field(id, fields::OTP) else { return Ok(None) };
        otp::Totp::parse(&value).map(|totp| Some(totp.code_now()))
    }
}

/// In the templates' group: not in the trash, a template.
fn is_template(entry: &EntryRef<'_>) -> bool {
    in_group(entry, entry.database().meta.entry_templates_group)
}

/// A template in the recycle bin is in the trash.
fn kind(entry: &EntryRef<'_>) -> Kind {
    if in_group(entry, entry.database().meta.recyclebin_uuid) {
        Kind::Trash
    } else if is_template(entry) {
        Kind::Template
    } else {
        Kind::Entry
    }
}

/// True when the entry sits in `group` (if the database has one), at any depth.
fn in_group(entry: &EntryRef<'_>, group: Option<Uuid>) -> bool {
    group.is_some_and(|g| is_in(entry, &HashSet::from([GroupId::from(g)])))
}

fn parse_id(id: &str) -> Result<EntryId, String> {
    Uuid::parse_str(id).map(EntryId::from).map_err(|_| NOT_FOUND.into())
}

fn parse_ids(ids: &[String]) -> Result<Vec<EntryId>, String> {
    ids.iter().map(|id| parse_id(id)).collect()
}

/// The ids of entries that differ between two versions of the database.
fn changed_entries(old: &Database, new: &Database) -> Vec<String> {
    let ids: HashSet<EntryId> = old.iter_all_entries().chain(new.iter_all_entries()).map(|e| e.id()).collect();
    let mut changed: Vec<String> = ids
        .into_iter()
        .filter(|&id| match (old.entry(id), new.entry(id)) {
            (Some(a), Some(b)) => *a != *b,
            (None, None) => false,
            _ => true,
        })
        .map(|id| id.uuid().to_string())
        .collect();
    changed.sort();
    changed
}

/// What the entry view shows of an entry or of one of its versions.
fn detail_of(entry: &EntryRef<'_>) -> EntryDetail {
    // Everything the summary does not carry, except the password (which has its own row).
    let mut fields: Vec<Field> = entry
        .fields
        .iter()
        .filter(|(name, value)| !in_summary(name, value) && name.as_str() != fields::PASSWORD)
        .map(|(name, value)| {
            let secret = is_secret(name, value);
            Field { name: name.clone(), protected: secret, value: (!secret).then(|| value.get().clone()) }
        })
        .collect();
    fields.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    let mut attachments: Vec<Attachment> =
        entry.attachments_named().map(|(name, a)| Attachment { name: name.to_string(), size: a.data.get().len() }).collect();
    attachments.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    let modified = entry.times.last_modification.map(edit::time_text);
    EntryDetail { summary: summary(entry), fields, attachments, modified, versions: versions_of(entry) }
}

/// A history limit as the window and the file write it: -1 for no limit.
fn as_stored(limit: Option<usize>) -> isize {
    limit.map_or(-1, |n| n as isize)
}

fn versions_of(entry: &EntryRef<'_>) -> usize {
    edit::history_count(entry)
}

/// What changed from `old` to `new`, by name: the standard fields first, then
/// the others, the tags, the icon, the expiry and the files.
fn changes(old: &EntryRef<'_>, new: &EntryRef<'_>) -> Vec<String> {
    const STANDARD: [(&str, &str); 6] = [
        (fields::TITLE, "Title"),
        (fields::USERNAME, "User name"),
        (fields::PASSWORD, "Password"),
        (fields::URL, "URL"),
        (fields::NOTES, "Notes"),
        (fields::OTP, "TOTP"),
    ];
    let differs = |name: &str| old.fields.get(name) != new.fields.get(name);
    let mut changed: Vec<String> = STANDARD.iter().filter(|(name, _)| differs(name)).map(|(_, label)| label.to_string()).collect();
    let others: BTreeSet<&String> =
        old.fields.keys().chain(new.fields.keys()).filter(|name| !STANDARD.iter().any(|(s, _)| s == name)).collect();
    let mut others: Vec<&String> = others.into_iter().filter(|name| differs(name)).collect();
    others.sort_by_key(|name| name.to_lowercase());
    changed.extend(others.into_iter().cloned());
    let files = |e: &EntryRef<'_>| {
        let mut all: Vec<(String, Zeroizing<Vec<u8>>)> =
            e.attachments_named().map(|(n, a)| (n.to_string(), Zeroizing::new(a.data.get().clone()))).collect();
        all.sort_by(|a, b| a.0.cmp(&b.0));
        all
    };
    let more = [
        (old.tags != new.tags, "Tags"),
        (old.icon() != new.icon(), "Icon"),
        (edit::expiry_of(&old.times) != edit::expiry_of(&new.times), "Expires"),
        (files(old) != files(new), "Files"),
    ];
    changed.extend(more.into_iter().filter(|(differ, _)| *differ).map(|(_, label)| label.to_string()));
    changed
}

/// Group names from the top, without the root group.
fn group_path(entry: &EntryRef<'_>) -> Vec<String> {
    edit::path_of(entry.database(), entry.parent().id())
}

/// True when the entry sits in one of `groups`, at any depth.
fn is_in(entry: &EntryRef<'_>, groups: &HashSet<GroupId>) -> bool {
    edit::ancestors(entry.database(), entry.parent().id()).iter().any(|g| groups.contains(g))
}

/// Values that only leave the backend on request: protected fields, and TOTP
/// secrets even when a client stored them unprotected (KeePassXC's `otp`,
/// KeePass's `TimeOtp-Secret*` / `HmacOtp-Secret*`, KeeTrayTOTP's `TOTP Seed`).
fn is_secret(name: &str, value: &Value<String>) -> bool {
    value.is_protected()
        || name == fields::OTP
        || name == "TOTP Seed"
        || name.starts_with("TimeOtp-Secret")
        || name.starts_with("HmacOtp-Secret")
}

/// The one rule for what the summary carries: a standard field that is not a
/// secret. Everything else is listed in the detail, masked if secret.
fn in_summary(name: &str, value: &Value<String>) -> bool {
    fields::KNOWN_FIELDS.contains(&name) && !is_secret(name, value)
}

fn summary(e: &EntryRef<'_>) -> EntrySummary {
    let text = |name: &str| match e.fields.get(name) {
        Some(value) if in_summary(name, value) => value.get().clone(),
        _ => String::new(),
    };
    let url = text(fields::URL);
    let group = group_path(e);
    EntrySummary {
        id: e.id().uuid().to_string(),
        title: text(fields::TITLE),
        username: text(fields::USERNAME),
        host: icons::host_of(&url),
        url,
        group,
        tags: e.tags.clone(),
        notes: text(fields::NOTES),
        custom_icon: e.custom_icon().map(|icon| icon.id().to_string()),
        icon: match e.icon() {
            Some(keepass::db::Icon::BuiltIn(id)) if *id != 0 => Some(*id),
            _ => None,
        },
        has_password: e.get_password().is_some_and(|p| !p.is_empty()),
        kind: kind(e),
        otp: e.fields.contains_key(fields::OTP),
        passkey: e.fields.keys().any(|name| name.starts_with(edit::passkey::PREFIX)),
        expires: edit::expiry_of(&e.times),
    }
}


/// The key from a master password, a key file, or both.
pub fn key(password: Option<&str>, key_file: Option<&Path>) -> Result<DatabaseKey, String> {
    match key_file {
        Some(path) => {
            let mut file = File::open(path).map_err(|e| format!("Cannot read the key file: {e}"))?;
            key_reading(password, Some(&mut file))
        }
        None => key_reading(password, None),
    }
}

/// [key] with the key file's content read from `key_file` (on a phone, a
/// document read into memory rather than a path).
pub fn key_reading(password: Option<&str>, key_file: Option<&mut dyn std::io::Read>) -> Result<DatabaseKey, String> {
    let mut key = DatabaseKey::new();
    if let Some(password) = password {
        key = key.with_password(password);
    }
    if let Some(key_file) = key_file {
        key = key.with_keyfile(key_file).map_err(|e| format!("Cannot read the key file: {e}"))?;
    }
    Ok(key)
}

/// Makes a new key file at `path` (never over a file already there): 32
/// random bytes in KeePass's XML key file, version 2.0, which KeePassXC and
/// Keepass2Android read too.
pub fn create_key_file(path: &Path) -> Result<(), String> {
    let mut data = Zeroizing::new([0u8; 32]);
    getrandom::fill(data.as_mut()).map_err(|e| format!("Cannot make a key: {e}"))?;
    let xml = key_file_xml(&data);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("Cannot make {}: {e}", path.display()))?;
    std::io::Write::write_all(&mut file, xml.as_bytes()).map_err(|e| format!("Cannot write {}: {e}", path.display()))
}

/// KeePass's XML key file, version 2.0: the key in hex, in groups of four
/// bytes, two lines of four, and the first four bytes of its SHA-256 to check
/// it. Written straight into memory that is wiped: the hex is the key.
fn key_file_xml(data: &[u8; 32]) -> Zeroizing<String> {
    use sha2::Digest;
    use std::fmt::Write;
    // The hash checks the key, it does not reveal it.
    let hash = crate::dbfile::hex(&sha2::Sha256::digest(data)[..4]).to_uppercase();
    let mut xml = Zeroizing::new(String::with_capacity(400));
    xml.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n<KeyFile>\r\n\t<Meta>\r\n\t\t<Version>2.0</Version>\r\n\t</Meta>\r\n\t<Key>\r\n");
    let _ = write!(xml, "\t\t<Data Hash=\"{hash}\">");
    for (i, byte) in data.iter().enumerate() {
        if i % 16 == 0 {
            xml.push_str("\r\n\t\t\t");
        } else if i % 4 == 0 {
            xml.push(' ');
        }
        let _ = write!(xml, "{byte:02X}");
    }
    xml.push_str("\r\n\t\t</Data>\r\n\t</Key>\r\n</KeyFile>\r\n");
    xml
}

/// What a new database is made with: KDBX 4.1, ChaCha20 for protected
/// values, and [encryption::DEFAULT].
fn new_database_config() -> keepass::config::DatabaseConfig {
    use keepass::config::{DatabaseConfig, DatabaseVersion};
    let mut config = DatabaseConfig::default();
    config.version = DatabaseVersion::KDB4(1);
    encryption::apply(&mut config, &encryption::DEFAULT).expect("the defaults are within bounds");
    config
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::edit::{FileChange, FileEdit};
    use zeroize::Zeroizing;

    /// A small database like the ones `sic2kdbx` writes.
    pub fn sample() -> Database {
        let mut db = Database::new();
        let mut root = db.root_mut();
        let mut work = root.add_group();
        work.name = "Work".into();
        work.add_entry().edit(|e| {
            e.set_unprotected(fields::TITLE, "Mail");
            e.set_unprotected(fields::USERNAME, "me@example.com");
            e.set_protected(fields::PASSWORD, "s3cret");
            e.set_unprotected(fields::URL, "example.com/login");
            e.set_unprotected(fields::NOTES, "work mail");
            e.set_unprotected("Recovery phone", "+0 000");
            e.set("PIN", Value::protected("pin-1234".to_string()));
            e.tags.push("Favorite".into());
        });
        root.add_entry().edit(|e| {
            e.set_unprotected(fields::TITLE, "alpha router");
            e.set_unprotected(fields::URL, "router");
        });
        let mut trash = root.add_group();
        trash.name = "Recycle Bin".into();
        let trash_id = trash.id();
        trash.add_entry().edit(|e| e.set_unprotected(fields::TITLE, "Deleted"));
        db.meta.recyclebin_uuid = Some(trash_id.uuid());
        db
    }

    /// The listed entries in use: not templates, not in the recycle bin.
    pub fn in_use(vault: &Vault) -> Vec<EntrySummary> {
        vault.listing().entries.into_iter().filter(|e| e.kind == Kind::Entry).collect()
    }

    fn kind_of(vault: &Vault, id: &str) -> Option<Kind> {
        vault.listing().entries.into_iter().find(|e| e.id == id).map(|e| e.kind)
    }

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
        assert_eq!(detail.modified, Some(edit::time_text(modified)));
        assert!(detail.modified.as_deref().is_some_and(|t| t.ends_with('Z')));
        assert_eq!(
            detail.fields,
            [
                Field { name: "PIN".into(), value: None, protected: true },
                Field { name: "Recovery phone".into(), value: Some("+0 000".into()), protected: false },
            ]
        );
        assert_eq!(vault.field(&id, fields::PASSWORD).unwrap().as_str(), "s3cret");
        assert_eq!(vault.field(&id, "PIN").unwrap().as_str(), "pin-1234");
        assert!(vault.field(&id, "Nope").is_none());
    }

    #[test]
    fn recycled_entries_show_but_do_not_edit() {
        let db = sample();
        let trash = db.meta.recyclebin_uuid.unwrap();
        let deleted = db.iter_all_entries().find(|e| e.parent().id().uuid() == trash).unwrap().id().uuid().to_string();
        let vault = Vault::from_database(db);
        assert_eq!(vault.detail(&deleted).unwrap().summary.title, "Deleted");
        assert!(vault.edit_data(&deleted).is_none());
        assert!(vault.detail("not-a-uuid").is_none());
    }

    #[test]
    fn templates_are_made_edited_and_deleted_as_templates() {
        let dir = tempfile::tempdir().unwrap();
        let (db, file) = crate::dbfile::tests::saved(dir.path(), &sample());
        let mut vault = Vault { db, file: Some(file), unsaved: false };
        let mut card = EntryData::default();
        card.title = "Card".into();
        card.group = vec!["Work".into()]; // ignored: a template goes among the templates
        let (id, _) = vault.save_entry(None, None, &card, true, &[]).unwrap();
        // A template's files are not changed.
        let file = FileChange::Add { name: "a.txt".into(), content: Zeroizing::new(b"a".to_vec()) };
        assert!(vault.save_entry(Some(&id), None, &card, false, &[file]).is_err());
        assert_eq!(kind_of(&vault, &id), Some(Kind::Template));
        let templates = vault.db.meta.entry_templates_group.expect("made on demand");
        assert_eq!(vault.db.group(GroupId::from(templates)).unwrap().name, "Templates");

        // Edited like an entry; it stays a template.
        let mut edited = vault.edit_data(&id).unwrap();
        edited.username = "holder".into();
        vault.save_entry(Some(&id), None, &edited, false, &[]).unwrap();
        assert_eq!(kind_of(&vault, &id), Some(Kind::Template));
        assert_eq!(vault.field(&id, fields::USERNAME).unwrap().as_str(), "holder");

        // The editor's group does not move it out.
        edited.group = vec!["Work".into()];
        vault.save_entry(Some(&id), None, &edited, true, &[]).unwrap();
        assert_eq!(kind_of(&vault, &id), Some(Kind::Template));

        // Deleted into the trash, where it is not edited.
        vault.delete_entries(std::slice::from_ref(&id)).unwrap();
        assert_eq!(kind_of(&vault, &id), Some(Kind::Trash));
        assert!(vault.edit_data(&id).is_none());
        // Deleted elsewhere while the editor had it open: saving brings it back.
        vault.save_entry(Some(&id), None, &edited, true, &[]).unwrap();
        assert_eq!(kind_of(&vault, &id), Some(Kind::Template));
    }

    #[test]
    fn history_lists_versions_shows_one_and_restores_it() {
        let dir = tempfile::tempdir().unwrap();
        let (db, file) = crate::dbfile::tests::saved(dir.path(), &sample());
        let mut vault = Vault { db, file: Some(file), unsaved: false };
        let id = in_use(&vault).into_iter().find(|e| e.title == "Mail").unwrap().id;
        let mut data = vault.edit_data(&id).unwrap();
        data.password = "newer".into();
        data.url = "example.org".into();
        vault.save_entry(Some(&id), None, &data, false, &[]).unwrap();
        add_file(&mut vault, &id, "a.txt", b"file");

        assert_eq!(vault.detail(&id).unwrap().versions, 2);
        let history = vault.history(&id).unwrap();
        assert_eq!(history.iter().map(|v| v.changed.clone()).collect::<Vec<_>>(), [vec!["Files"], vec!["Password", "URL"]]);
        assert!(history[1].modified.is_some());
        let json = serde_json::to_string(&history).unwrap();
        assert!(!json.contains("s3cret") && !json.contains("newer"), "{json}");

        // The oldest version: its values on request, how it differs from now.
        let old = vault.version(&id, 1).unwrap();
        assert_eq!(old.detail.summary.url, "example.com/login");
        assert!(old.detail.attachments.is_empty());
        assert_eq!(
            old.differs,
            [
                Difference { name: fields::PASSWORD.into(), current: None, protected: true },
                Difference { name: fields::URL.into(), current: Some("example.org".into()), protected: false },
            ]
        );
        assert_eq!(vault.field_in(&id, Some(1), fields::PASSWORD).unwrap().as_str(), "s3cret");
        assert_eq!(vault.attachment(&id, Some(0), "a.txt"), None);
        assert!(vault.version(&id, 5).is_none());

        // Restored: its content is current again, the replaced one in history.
        // The version shown is checked: the history may have moved on meanwhile.
        assert!(vault.restore_version(&id, 1, Some("2000-01-01T00:00:00Z")).unwrap_err().contains("history changed"));
        vault.restore_version(&id, 1, history[1].modified.as_deref()).unwrap();
        assert_eq!(vault.field(&id, fields::PASSWORD).unwrap().as_str(), "s3cret");
        assert!(vault.detail(&id).unwrap().attachments.is_empty());
        assert_eq!(vault.detail(&id).unwrap().versions, 3);
        assert_eq!(vault.field_in(&id, Some(0), fields::PASSWORD).unwrap().as_str(), "newer");
        assert_eq!(vault.attachment(&id, Some(0), "a.txt").unwrap().as_slice(), b"file");
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
            e.times.expiry = chrono::NaiveDate::from_ymd_opt(2030, 1, 2).unwrap().and_hms_opt(3, 4, 5);
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
        let json = serde_json::to_string(&listing.entries).unwrap();
        assert!(!json.contains("JBSWY3DP") && !json.contains("\"key\""), "{json}");
    }

    #[test]
    fn opens_a_saved_file_and_reports_a_wrong_password() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.kdbx");
        sample().save(&mut File::create(&path).unwrap(), DatabaseKey::new().with_password("pw")).unwrap();
        let vault = Vault::open(&path, Some("pw"), None).unwrap();
        assert_eq!(in_use(&vault).len(), 2);
        assert_eq!(Vault::open(&path, Some("nope"), None).err().unwrap(), "Wrong password or key file");
        assert_eq!(Vault::open(&path, None, None).err().unwrap(), "Enter the password or choose a key file");
    }

    fn fixture_path(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
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
    fn refuses_a_file_it_cannot_read_safely() {
        let refused = Vault::open(&fixture_path("unordered.kdbx"), Some("test"), None);
        assert_eq!(refused.err().unwrap(), crate::dbfile::UNORDERED);
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
        assert_eq!(vault.web_url(id).unwrap().as_str(), "https://bank.example.com/");
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

    /// A fixture copied to a temporary folder, opened with its password.
    fn fixture(name: &str, dir: &Path) -> Vault {
        let path = dir.join(name);
        std::fs::copy(fixture_path(name), &path).unwrap();
        Vault::open(&path, Some("test"), None).unwrap()
    }

    #[test]
    fn saving_keeps_everything_the_app_does_not_show() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let before = vault.db.clone();
        vault.file.as_mut().unwrap().save(&mut before.clone()).unwrap();
        vault.db.config.version = keepass::config::DatabaseVersion::KDB4(1);
        let reopened = Vault::open(&dir.path().join("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        assert_eq!(reopened.db, vault.db);
        // Everything but the format version, which saving moves to 4.1.
        let mut expected = before;
        expected.config.version = vault.db.config.version.clone();
        assert_eq!(reopened.db, expected);
    }

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

        let response = vault.sign_with_passkey(&id, r#"{"rpId": "example.com", "challenge": "c2lnbg"}"#, &caller).unwrap();
        let json: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["id"], credential_id.as_str());
        assert_eq!(json["response"]["userHandle"], "dQ");
        assert!(vault.sign_with_passkey(&id, r#"{"rpId": "other.com", "challenge": "Yw"}"#, &caller).is_err());
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

    #[test]
    fn logins_are_offered_to_their_app_or_site_and_saved() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let app = LoginPlace::App("com.example.mail".into());
        let site = LoginPlace::Site("www.mail.example".into());
        assert!(vault.logins_for(&app).is_empty());

        // A new login for the app, then offered to it (and not to others).
        let id = vault.save_login(None, &app, "Mail", "alice", "pw-1").unwrap();
        let offered = vault.logins_for(&app);
        assert_eq!((offered[0].id.as_str(), offered[0].username.as_str()), (id.as_str(), "alice"));
        assert!(vault.logins_for(&LoginPlace::App("com.example.other".into())).is_empty());
        assert!(vault.logins_for(&LoginPlace::App("com.example.MAIL".into())).is_empty());
        let (user, password) = vault.login(&id).unwrap();
        assert_eq!((user.as_str(), password.as_str()), ("alice", "pw-1"));

        // An app's changed password updates the entry; the user name stays.
        assert_eq!(vault.entries_for_login(&app).len(), 1);
        vault.save_login(Some(&id), &app, "", "someone", "pw-2").unwrap();
        let (user, password) = vault.login(&id).unwrap();
        assert_eq!((user.as_str(), password.as_str()), ("alice", "pw-2"));

        // A site's login is offered to its own host (a www. aside), not to the site's other hosts.
        let web = vault.save_login(None, &LoginPlace::Site("mail.example".into()), "Mail web", "bob", "pw-3").unwrap();
        assert_eq!(vault.logins_for(&site).iter().map(|c| c.id.clone()).collect::<Vec<_>>(), [web]);
        assert!(vault.logins_for(&LoginPlace::Site("evil.mail.example".into())).is_empty());
        assert!(vault.save_login(None, &app, "Mail", "alice", "").is_err());
        assert_eq!(LoginPlace::of(Some("https://www.mail.example"), "com.android.chrome").unwrap(), LoginPlace::Site("www.mail.example".into()));
        assert_eq!(LoginPlace::of(None, "com.example.mail").unwrap(), app);
    }

    #[test]
    fn an_import_is_saved_and_reads_back_with_its_passkey() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let before = vault.listing().entries.len();
        let import = crate::cxf::parse(
            r#"{"version": {"major": 1, "minor": 0}, "exporterRpId": "example.com", "accounts": [{"id": "YQ",
              "username": "", "email": "", "collections": [], "items": [{"id": "MQ", "title": "Site", "credentials": [
                {"type": "passkey", "credentialId": "Y3JlZA", "rpId": "site.example", "username": "bob",
                 "userDisplayName": "Bob", "userHandle": "dXNlcg", "key": "AAECAwQF"}]}]}]}"#,
        )
        .unwrap();
        assert_eq!(vault.import(&import, "Imported").unwrap(), 1);
        let reopened = Vault::open(&dir.path().join("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        let listing = reopened.listing();
        assert_eq!(listing.entries.len(), before + 1);
        let site = listing.entries.iter().find(|e| e.title == "Site").unwrap();
        assert!(site.passkey);
        assert_eq!(site.group, ["Imported"]);
    }

    #[test]
    fn editing_an_entry_keeps_its_attachments_history_and_custom_data() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let id = vault.listing().entries.iter().find(|e| e.title == "Router").unwrap().id.clone();
        let old = vault.db.entry(EntryId::from(Uuid::parse_str(&id).unwrap())).unwrap().clone();

        let mut data = vault.edit_data(&id).unwrap();
        data.password = "changed".into();
        vault.save_entry(Some(&id), None, &data, false, &[]).unwrap();

        let reopened = Vault::open(&dir.path().join("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        let entry = reopened.db.entry(old.id()).unwrap();
        assert_eq!(entry.get_password(), Some("changed"));
        assert_eq!(entry.attachments().count(), 2);
        assert_eq!(entry.custom_data, old.custom_data);
        assert_eq!(entry.history.as_ref().unwrap().get_entries().len(), old.history.as_ref().unwrap().get_entries().len() + 1);
        assert_eq!(entry.get(fields::OTP), old.get(fields::OTP));
        // Templates and the recycle bin stay where they were.
        assert_eq!(in_use(&reopened).len(), 3);
        assert!(std::fs::metadata(dir.path().join("sic2kdbx.kdbx.bak")).is_ok());
    }

    #[test]
    fn attaching_a_file_saves_it_and_keeps_the_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let all_files = |v: &Vault| {
            let mut all: Vec<(String, String, Vec<u8>)> = v
                .db
                .iter_all_entries()
                .flat_map(|e| {
                    let title = e.get_title().unwrap_or_default().to_string();
                    e.attachments_named().map(|(n, a)| (title.clone(), n.to_string(), a.data.get().clone())).collect::<Vec<_>>()
                })
                .collect();
            all.sort();
            all
        };
        let before = all_files(&vault);
        let mail = id_of(&vault, "Mail").uuid().to_string();
        assert!(vault.detail(&mail).unwrap().attachments.is_empty());

        add_file(&mut vault, &mail, "key.txt", b"recovery codes");
        let reopened = Vault::open(&dir.path().join("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        assert_eq!(reopened.detail(&mail).unwrap().attachments, [Attachment { name: "key.txt".into(), size: 14 }]);
        assert_eq!(reopened.attachment(&mail, None, "key.txt").unwrap().as_slice(), b"recovery codes");
        assert!(reopened.attachment(&mail, None, "other.txt").is_none());
        let mut expected = before;
        expected.push(("Mail".into(), "key.txt".into(), b"recovery codes".to_vec()));
        expected.sort();
        assert_eq!(all_files(&reopened), expected);
        // The router's files are listed, by name.
        let router = id_of(&reopened, "Router").uuid().to_string();
        assert_eq!(reopened.detail(&router).unwrap().attachments.len(), 2);
    }

    /// Saves the entry unchanged with these file changes, as the editor does.
    fn change_files(vault: &mut Vault, id: &str, changes: Vec<FileEdit>) {
        let data = vault.edit_data(id).unwrap();
        vault.save_entry(Some(id), None, &data, false, &changes).unwrap();
    }

    fn add_file(vault: &mut Vault, id: &str, name: &str, content: &[u8]) {
        change_files(vault, id, vec![FileChange::Add { name: name.into(), content: Zeroizing::new(content.to_vec()) }]);
    }

    #[test]
    fn removing_a_file_keeps_it_in_history_and_the_other_files_intact() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let router = id_of(&vault, "Router");
        let id = router.uuid().to_string();
        let mail = id_of(&vault, "Mail").uuid().to_string();
        // A file after the router's in the database: it must keep its data
        // when one of the router's is removed.
        add_file(&mut vault, &mail, "later.txt", b"attached later");
        let files = |v: &Vault, id: EntryId| {
            let mut all: Vec<(String, Vec<u8>)> =
                v.db.entry(id).unwrap().attachments_named().map(|(n, a)| (n.to_string(), a.data.get().clone())).collect();
            all.sort();
            all
        };
        let before = files(&vault, router);
        let (gone, kept) = (before[0].clone(), before[1].clone());

        change_files(&mut vault, &id, vec![FileChange::Remove { name: gone.0.clone() }, FileChange::Remove { name: "no such file".into() }]);
        let reopened = Vault::open(&dir.path().join("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        assert_eq!(files(&reopened, router), [kept]);
        assert_eq!(reopened.detail(&id).unwrap().attachments.len(), 1);
        assert_eq!(reopened.attachment(&mail, None, "later.txt").unwrap().as_slice(), b"attached later");
        let previous = reopened.db.entry(router).unwrap().historical(0).unwrap().attachments_named().map(|(n, a)| (n.to_string(), a.data.get().clone())).collect::<Vec<_>>();
        assert!(previous.contains(&gone), "{previous:?}");
        // Attaching and renaming after a removal still number the files right.
        let mut reopened = reopened;
        add_file(&mut reopened, &id, "new.txt", b"new");
        change_files(&mut reopened, &mail, vec![FileChange::Rename { name: "later.txt".into(), to: "renamed.txt".into() }]);
        let again = Vault::open(&dir.path().join("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        assert_eq!(again.attachment(&id, None, "new.txt").unwrap().as_slice(), b"new");
        assert_eq!(again.attachment(&mail, None, "renamed.txt").unwrap().as_slice(), b"attached later");
        assert!(again.attachment(&mail, None, "later.txt").is_none());
        let mut again = again;
        let replaced = Zeroizing::new(b"replaced".to_vec());
        change_files(&mut again, &mail, vec![FileChange::Remove { name: "renamed.txt".into() }, FileChange::Add { name: "renamed.txt".into(), content: replaced }]);
        let last = Vault::open(&dir.path().join("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        assert_eq!(last.attachment(&mail, None, "renamed.txt").unwrap().as_slice(), b"replaced");
        assert_eq!(last.attachment(&id, None, "new.txt").unwrap().as_slice(), b"new");
    }

    #[test]
    fn saving_an_untouched_entry_leaves_the_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let id = in_use(&vault)[0].id.clone();
        let data = vault.edit_data(&id).unwrap();
        vault.save_entry(Some(&id), None, &data, false, &[]).unwrap();
        assert!(!dir.path().join("sic2kdbx.kdbx.bak").exists());
    }

    #[test]
    fn a_chosen_icon_saves_and_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sic2kdbx.kdbx");
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let mail = id_of(&vault, "Mail").uuid().to_string();
        let router = id_of(&vault, "Router").uuid().to_string();
        let png = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0]);
        for (id, icon) in [(&mail, edit::IconChoice::Custom { data: png }), (&router, edit::IconChoice::Builtin { id: 37 })] {
            let mut data = vault.edit_data(id).unwrap();
            data.icon = icon;
            vault.save_entry(Some(id), None, &data, false, &[]).unwrap();
        }
        let reopened = Vault::open(&path, Some("test"), None).unwrap();
        let listing = reopened.listing();
        let summary = |id: &str| listing.entries.iter().find(|e| e.id == id).unwrap().clone();
        assert!(listing.custom_icons[summary(&mail).custom_icon.as_ref().unwrap()].starts_with("data:image/png"));
        assert_eq!(summary(&router).icon, Some(37));
    }

    #[test]
    fn a_new_database_opens_and_is_never_made_over_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new").join("Passwords.kdbx");
        Vault::create(&path, "Passwords", Some("pw"), None).unwrap();
        let mut vault = Vault::open(&path, Some("pw"), None).unwrap();
        assert!(vault.listing().entries.is_empty());
        assert!(matches!(vault.db.config.kdf_config, keepass::config::KdfConfig::Argon2id { .. }));
        // It takes entries, and deleting one makes the recycle bin.
        let mut first = edit::EntryData::default();
        first.title = "First".into();
        let (id, _) = vault.save_entry(None, None, &first, false, &[]).unwrap();
        vault.delete_entries(std::slice::from_ref(&id)).unwrap();
        assert!(Vault::create(&path, "Again", Some("pw"), None).unwrap_err().contains("already there"));
        assert!(Vault::create(&dir.path().join("b.kdbx"), "B", None, None).is_err());
    }

    #[test]
    fn deleting_saves_and_moves_the_entry_to_the_bin() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let id = in_use(&vault)[0].id.clone();
        vault.delete_entries(std::slice::from_ref(&id)).unwrap();
        let reopened = Vault::open(&dir.path().join("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        assert_eq!(kind_of(&reopened, &id), Some(Kind::Trash));
        assert_eq!(in_use(&reopened).len(), 2);
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

    /// Another device changes the file: it is read, changed and written as a
    /// whole, the way a sync client delivers it.
    fn elsewhere(path: &Path, change: impl FnOnce(&mut Database)) {
        let key = || DatabaseKey::new().with_password("test");
        let mut db = Database::open(&mut File::open(path).unwrap(), key()).unwrap();
        change(&mut db);
        db.config.version = keepass::config::DatabaseVersion::KDB4(1); // the only version keepass-rs writes
        db.save(&mut File::create(path).unwrap(), key()).unwrap();
    }

    fn id_of(vault: &Vault, title: &str) -> EntryId {
        let id = vault.listing().entries.into_iter().find(|e| e.title == title).unwrap().id;
        parse_id(&id).unwrap()
    }

    #[test]
    fn a_change_from_elsewhere_is_kept_when_saving() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sic2kdbx.kdbx");
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        elsewhere(&path, |db| {
            db.root_mut().add_entry().set_unprotected(fields::TITLE, "Added on the phone");
        });
        let mail = id_of(&vault, "Mail").uuid().to_string();
        let mut data = vault.edit_data(&mail).unwrap();
        data.password = "from this PC".into();
        vault.save_entry(Some(&mail), None, &data, false, &[]).unwrap();

        let reopened = Vault::open(&path, Some("test"), None).unwrap();
        let titles: Vec<String> = reopened.listing().entries.into_iter().map(|e| e.title).collect();
        assert!(titles.contains(&"Added on the phone".to_string()), "{titles:?}");
        assert_eq!(reopened.field(&mail, fields::PASSWORD).unwrap().as_str(), "from this PC");
    }

    #[test]
    fn when_both_change_an_entry_this_edit_wins_and_the_other_goes_to_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sic2kdbx.kdbx");
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let mail = id_of(&vault, "Mail");
        let base = vault.edit_data(&mail.uuid().to_string()).unwrap(); // the editor opens
        elsewhere(&path, |db| {
            db.entry_mut(mail).unwrap().track_changes().set_protected(fields::PASSWORD, "from the phone");
        });
        let mut data = base.clone();
        data.password = "from this PC".into();
        let (_, conflicts) = vault.save_entry(Some(&mail.uuid().to_string()), Some(&base), &data, false, &[]).unwrap();
        assert_eq!(conflicts, ["Password"]);

        let reopened = Vault::open(&path, Some("test"), None).unwrap();
        let entry = reopened.db.entry(mail).unwrap();
        assert_eq!(entry.get_password(), Some("from this PC"));
        assert_eq!(entry.history.as_ref().unwrap().get_entries()[0].get_password(), Some("from the phone"));
    }

    #[test]
    fn an_entry_deleted_elsewhere_and_edited_here_comes_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sic2kdbx.kdbx");
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let mail = id_of(&vault, "Mail");
        let data = vault.edit_data(&mail.uuid().to_string()).unwrap();
        // Keepass2Android moves a deleted entry to its recycle bin.
        elsewhere(&path, |db| {
            let bin = db.recycle_bin().unwrap().id();
            db.entry_mut(mail).unwrap().move_to(bin).unwrap();
        });
        vault.save_entry(Some(&mail.uuid().to_string()), None, &data, false, &[]).unwrap();
        let reopened = Vault::open(&path, Some("test"), None).unwrap();
        assert_eq!(kind_of(&reopened, &mail.uuid().to_string()), Some(Kind::Entry));
    }

    #[test]
    fn reload_reports_what_changed_and_deleting_twice_is_fine() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sic2kdbx.kdbx");
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        assert_eq!(vault.reload().unwrap(), None);
        let mail = id_of(&vault, "Mail");
        elsewhere(&path, |db| {
            let bin = db.recycle_bin().unwrap().id();
            db.entry_mut(mail).unwrap().move_to(bin).unwrap();
        });
        assert_eq!(vault.reload().unwrap(), Some(vec![mail.uuid().to_string()]));
        assert_eq!(kind_of(&vault, &mail.uuid().to_string()), Some(Kind::Trash));
        vault.delete_entries(&[mail.uuid().to_string()]).unwrap();
    }

    #[test]
    fn a_saved_change_survives_an_older_file_coming_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sic2kdbx.kdbx");
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let older = std::fs::read(&path).unwrap();
        let mail = id_of(&vault, "Mail").uuid().to_string();
        let mut data = vault.edit_data(&mail).unwrap();
        data.password = "saved here".into();
        vault.save_entry(Some(&mail), None, &data, false, &[]).unwrap();

        std::fs::write(&path, &older).unwrap(); // a sync client brings the old copy back
        vault.reload().unwrap();
        assert_eq!(vault.field(&mail, fields::PASSWORD).unwrap().as_str(), "saved here");
        vault.save_pending().unwrap();
        let reopened = Vault::open(&path, Some("test"), None).unwrap();
        assert_eq!(reopened.field(&mail, fields::PASSWORD).unwrap().as_str(), "saved here");
    }

    #[test]
    fn a_new_master_password_opens_the_file_and_the_old_one_no_longer_does() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sic2kdbx.kdbx");
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let wrong = vault.change_key((Some("nope"), None), Some("new one"), None).unwrap_err();
        assert!(wrong.contains("not right"), "{wrong}");
        assert!(vault.change_key((Some("test"), None), None, None).is_err(), "a database needs a key");

        vault.change_key((Some("test"), None), Some("new one"), None).unwrap();
        assert!(vault.db.meta.master_key_changed.is_some());
        // The file's key is the new one from now on.
        assert!(vault.change_key((Some("test"), None), Some("x"), None).is_err());
        assert_eq!(Vault::open(&path, Some("test"), None).err().unwrap(), "Wrong password or key file");
        let reopened = Vault::open(&path, Some("new one"), None).unwrap();
        assert_eq!(in_use(&reopened).len(), in_use(&vault).len());
        // The previous file, with the old key, is the backup.
        assert!(Vault::open(&dir.path().join("sic2kdbx.kdbx.bak"), Some("test"), None).is_ok());
    }

    #[test]
    fn new_encryption_is_saved_and_opens_with_the_same_key() {
        use crate::encryption::{Cipher, Encryption, Kdf};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sic2kdbx.kdbx");
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let wanted = Encryption { cipher: Cipher::ChaCha20, kdf: Kdf::Argon2d, iterations: 3, memory: 8 << 20, parallelism: 1 };
        vault.set_encryption(&wanted).unwrap();
        let reopened = Vault::open(&path, Some("test"), None).unwrap();
        assert_eq!(reopened.settings().encryption, wanted);
        assert_eq!(in_use(&reopened).len(), in_use(&vault).len());
    }

    #[test]
    fn a_working_copy_on_a_newer_key_asks_for_it_and_takes_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sic2kdbx.kdbx");
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        // Another program changes the key of the file on this PC.
        let mut db = vault.db.clone();
        db.meta.master_key_changed = Some(keepass::db::Times::now());
        db.config.version = keepass::config::DatabaseVersion::KDB4(1);
        db.save(&mut File::create(&path).unwrap(), DatabaseKey::new().with_password("other")).unwrap();

        let refused = vault.reload().unwrap_err();
        assert!(matches!(refused, OpenError::OtherKey(_)), "{refused}");
        vault.remember_key(DatabaseKey::new().with_password("other")).unwrap();
        vault.reload().unwrap();
        assert!(vault.uses_key(&DatabaseKey::new().with_password("other")).unwrap());
    }

    #[test]
    fn an_older_copy_on_the_old_key_is_written_again_with_the_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sic2kdbx.kdbx");
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let older = std::fs::read(&path).unwrap();
        vault.change_key((Some("test"), None), Some("new"), None).unwrap();

        std::fs::write(&path, &older).unwrap(); // a sync client brings the old copy back
        vault.reload().unwrap();
        assert!(vault.has_unsaved());
        vault.save_pending().unwrap();
        assert!(Vault::open(&path, Some("new"), None).is_ok());
        assert!(Vault::open(&path, Some("test"), None).is_err());
    }

    #[test]
    fn saving_what_was_kept_tells_a_file_on_another_key_apart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sic2kdbx.kdbx");
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let older = std::fs::read(&path).unwrap();
        vault.change_key((Some("test"), None), Some("new"), None).unwrap();
        std::fs::write(&path, &older).unwrap();
        vault.reload().unwrap();

        // Before it is written, the file is replaced again, on a key not known here.
        let mut db = vault.db.clone();
        db.config.version = keepass::config::DatabaseVersion::KDB4(1);
        db.save(&mut File::create(&path).unwrap(), DatabaseKey::new().with_password("other")).unwrap();
        assert!(matches!(vault.save_pending(), Err(OpenError::OtherKey(_))));
    }

    #[test]
    fn a_new_key_file_alone_opens_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sic2kdbx.kdbx");
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let key_file = dir.path().join("PswManager.keyx");
        create_key_file(&key_file).unwrap();
        assert!(create_key_file(&key_file).is_err(), "never over a file already there");

        vault.change_key((Some("test"), None), None, Some(&key_file)).unwrap();
        assert!(Vault::open(&path, None, Some(&key_file)).is_ok());
        assert!(Vault::open(&path, Some("test"), None).is_err());
    }

    #[test]
    fn a_key_file_read_into_memory_opens_the_file_as_its_path_does() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sic2kdbx.kdbx");
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let key_file = dir.path().join("PswManager.keyx");
        create_key_file(&key_file).unwrap();
        vault.change_key((Some("test"), None), Some("pw"), Some(&key_file)).unwrap();
        let content = std::fs::read(&key_file).unwrap();
        let key = |password| key_reading(password, Some(&mut content.as_slice())).unwrap();
        assert!(Vault::open_with_key(&path, key(Some("pw"))).is_ok());
        assert!(Vault::open_with_key(&path, key(None)).is_err());
    }

    #[test]
    fn a_key_file_carries_the_hash_keepass_checks() {
        let data: [u8; 32] = std::array::from_fn(|i| i as u8);
        let xml = key_file_xml(&data);
        let hash = crate::dbfile::hex(&<sha2::Sha256 as sha2::Digest>::digest(data)[..4]).to_uppercase();
        assert!(xml.contains(&format!("<Data Hash=\"{hash}\">")), "{}", xml.as_str());
        assert!(xml.contains("00010203 04050607 08090A0B 0C0D0E0F\r\n"), "{}", xml.as_str());
        assert!(xml.contains("<Version>2.0</Version>"));
    }

    #[test]
    fn versions_another_device_kept_are_trimmed_by_the_next_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sic2kdbx.kdbx");
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        vault.set_history_limits(1, 1 << 20).unwrap();
        let mail = id_of(&vault, "Mail");
        // A client that ignores the limit keeps three versions.
        elsewhere(&path, |db| {
            for i in 0..3 {
                db.entry_mut(mail).unwrap().edit_tracking(|e| e.set_unprotected(fields::NOTES, format!("phone {i}")));
            }
        });
        let router = id_of(&vault, "Router").uuid().to_string();
        let mut data = vault.edit_data(&router).unwrap();
        data.notes = "a change here".into();
        vault.save_entry(Some(&router), None, &data, false, &[]).unwrap();

        let reopened = Vault::open(&path, Some("test"), None).unwrap();
        assert_eq!(reopened.detail(&mail.uuid().to_string()).unwrap().versions, 1);
    }

    #[test]
    fn history_limits_outside_the_lists_are_refused_unless_the_file_has_them() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        assert!(vault.set_history_limits(500, 6 << 20).is_err());
        assert!(vault.set_history_limits(10, 1).is_err());
        vault.set_history_limits(0, 1 << 20).unwrap();
        // Another client's own limit is kept while the other one changes.
        vault.db.meta.history_max_items = Some(-1);
        vault.set_history_limits(-1, 2 << 20).unwrap();
        assert_eq!((vault.settings().history_max_items, vault.settings().history_max_size), (-1, 2 << 20));
    }

    #[test]
    fn a_setting_changed_here_survives_an_older_file_coming_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sic2kdbx.kdbx");
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let older = std::fs::read(&path).unwrap();
        vault.set_setting(edit::Setting::Name, "Named here").unwrap();

        std::fs::write(&path, &older).unwrap(); // a sync client brings the old copy back
        vault.reload().unwrap();
        assert_eq!(vault.settings().name, "Named here");
        assert!(vault.has_unsaved());
        vault.save_pending().unwrap();
        assert_eq!(Vault::open(&path, Some("test"), None).unwrap().settings().name, "Named here");
    }

    #[test]
    fn nothing_is_saved_while_the_file_cannot_be_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sic2kdbx.kdbx");
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        std::fs::write(&path, b"half-synced").unwrap();
        let mail = id_of(&vault, "Mail").uuid().to_string();
        let mut data = vault.edit_data(&mail).unwrap();
        data.password = "x".into();
        assert!(vault.save_entry(Some(&mail), None, &data, false, &[]).unwrap_err().contains("cannot be read now"));
        assert_eq!(std::fs::read(&path).unwrap(), b"half-synced");
    }

    #[test]
    fn moving_an_entry_to_another_group_saves() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let router = id_of(&vault, "Router").uuid().to_string();
        let mut data = vault.edit_data(&router).unwrap();
        data.group = vec!["Elsewhere".into()];
        data.password = "and changed".into();
        vault.save_entry(Some(&router), None, &data, false, &[]).unwrap();
        let reopened = Vault::open(&dir.path().join("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        let detail = reopened.detail(&router).unwrap();
        assert_eq!(detail.summary.group, ["Elsewhere"]);
        assert_eq!(reopened.field(&router, fields::PASSWORD).unwrap().as_str(), "and changed");
    }

    #[test]
    fn a_stale_editor_keeps_the_fields_it_did_not_touch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sic2kdbx.kdbx");
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let router = id_of(&vault, "Router");
        let base = vault.edit_data(&router.uuid().to_string()).unwrap(); // the editor opens
        elsewhere(&path, |db| {
            db.entry_mut(router).unwrap().track_changes().set_unprotected(fields::USERNAME, "changed on the phone");
            let mut root = db.root_mut();
            let mut job = root.add_group();
            job.name = "Job".into();
            let job = job.id();
            db.entry_mut(router).unwrap().move_to(job).unwrap();
        });
        let mut data = base.clone();
        data.password = "changed on the PC".into();
        let (_, conflicts) = vault.save_entry(Some(&router.uuid().to_string()), Some(&base), &data, false, &[]).unwrap();
        assert!(conflicts.is_empty());

        let reopened = Vault::open(&path, Some("test"), None).unwrap();
        let id = router.uuid().to_string();
        assert_eq!(reopened.field(&id, fields::USERNAME).unwrap().as_str(), "changed on the phone");
        assert_eq!(reopened.field(&id, fields::PASSWORD).unwrap().as_str(), "changed on the PC");
        assert_eq!(reopened.detail(&id).unwrap().summary.group, ["Job"]);
    }
}
