//! The unlocked database. Everything decrypted stays here: the frontend gets
//! summaries without secrets, a secret only when asked for one field, and a
//! whole entry only while it is being edited.

use crate::dbfile::{DbFile, Read, SaveError, Snapshot};
use crate::edit::{self, EntryData, NOT_FOUND};
use crate::sync::Outcome;
use crate::{icons, otp};
use keepass::db::{fields, EntryId, EntryRef, GroupId, Value};
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
}

#[derive(Debug, Clone, Serialize)]
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
        Ok(Vault { db, file: Some(file), unsaved: false })
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
        let mut attachments: Vec<Attachment> =
            entry.attachments_named().map(|(name, a)| Attachment { name: name.to_string(), size: a.data.get().len() }).collect();
        attachments.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Some(EntryDetail { summary: summary(&entry), fields, attachments })
    }

    /// The content of one of the entry's files, for saving it to disk.
    pub fn attachment(&self, id: &str, name: &str) -> Option<Zeroizing<Vec<u8>>> {
        let entry = self.entry(id)?;
        entry.attachment_by_name(name).map(|a| Zeroizing::new(a.data.get().clone()))
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
    /// `base` is the entry as the editor opened it: only what the editor
    /// changed against it is applied, so changes another device made since
    /// stay. Returns the entry's id and the fields both changed (the
    /// editor's version won). Nothing changes when saving fails.
    pub fn save_entry(&mut self, id: Option<&str>, base: Option<&EntryData>, data: &EntryData) -> Result<(String, Vec<String>), String> {
        let id = id.map(parse_id).transpose()?;
        let (id, conflicts) = self.change(|db, hidden| {
            let current = id.and_then(|id| db.entry(id));
            if current.as_ref().is_some_and(|e| is_template(e)) {
                return Err(NOT_FOUND.into());
            }
            match (current, base) {
                (Some(entry), Some(base)) => {
                    let (merged, conflicts) = edit::merge3(&edit::read(&entry, group_path(&entry)), base, data);
                    Ok((edit::apply(db, id, &merged, hidden)?, conflicts))
                }
                _ => Ok((edit::apply(db, id, data, hidden)?, Vec::new())),
            }
        })?;
        Ok((id.uuid().to_string(), conflicts))
    }

    /// Attaches a file to an entry and saves the file; returns the name the
    /// file got (see [edit::attach]).
    pub fn attach(&mut self, id: &str, name: &str, data: &[u8]) -> Result<String, String> {
        let id = parse_id(id)?;
        // Templates and the recycle bin are among the hidden groups.
        self.change(|db, hidden| edit::attach(db, id, name, data, hidden))
    }

    /// Removes a file from an entry and saves the file; the entry's history
    /// keeps it (see [edit::detach]).
    pub fn detach(&mut self, id: &str, name: &str) -> Result<(), String> {
        let id = parse_id(id)?;
        self.change(|db, hidden| edit::detach(db, id, name, hidden))
    }

    /// Renames one of an entry's files and saves the file; returns the new
    /// name (see [edit::rename_attachment]).
    pub fn rename_attachment(&mut self, id: &str, from: &str, to: &str) -> Result<String, String> {
        let id = parse_id(id)?;
        self.change(|db, hidden| edit::rename_attachment(db, id, from, to, hidden))
    }

    /// Gives one of an entry's files new content and saves the file (see
    /// [edit::replace_attachment]).
    pub fn replace_attachment(&mut self, id: &str, name: &str, data: &[u8]) -> Result<(), String> {
        let id = parse_id(id)?;
        self.change(|db, hidden| edit::replace_attachment(db, id, name, data, hidden))
    }

    /// Moves an entry to the recycle bin and saves the file. An entry already
    /// gone (deleted or binned elsewhere) needs nothing.
    pub fn delete_entry(&mut self, id: &str) -> Result<(), String> {
        let id = parse_id(id)?;
        self.change(|db, hidden| match db.entry(id) {
            Some(entry) if !is_in(&entry, hidden) => edit::recycle(db, id),
            _ => Ok(()),
        })
    }

    /// The file as it is now, if it changed on disk since it was last read or
    /// written: it becomes the database, and the ids of the entries that
    /// differ are returned (added, changed, moved or gone).
    pub fn reload(&mut self) -> Result<Option<Vec<String>>, String> {
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
    pub fn adopt(&mut self, since: &Snapshot, read: Read) -> Option<Vec<String>> {
        let mut db = self.file.as_mut()?.adopt(since, read)?;
        if !edit::keep_newer(&mut db, &self.db).is_empty() {
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
    pub fn take_remote(&mut self, since: &Snapshot, mut theirs: Database, raw: &[u8], merge: bool) -> Result<Option<Outcome>, String> {
        let file = self.file.as_mut().ok_or("This database cannot be saved")?;
        if !file.is_at(since) {
            return Ok(None);
        }
        let kept_ours = merge && !edit::merge(&mut theirs, &self.db).is_empty();
        let written = if kept_ours { file.save(&mut theirs) } else { file.write(raw) };
        match written {
            Ok(()) => {}
            Err(SaveError::Changed) => return Ok(None),
            Err(SaveError::Failed(message)) => return Err(message),
        }
        let changed = changed_entries(&self.db, &theirs);
        self.db = theirs;
        self.unsaved = false;
        Ok(Some(if kept_ours { Outcome::Merged(changed) } else { Outcome::Downloaded(changed) }))
    }

    /// True when the database holds changes its file lacks (kept from an
    /// older file, not written yet).
    pub fn has_unsaved(&self) -> bool {
        self.unsaved
    }

    /// Writes changes kept from an older file, if any.
    pub fn save_pending(&mut self) -> Result<(), String> {
        if self.unsaved {
            self.change(|_, _| Ok(()))?;
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
        const ATTEMPTS: usize = 3;
        for _ in 0..ATTEMPTS {
            self.reload()?;
            let mut db = self.db.clone();
            let result = change(&mut db, &self.hidden_groups())?;
            if db == self.db && !self.unsaved {
                return Ok(result);
            }
            match self.file.as_mut().ok_or("This database cannot be saved")?.save(&mut db) {
                Ok(()) => {
                    self.db = db;
                    self.unsaved = false;
                    return Ok(result);
                }
                Err(SaveError::Changed) => continue,
                Err(SaveError::Failed(message)) => return Err(message),
            }
        }
        Err("The database file keeps changing on disk; try again in a moment".into())
    }

    /// Every group entries can go in, as paths, for the editor.
    pub fn group_paths(&self) -> Vec<Vec<String>> {
        let hidden = self.hidden_groups();
        let mut paths: Vec<Vec<String>> = self
            .db
            .iter_all_groups()
            .filter(|g| g.parent().is_some() && !edit::ancestors(&self.db, g.id()).iter().any(|a| hidden.contains(a)))
            .map(|g| edit::path_of(&self.db, g.id()))
            .collect();
        paths.sort_by_key(|p| p.iter().map(|n| n.to_lowercase()).collect::<Vec<_>>());
        paths
    }

    /// The entry's current TOTP code, or `None` when it has no secret.
    pub fn totp(&self, id: &str) -> Result<Option<otp::Code>, String> {
        let Some(value) = self.field(id, fields::OTP) else { return Ok(None) };
        otp::Totp::parse(&value).map(|totp| Some(totp.code_now()))
    }
}

/// Templates are not entries the user edits (they are hidden from the list).
fn is_template(entry: &EntryRef<'_>) -> bool {
    let templates = entry.database().meta.entry_templates_group.map(GroupId::from);
    templates.is_some_and(|t| is_in(entry, &HashSet::from([t])))
}

fn parse_id(id: &str) -> Result<EntryId, String> {
    Uuid::parse_str(id).map(EntryId::from).map_err(|_| NOT_FOUND.into())
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
    fn editing_an_entry_keeps_its_attachments_history_and_custom_data() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let id = vault.listing().entries.iter().find(|e| e.title == "Router").unwrap().id.clone();
        let old = vault.db.entry(EntryId::from(Uuid::parse_str(&id).unwrap())).unwrap().clone();

        let mut data = vault.edit_data(&id).unwrap();
        data.password = "changed".into();
        vault.save_entry(Some(&id), None, &data).unwrap();

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

        assert_eq!(vault.attach(&mail, "key.txt", b"recovery codes").unwrap(), "key.txt");
        let reopened = Vault::open(&dir.path().join("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        assert_eq!(reopened.detail(&mail).unwrap().attachments, [Attachment { name: "key.txt".into(), size: 14 }]);
        assert_eq!(reopened.attachment(&mail, "key.txt").unwrap().as_slice(), b"recovery codes");
        assert!(reopened.attachment(&mail, "other.txt").is_none());
        let mut expected = before;
        expected.push(("Mail".into(), "key.txt".into(), b"recovery codes".to_vec()));
        expected.sort();
        assert_eq!(all_files(&reopened), expected);
        // The router's files are listed, by name.
        let router = id_of(&reopened, "Router").uuid().to_string();
        assert_eq!(reopened.detail(&router).unwrap().attachments.len(), 2);
    }

    #[test]
    fn removing_a_file_keeps_it_in_history_and_the_other_files_intact() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let router = id_of(&vault, "Router");
        let id = router.uuid().to_string();
        let mail = id_of(&vault, "Mail").uuid().to_string();
        // A file after the router's in the database, so removing one of the
        // router's would misnumber it if the database let go of it.
        vault.attach(&mail, "later.txt", b"attached later").unwrap();
        let files = |v: &Vault, id: EntryId| {
            let mut all: Vec<(String, Vec<u8>)> =
                v.db.entry(id).unwrap().attachments_named().map(|(n, a)| (n.to_string(), a.data.get().clone())).collect();
            all.sort();
            all
        };
        let before = files(&vault, router);
        let (gone, kept) = (before[0].clone(), before[1].clone());

        vault.detach(&id, &gone.0).unwrap();
        vault.detach(&id, "no such file").unwrap();
        let reopened = Vault::open(&dir.path().join("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        assert_eq!(files(&reopened, router), [kept]);
        assert_eq!(reopened.detail(&id).unwrap().attachments.len(), 1);
        assert_eq!(reopened.attachment(&mail, "later.txt").unwrap().as_slice(), b"attached later");
        let previous = reopened.db.entry(router).unwrap().historical(0).unwrap().attachments_named().map(|(n, a)| (n.to_string(), a.data.get().clone())).collect::<Vec<_>>();
        assert!(previous.contains(&gone), "{previous:?}");
        // Attaching and renaming after a removal still number the files right.
        let mut reopened = reopened;
        reopened.attach(&id, "new.txt", b"new").unwrap();
        assert_eq!(reopened.rename_attachment(&mail, "later.txt", "renamed.txt").unwrap(), "renamed.txt");
        let again = Vault::open(&dir.path().join("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        assert_eq!(again.attachment(&id, "new.txt").unwrap().as_slice(), b"new");
        assert_eq!(again.attachment(&mail, "renamed.txt").unwrap().as_slice(), b"attached later");
        assert!(again.attachment(&mail, "later.txt").is_none());
        let mut again = again;
        again.replace_attachment(&mail, "renamed.txt", b"replaced").unwrap();
        let last = Vault::open(&dir.path().join("sic2kdbx.kdbx"), Some("test"), None).unwrap();
        assert_eq!(last.attachment(&mail, "renamed.txt").unwrap().as_slice(), b"replaced");
        assert_eq!(last.attachment(&id, "new.txt").unwrap().as_slice(), b"new");
    }

    #[test]
    fn saving_an_untouched_entry_leaves_the_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = fixture("sic2kdbx.kdbx", dir.path());
        let id = vault.listing().entries[0].id.clone();
        let data = vault.edit_data(&id).unwrap();
        vault.save_entry(Some(&id), None, &data).unwrap();
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
        assert!(vault.totp(&mail).unwrap().is_none());
    }

    #[test]
    fn group_paths_leave_out_the_bin_and_templates() {
        let dir = tempfile::tempdir().unwrap();
        let vault = fixture("sic2kdbx.kdbx", dir.path());
        assert_eq!(vault.group_paths(), [vec!["NET".to_string()], vec!["Work; Home, X".to_string()]]);
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
        vault.save_entry(Some(&mail), None, &data).unwrap();

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
        let (_, conflicts) = vault.save_entry(Some(&mail.uuid().to_string()), Some(&base), &data).unwrap();
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
        vault.save_entry(Some(&mail.uuid().to_string()), None, &data).unwrap();
        let reopened = Vault::open(&path, Some("test"), None).unwrap();
        assert!(reopened.detail(&mail.uuid().to_string()).is_some());
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
        assert!(vault.detail(&mail.uuid().to_string()).is_none());
        vault.delete_entry(&mail.uuid().to_string()).unwrap();
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
        vault.save_entry(Some(&mail), None, &data).unwrap();

        std::fs::write(&path, &older).unwrap(); // a sync client brings the old copy back
        vault.reload().unwrap();
        assert_eq!(vault.field(&mail, fields::PASSWORD).unwrap().as_str(), "saved here");
        vault.save_pending().unwrap();
        let reopened = Vault::open(&path, Some("test"), None).unwrap();
        assert_eq!(reopened.field(&mail, fields::PASSWORD).unwrap().as_str(), "saved here");
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
        assert!(vault.save_entry(Some(&mail), None, &data).unwrap_err().contains("cannot be read now"));
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
        vault.save_entry(Some(&router), None, &data).unwrap();
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
        let (_, conflicts) = vault.save_entry(Some(&router.uuid().to_string()), Some(&base), &data).unwrap();
        assert!(conflicts.is_empty());

        let reopened = Vault::open(&path, Some("test"), None).unwrap();
        let id = router.uuid().to_string();
        assert_eq!(reopened.field(&id, fields::USERNAME).unwrap().as_str(), "changed on the phone");
        assert_eq!(reopened.field(&id, fields::PASSWORD).unwrap().as_str(), "changed on the PC");
        assert_eq!(reopened.detail(&id).unwrap().summary.group, ["Job"]);
    }
}
