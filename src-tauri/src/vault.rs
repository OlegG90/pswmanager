//! The unlocked database. Everything decrypted stays here: the frontend gets
//! summaries without secrets, and a secret only when asked for one field.

use crate::icons;
use keepass::db::{fields, EntryId, EntryRef, GroupId};
use keepass::error::{DatabaseKeyError, DatabaseOpenError};
use keepass::{Database, DatabaseKey};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::path::Path;
use uuid::Uuid;
use zeroize::Zeroizing;

pub struct Vault {
    db: Database,
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
        let mut file = File::open(path).map_err(|e| format!("Cannot open the database: {e}"))?;
        let db = Database::open(&mut file, key).map_err(|e| open_error(&e))?;
        Ok(Vault { db })
    }

    #[cfg(test)]
    pub fn from_database(db: Database) -> Vault {
        Vault { db }
    }

    /// All entries the user works with: not in the recycle bin, not templates.
    fn visible_entries(&self) -> impl Iterator<Item = EntryRef<'_>> {
        let hidden: HashSet<GroupId> = self.hidden_groups();
        self.db.iter_all_entries().filter(move |e| !ancestors(e).iter().any(|g| hidden.contains(g)))
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
                    if let Some(url) = icons::data_url(&icon.data) {
                        custom_icons.insert(key.clone(), url);
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

    /// The distinct sites of all entries, for fetching their icons.
    pub fn hosts(&self) -> Vec<String> {
        let mut hosts: Vec<String> = self.visible_entries().filter_map(|e| icons::host_of(e.get_url()?)).collect();
        hosts.sort();
        hosts.dedup();
        hosts
    }

    pub fn detail(&self, id: &str) -> Option<EntryDetail> {
        let entry = self.entry(id)?;
        let mut fields: Vec<Field> = entry
            .fields
            .iter()
            .filter(|(name, _)| !fields::KNOWN_FIELDS.contains(&name.as_str()))
            .map(|(name, value)| Field {
                name: name.clone(),
                protected: value.is_protected(),
                value: (!value.is_protected()).then(|| value.get().clone()),
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

    /// A visible entry by id; entries in the recycle bin are not reachable.
    fn entry(&self, id: &str) -> Option<EntryRef<'_>> {
        let id = EntryId::from(Uuid::parse_str(id).ok()?);
        let entry = self.db.entry(id)?;
        let hidden = self.hidden_groups();
        (!ancestors(&entry).iter().any(|g| hidden.contains(g))).then_some(entry)
    }
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
    let text = |name: &str| e.get(name).unwrap_or_default().to_string();
    let url = text(fields::URL);
    let mut chain = ancestors(e);
    chain.pop(); // the root group
    let group = chain.iter().rev().filter_map(|&id| e.database().group(id).map(|g| g.name.clone())).collect();
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

fn open_error(e: &DatabaseOpenError) -> String {
    match e {
        DatabaseOpenError::Key(DatabaseKeyError::IncorrectKey) => "Wrong password or key file".into(),
        DatabaseOpenError::Key(DatabaseKeyError::EmptyKey) => "Enter the password or choose a key file".into(),
        DatabaseOpenError::Io(e) => format!("Cannot read the database: {e}"),
        DatabaseOpenError::UnsupportedVersion => "This database version is not supported".into(),
        other => format!("Cannot open the database: {other}"),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use keepass::db::Value;

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
        assert_eq!(vault.hosts(), ["example.com"]);
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

    #[test]
    fn opens_fields_that_are_not_contiguous() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pykeepass.kdbx");
        let vault = Vault::open(&fixture, Some("test"), None).unwrap();
        let id = vault.listing().entries[0].id.clone();
        let detail = vault.detail(&id).unwrap();
        assert_eq!(detail.summary.tags, ["Tag"]);
        assert_eq!(detail.fields.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["Extra", "Secret"]);
        assert_eq!(vault.field(&id, "Secret").unwrap().as_str(), "hidden");
    }
}
