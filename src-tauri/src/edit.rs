//! Changes to the database: an entry as the editor sees it, applied back with
//! its previous version kept in the entry's history.

use crate::otp;
use keepass::db::{fields, EntryId, EntryRef, GroupId, History, Times, Value};
use keepass::Database;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// KeePass keeps this many versions when the database does not say.
const DEFAULT_HISTORY_ITEMS: usize = 10;
const RECYCLE_BIN_ICON: usize = 43;

/// Everything the editor shows and sends back, secrets included: while an
/// entry is being edited its values are in the window anyway.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase")]
pub struct EntryData {
    pub title: String,
    pub username: String,
    pub password: String,
    pub url: String,
    pub notes: String,
    /// The TOTP secret or `otpauth://` URI; empty for none.
    pub otp: String,
    pub tags: Vec<String>,
    /// Group names from the top, without the root group.
    pub group: Vec<String>,
    /// Additional attributes.
    pub fields: Vec<FieldData>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase")]
pub struct FieldData {
    pub name: String,
    pub value: String,
    pub protected: bool,
}

/// The fields the editor has its own inputs for.
const STANDARD: [&str; 6] = [fields::TITLE, fields::USERNAME, fields::PASSWORD, fields::URL, fields::NOTES, fields::OTP];

pub fn read(entry: &EntryRef<'_>, group: Vec<String>) -> EntryData {
    let text = |name: &str| entry.fields.get(name).map(|v| v.get().clone()).unwrap_or_default();
    let mut additional: Vec<FieldData> = entry
        .fields
        .iter()
        .filter(|(name, _)| !STANDARD.contains(&name.as_str()))
        .map(|(name, value)| FieldData { name: name.clone(), value: value.get().clone(), protected: value.is_protected() })
        .collect();
    additional.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    EntryData {
        title: text(fields::TITLE),
        username: text(fields::USERNAME),
        password: text(fields::PASSWORD),
        url: text(fields::URL),
        notes: text(fields::NOTES),
        otp: text(fields::OTP),
        tags: entry.tags.clone(),
        group,
        fields: additional,
    }
}

/// Creates the entry (`id` is `None`) or changes it, and returns its id. An
/// edit that changes nothing leaves the entry and its history alone.
/// `hidden` are the groups entries cannot be put in (recycle bin, templates).
pub fn apply(db: &mut Database, id: Option<EntryId>, data: &EntryData, hidden: &HashSet<GroupId>) -> Result<EntryId, String> {
    let otp = match data.otp.trim() {
        "" => None,
        value => Some(otp::normalize(value, &data.title)?),
    };
    let group = group_at(db, &data.group, hidden)?;
    let tags: Vec<String> = data.tags.iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();

    let Some(id) = id else {
        let wanted = wanted_fields(db, None, data, otp)?;
        let mut parent = db.group_mut(group).ok_or("The group is gone")?;
        let mut entry = parent.add_entry();
        entry.fields = wanted;
        entry.tags = tags;
        return Ok(entry.id());
    };

    let entry = db.entry(id).ok_or("That entry is no longer in the database")?;
    let wanted = wanted_fields(db, Some(&entry), data, otp)?;
    let moved = entry.parent().id() != group;
    if entry.fields == wanted && entry.tags == tags && !moved {
        return Ok(id);
    }
    {
        let mut entry = db.entry_mut(id).expect("checked above");
        let mut tracked = entry.track_changes();
        tracked.edit(|e| {
            e.fields = wanted;
            e.tags = tags;
        });
        if moved {
            tracked.move_to(group).map_err(|e| e.to_string())?;
        }
    } // dropping the tracker files the old version into the history
    trim_history(db, id);
    Ok(id)
}

/// The entry's fields as the editor wants them, each keeping whether it was
/// protected; the password and the TOTP secret are always protected.
fn wanted_fields(
    db: &Database,
    entry: Option<&EntryRef<'_>>,
    data: &EntryData,
    otp: Option<String>,
) -> Result<HashMap<String, Value<String>>, String> {
    let mut names = HashSet::new();
    for field in &data.fields {
        let name = field.name.trim();
        if name.is_empty() {
            return Err("A field needs a name".into());
        }
        if STANDARD.iter().any(|s| s.eq_ignore_ascii_case(name)) {
            return Err(format!("\"{name}\" is a standard field; use its own input"));
        }
        if !names.insert(name.to_lowercase()) {
            return Err(format!("There are two fields named \"{name}\""));
        }
    }

    let protection = db.meta.memory_protection.as_ref();
    let was_protected = |name: &str| entry.and_then(|e| e.fields.get(name)).map(Value::is_protected);
    let standard = [
        (fields::TITLE, &data.title, protection.is_some_and(|p| p.protect_title)),
        (fields::USERNAME, &data.username, protection.is_some_and(|p| p.protect_username)),
        (fields::PASSWORD, &data.password, true),
        (fields::URL, &data.url, protection.is_some_and(|p| p.protect_url)),
        (fields::NOTES, &data.notes, protection.is_some_and(|p| p.protect_notes)),
    ];
    let mut wanted = HashMap::new();
    for (name, value, protect_by_default) in standard {
        // A standard field the entry never had stays absent while it is empty,
        // so saving an untouched entry changes nothing.
        let existed = entry.is_some_and(|e| e.fields.contains_key(name));
        if value.is_empty() && entry.is_some() && !existed {
            continue;
        }
        let protected = name == fields::PASSWORD || was_protected(name).unwrap_or(protect_by_default);
        wanted.insert(name.to_string(), value_of(value, protected));
    }
    if let Some(otp) = otp {
        wanted.insert(fields::OTP.to_string(), Value::protected(otp));
    }
    for field in &data.fields {
        wanted.insert(field.name.trim().to_string(), value_of(&field.value, field.protected));
    }
    Ok(wanted)
}

fn value_of(value: &str, protected: bool) -> Value<String> {
    if protected {
        Value::protected(value.to_string())
    } else {
        Value::unprotected(value.to_string())
    }
}

/// The group at `path` below the root, created where missing.
fn group_at(db: &mut Database, path: &[String], hidden: &HashSet<GroupId>) -> Result<GroupId, String> {
    let mut id = db.root().id();
    for name in path.iter().map(|n| n.trim()).filter(|n| !n.is_empty()) {
        let existing = db.group(id).and_then(|g| g.groups().find(|c| c.name == name).map(|c| c.id()));
        id = match existing {
            Some(child) => child,
            None => {
                let mut parent = db.group_mut(id).ok_or("The group is gone")?;
                let mut group = parent.add_group();
                group.name = name.to_string();
                group.id()
            }
        };
        if hidden.contains(&id) {
            return Err(format!("Entries cannot be put in \"{name}\""));
        }
    }
    Ok(id)
}

/// Keeps only as many old versions as the database allows (KeePass's
/// `HistoryMaxItems`; negative means no limit).
fn trim_history(db: &mut Database, id: EntryId) {
    let limit = match db.meta.history_max_items {
        Some(n) if n < 0 => return,
        Some(n) => n as usize,
        None => DEFAULT_HISTORY_ITEMS,
    };
    let Some(mut entry) = db.entry_mut(id) else { return };
    let Some(history) = &entry.history else { return };
    if history.get_entries().len() <= limit {
        return;
    }
    let kept: Vec<_> = history.get_entries().iter().take(limit).cloned().collect();
    let mut trimmed = History::default();
    for old in kept.into_iter().rev() {
        trimmed.add_entry(old); // adds at the front, so oldest first keeps the order
    }
    entry.history = Some(trimmed);
}

/// Moves the entry to the recycle bin, creating the bin if the database has
/// none. A database that turned the bin off deletes the entry for good.
pub fn recycle(db: &mut Database, id: EntryId) -> Result<(), String> {
    db.entry(id).ok_or("That entry is no longer in the database")?;
    if db.meta.recyclebin_enabled == Some(false) {
        db.entry_mut(id).expect("checked above").track_changes().remove();
        return Ok(());
    }
    let bin = match db.meta.recyclebin_uuid.map(GroupId::from).filter(|g| db.group(*g).is_some()) {
        Some(bin) => bin,
        None => {
            let mut root = db.root_mut();
            let mut group = root.add_group();
            group.name = "Recycle Bin".into();
            group.enable_autotype = Some(false);
            group.enable_searching = Some(false);
            group.set_icon_builtin(RECYCLE_BIN_ICON);
            let bin = group.id();
            db.meta.recyclebin_uuid = Some(bin.uuid());
            db.meta.recyclebin_enabled = Some(true);
            db.meta.recyclebin_changed = Some(Times::now());
            bin
        }
    };
    let mut entry = db.entry_mut(id).expect("checked above");
    entry.move_to(bin).map_err(|e| e.to_string())?;
    entry.times.location_changed = Some(Times::now());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(title: &str) -> EntryData {
        let mut data = EntryData::default();
        data.title = title.into();
        data.username = "me".into();
        data.password = "pw1".into();
        data
    }

    /// `data(title)` with one change.
    fn with(title: &str, change: impl FnOnce(&mut EntryData)) -> EntryData {
        let mut data = data(title);
        change(&mut data);
        data
    }

    fn history_len(db: &Database, id: EntryId) -> usize {
        db.entry(id).unwrap().history.as_ref().map_or(0, |h| h.get_entries().len())
    }

    #[test]
    fn creates_an_entry_in_a_new_group() {
        let mut db = Database::new();
        let input = with("Mail", |d| {
            d.group = vec!["Work".into(), "Mail".into()];
            d.tags = vec![" a ".into(), "".into()];
        });
        let id = apply(&mut db, None, &input, &HashSet::new()).unwrap();
        let entry = db.entry(id).unwrap();
        assert_eq!(entry.get_title(), Some("Mail"));
        assert!(entry.fields[fields::PASSWORD].is_protected());
        assert_eq!(entry.tags, ["a"]);
        assert_eq!(entry.parent().name, "Mail");
        assert_eq!(entry.parent().parent().unwrap().name, "Work");
        // Reading it back gives what was entered.
        let mut expected = input.clone();
        expected.tags = vec!["a".into()];
        assert_eq!(read(&entry, input.group.clone()), expected);
    }

    #[test]
    fn an_edit_keeps_the_old_version_and_protection() {
        let mut db = Database::new();
        let id = apply(&mut db, None, &data("Bank"), &HashSet::new()).unwrap();
        db.entry_mut(id).unwrap().set_protected(fields::USERNAME, "me");
        db.entry_mut(id).unwrap().set_unprotected("Extra", "keep");

        let mut changed = read(&db.entry(id).unwrap(), vec![]);
        changed.password = "pw2".into();
        changed.fields.push(FieldData { name: "PIN".into(), value: "1234".into(), protected: true });
        apply(&mut db, Some(id), &changed, &HashSet::new()).unwrap();

        let entry = db.entry(id).unwrap();
        assert_eq!(entry.get_password(), Some("pw2"));
        assert!(entry.fields[fields::USERNAME].is_protected());
        assert!(entry.fields["PIN"].is_protected());
        assert_eq!(entry.get("Extra"), Some("keep"));
        assert_eq!(history_len(&db, id), 1);
        assert_eq!(entry.history.as_ref().unwrap().get_entries()[0].get_password(), Some("pw1"));
    }

    #[test]
    fn saving_an_untouched_entry_changes_nothing() {
        let mut db = Database::new();
        let id = db.root_mut().add_entry().edit(|e| e.set_unprotected(fields::TITLE, "Only a title")).id();
        let before = db.clone();
        let same = read(&db.entry(id).unwrap(), vec![]);
        apply(&mut db, Some(id), &same, &HashSet::new()).unwrap();
        assert_eq!(db, before);
    }

    #[test]
    fn totp_secrets_are_stored_as_protected_uris() {
        let mut db = Database::new();
        let id = apply(&mut db, None, &with("Site", |d| d.otp = "jbsw y3dp".into()), &HashSet::new()).unwrap();
        let otp = &db.entry(id).unwrap().fields[fields::OTP];
        assert!(otp.is_protected());
        assert_eq!(otp.get(), "otpauth://totp/Site?secret=JBSWY3DP");
        assert!(apply(&mut db, None, &with("x", |d| d.otp = "bad!".into()), &HashSet::new()).is_err());
    }

    #[test]
    fn field_names_are_checked() {
        let mut db = Database::new();
        let field = |name: &str| FieldData { name: name.into(), value: "v".into(), protected: false };
        for fields in [vec![field("")], vec![field("password")], vec![field("A"), field("a")]] {
            assert!(apply(&mut db, None, &with("x", |d| d.fields = fields), &HashSet::new()).is_err());
        }
    }

    #[test]
    fn history_is_trimmed_to_the_database_limit() {
        let mut db = Database::new();
        db.meta.history_max_items = Some(2);
        let id = apply(&mut db, None, &data("x"), &HashSet::new()).unwrap();
        for n in 0..5 {
            apply(&mut db, Some(id), &with("x", |d| d.password = format!("p{n}")), &HashSet::new()).unwrap();
        }
        assert_eq!(history_len(&db, id), 2);
        let newest_old = db.entry(id).unwrap().history.as_ref().unwrap().get_entries()[0].get_password().map(str::to_string);
        assert_eq!(newest_old.as_deref(), Some("p3"));
    }

    #[test]
    fn deleting_moves_to_a_recycle_bin_created_on_demand() {
        let mut db = Database::new();
        let id = apply(&mut db, None, &data("x"), &HashSet::new()).unwrap();
        recycle(&mut db, id).unwrap();
        let bin = GroupId::from(db.meta.recyclebin_uuid.unwrap());
        assert_eq!(db.entry(id).unwrap().parent().id(), bin);
        assert!(db.entry(id).unwrap().times.location_changed.is_some());

        let hidden = HashSet::from([bin]);
        let into_bin = with("y", |d| d.group = vec!["Recycle Bin".into()]);
        assert!(apply(&mut db, None, &into_bin, &hidden).is_err());
    }

    #[test]
    fn deleting_without_a_recycle_bin_removes_for_good() {
        let mut db = Database::new();
        db.meta.recyclebin_enabled = Some(false);
        let id = apply(&mut db, None, &data("x"), &HashSet::new()).unwrap();
        recycle(&mut db, id).unwrap();
        assert!(db.entry(id).is_none());
        assert!(db.deleted_objects.contains_key(&id.uuid()));
    }
}
