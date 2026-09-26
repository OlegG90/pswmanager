//! The unlocked database. Everything decrypted stays here: the frontend gets
//! summaries without secrets, a secret only when asked for one field, and a
//! whole entry only while it is being edited.

use crate::dbfile::DbFile;
use crate::edit::{self, EntryData};
use crate::{icons, otp};
use keepass::db::{fields, EntryId, EntryRef, GroupId, Value};
use keepass::config::DatabaseVersion;
use keepass::{Database, DatabaseKey};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::path::Path;
use uuid::Uuid;
use zeroize::Zeroizing;

pub struct Vault {
    db: Database,
    /// The file it came from; `None` only in tests that never save.
    file: Option<DbFile>,
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
    pub has_password: bool,
}

/// An additional attribute. A protected one comes without its value.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    pub name: String,
    pub value: Option<String>,
    pub protected: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryDetail {
    #[serde(flatten)]
    pub summary: EntrySummary,
    pub fields: Vec<Field>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub entries: Vec<EntrySummary>,
    /// The database's own icons used by the entries, as `data:` URLs.
    pub custom_icons: BTreeMap<String, String>,
}

impl Vault {
    /// Opens a KDBX file with a password, a key file, or both.
    pub fn open(path: &Path, password: Option<&str>, key_file: Option<&Path>) -> Result<Vault, String> {
        let mut key = DatabaseKey::new();
        if let Some(password) = password {
            key = key.with_password(password);
        }
        if let Some(key_file) = key_file {
            let mut file = File::open(key_file).map_err(|e| format!("Cannot read the key file: {e}"))?;
            key = key.with_keyfile(&mut file).map_err(|e| format!("Cannot read the key file: {e}"))?;
        }
        let (db, file) = DbFile::open(path, key)?;
        Ok(Vault { db, file: Some(file) })
    }

    #[cfg(test)]
    pub fn from_database(db: Database) -> Vault {
        Vault { db, file: None }
    }

    /// All entries the user works with: not in the recycle bin, not templates.
    fn visible_entries(&self) -> impl Iterator<Item = EntryRef<'_>> {
        let hidden = self.hidden_groups();
        self.db.iter_all_entries().filter(move |e| !is_in(e, &hidden))
    }

    fn hidden_groups(&self) -> HashSet<GroupId> {
        let meta = &self.db.meta;
        let ids = [meta.recyclebin_uuid, meta.entry_templates_group];
        ids.into_iter().flatten().map(GroupId::from).collect()
    }

    pub fn listing(&self) -> Listing {
        let mut custom_icons = BTreeMap::new();
        let mut entries: Vec<EntrySummary> = self
            .visible_entries()
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
        Listing { entries, custom_icons }
    }

    pub fn detail(&self, id: &str) -> Option<EntryDetail> {
        let entry = self.entry(id)?;
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
        Some(EntryDetail { summary: summary(&entry), fields })
    }

    /// One field's value, protected or not (`Password`, `UserName`, `URL` or
    /// an additional attribute).
    pub fn field(&self, id: &str, name: &str) -> Option<Zeroizing<String>> {
        let entry = self.entry(id)?;
        entry.fields.get(name).map(|v| Zeroizing::new(v.get().clone()))
    }

    /// The entry's URL as a web address, if it is one.
    pub fn web_url(&self, id: &str) -> Option<url::Url> {
        icons::web_url(&self.field(id, fields::URL)?)
    }

    /// A visible entry by id; entries in the recycle bin are not reachable.
    fn entry(&self, id: &str) -> Option<EntryRef<'_>> {
        let id = EntryId::from(Uuid::parse_str(id).ok()?);
        let entry = self.db.entry(id)?;
        (!is_in(&entry, &self.hidden_groups())).then_some(entry)
    }

    /// The entry with every value, for the editor.
    pub fn edit_data(&self, id: &str) -> Option<EntryData> {
        let entry = self.entry(id)?;
        Some(edit::read(&entry, group_path(&entry)))
    }

    /// Creates (`id` is `None`) or changes an entry and saves the file.
    /// Returns the entry's id. Nothing changes when saving fails.
    pub fn save_entry(&mut self, id: Option<&str>, data: &EntryData) -> Result<String, String> {
        let id = id.map(|id| self.entry(id).map(|e| e.id()).ok_or("That entry is no longer in the database")).transpose()?;
        let hidden = self.hidden_groups();
        let mut db = self.db.clone();
        let id = edit::apply(&mut db, id, data, &hidden)?;
        // An untouched entry does not rewrite the file (and wake the sync client).
        if db != self.db {
            self.commit(db)?;
        }
        Ok(id.uuid().to_string())
    }

    /// Moves an entry to the recycle bin and saves the file.
    pub fn delete_entry(&mut self, id: &str) -> Result<(), String> {
        let id = self.entry(id).map(|e| e.id()).ok_or("That entry is no longer in the database")?;
        let mut db = self.db.clone();
        edit::recycle(&mut db, id)?;
        self.commit(db)
    }

    /// Saves `db` and makes it the current database, or keeps the old one.
    /// The file is written as KDBX 4.1, the only version keepass-rs writes;
    /// the cipher and key derivation stay as they were.
    fn commit(&mut self, mut db: Database) -> Result<(), String> {
        db.config.version = DatabaseVersion::KDB4(1);
        self.file.as_mut().ok_or("This database cannot be saved")?.save(&db)?;
        self.db = db;
        Ok(())
    }

    /// Every group entries can go in, as paths, for the editor.
    pub fn group_paths(&self) -> Vec<Vec<String>> {
        let hidden = self.hidden_groups();
        let mut paths: Vec<Vec<String>> = self
            .db
            .iter_all_groups()
            .filter(|g| g.parent().is_some())
            .filter_map(|g| {
                let mut path = Vec::new();
                let mut id = Some(g.id());
                while let Some(group) = id.and_then(|id| self.db.group(id)) {
                    if hidden.contains(&group.id()) {
                        return None;
                    }
                    id = group.parent().map(|p| p.id());
                    if id.is_some() {
                        path.push(group.name.clone());
                    }
                }
                path.reverse();
                Some(path)
            })
            .collect();
        paths.sort_by_key(|p| p.iter().map(|n| n.to_lowercase()).collect::<Vec<_>>());
        paths
    }

    /// The entry's current TOTP code, if it has a usable secret.
    pub fn totp(&self, id: &str) -> Option<Result<otp::Code, String>> {
        let value = self.field(id, fields::OTP)?;
        Some(otp::Totp::parse(&value).map(|totp| totp.code_now()))
    }
}

/// Group names from the top, without the root group.
fn group_path(entry: &EntryRef<'_>) -> Vec<String> {
    let mut chain = ancestors(entry);
    chain.pop(); // the root group
    chain.iter().rev().filter_map(|&id| entry.database().group(id).map(|g| g.name.clone())).collect()
}

/// True when the entry sits in one of `groups`, at any depth.
fn is_in(entry: &EntryRef<'_>, groups: &HashSet<GroupId>) -> bool {
    ancestors(entry).iter().any(|g| groups.contains(g))
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

/// The ids of the groups above an entry, nearest first, ending with the root.
fn ancestors(entry: &EntryRef<'_>) -> Vec<GroupId> {
    let db = entry.database();
    let mut ids = Vec::new();
    let mut next = Some(entry.parent().id());
    while let Some(id) = next {
        ids.push(id);
        next = db.group(id).and_then(|g| g.parent().map(|p| p.id()));
    }
    ids
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
        has_password: e.get_password().is_some_and(|p| !p.is_empty()),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

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
            e.set("PIN", Value::protected("1234".to_string()));
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

    #[test]
    fn listing_hides_the_recycle_bin_and_secrets() {
        let vault = Vault::from_database(sample());
        let listing = vault.listing();
        let titles: Vec<&str> = listing.entries.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, ["alpha router", "Mail"]);
        let mail = &listing.entries[1];
        assert_eq!(mail.group, ["Work"]);
        assert_eq!(mail.host.as_deref(), Some("example.com"));
        assert!(mail.has_password);
        let json = serde_json::to_string(&listing.entries).unwrap();
        assert!(!json.contains("s3cret") && !json.contains("1234"), "{json}");
    }

    #[test]
    fn detail_masks_protected_fields() {
        let vault = Vault::from_database(sample());
        let id = vault.listing().entries[1].id.clone();
        let detail = vault.detail(&id).unwrap();
        assert_eq!(
            detail.fields,
            [
                Field { name: "PIN".into(), value: None, protected: true },
                Field { name: "Recovery phone".into(), value: Some("+0 000".into()), protected: false },
            ]
        );
        assert_eq!(vault.field(&id, fields::PASSWORD).unwrap().as_str(), "s3cret");
        assert_eq!(vault.field(&id, "PIN").unwrap().as_str(), "1234");
        assert!(vault.field(&id, "Nope").is_none());
    }

    #[test]
    fn recycled_and_unknown_entries_are_unreachable() {
        let db = sample();
        let trash = db.meta.recyclebin_uuid.unwrap();
        let deleted = db.iter_all_entries().find(|e| e.parent().id().uuid() == trash).unwrap().id().uuid();
        let vault = Vault::from_database(db);
        assert!(vault.detail(&deleted.to_string()).is_none());
        assert!(vault.detail("not-a-uuid").is_none());
    }

    #[test]
    fn opens_a_saved_file_and_reports_a_wrong_password() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.kdbx");
        sample().save(&mut File::create(&path).unwrap(), DatabaseKey::new().with_password("pw")).unwrap();
        let vault = Vault::open(&path, Some("pw"), None).unwrap();
        assert_eq!(vault.listing().entries.len(), 2);
        assert_eq!(Vault::open(&path, Some("nope"), None).err().unwrap(), "Wrong password or key file");
        assert_eq!(Vault::open(&path, None, None).err().unwrap(), "Enter the password or choose a key file");
    }

    fn fixture_path(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
    }

    /// Protected values share one keystream in file order; reading them in any
    /// other order gives wrong secrets, not an error. The fixtures are made by
    /// `scripts/make-fixtures.py`.
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
        for name in ["sic2kdbx.kdbx"] {
            let dir = tempfile::tempdir().unwrap();
            let mut vault = fixture(name, dir.path());
            let mut before = vault.db.clone();
            vault.commit(before.clone()).unwrap();
            let reopened = Vault::open(&dir.path().join(name), Some("test"), None).unwrap();
            before.config.version = DatabaseVersion::KDB4(1);
            assert_eq!(reopened.db, before, "{name}");
        }
    }

    #[test]
    fn editing_an_entry_keeps_its_attachments_history_and_custom_data() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let id = vault.listing().entries.iter().find(|e| e.title == "Router").unwrap().id.clone();
        let old = vault.db.entry(EntryId::from(Uuid::parse_str(&id).unwrap())).unwrap().clone();

        let mut data = vault.edit_data(&id).unwrap();
        data.password = "changed".into();
        vault.save_entry(Some(&id), &data).unwrap();

        let reopened = Vault::open(&dir.path().join("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        let entry = reopened.db.entry(old.id()).unwrap();
        assert_eq!(entry.get_password(), Some("changed"));
        assert_eq!(entry.attachments().count(), 2);
        assert_eq!(entry.custom_data, old.custom_data);
        assert_eq!(entry.history.as_ref().unwrap().get_entries().len(), old.history.as_ref().unwrap().get_entries().len() + 1);
        assert_eq!(entry.get(fields::OTP), old.get(fields::OTP));
        // Templates and the recycle bin stay where they were.
        assert_eq!(reopened.listing().entries.len(), 3);
        assert!(std::fs::metadata(dir.path().join("sic2kdbx.kdbx.bak")).is_ok());
    }

    #[test]
    fn saving_an_untouched_entry_leaves_the_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let id = vault.listing().entries[0].id.clone();
        let data = vault.edit_data(&id).unwrap();
        vault.save_entry(Some(&id), &data).unwrap();
        assert!(!dir.path().join("sic2kdbx.kdbx.bak").exists());
    }

    #[test]
    fn deleting_saves_and_hides_the_entry() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let id = vault.listing().entries[0].id.clone();
        vault.delete_entry(&id).unwrap();
        let reopened = Vault::open(&dir.path().join("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        assert!(reopened.detail(&id).is_none());
        assert_eq!(reopened.listing().entries.len(), 2);
    }

    #[test]
    fn totp_codes_come_from_the_otp_field() {
        let dir = tempfile::tempdir().unwrap();
        let vault = fixture("sic2kdbx.kdbx", dir.path());
        let router = vault.listing().entries.iter().find(|e| e.title == "Router").unwrap().id.clone();
        let code = vault.totp(&router).unwrap().unwrap();
        assert_eq!((code.code.len(), code.period), (6, 30));
        let mail = vault.listing().entries.iter().find(|e| e.title == "Mail").unwrap().id.clone();
        assert!(vault.totp(&mail).is_none());
    }

    #[test]
    fn group_paths_leave_out_the_bin_and_templates() {
        let dir = tempfile::tempdir().unwrap();
        let vault = fixture("sic2kdbx.kdbx", dir.path());
        assert_eq!(vault.group_paths(), [vec!["NET".to_string()], vec!["Work; Home, X".to_string()]]);
    }
}
