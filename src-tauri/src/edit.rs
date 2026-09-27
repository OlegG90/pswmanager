//! Changes to the database: an entry as the editor sees it, applied back with
//! its previous version kept in the entry's history.

use crate::otp;
use keepass::db::{fields, Entry, EntryId, EntryRef, GroupId, History, Times, Value};
use keepass::Database;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// KeePass keeps this many versions when the database does not say.
const DEFAULT_HISTORY_ITEMS: usize = 10;
const RECYCLE_BIN_ICON: usize = 43;

pub const NOT_FOUND: &str = "That entry is no longer in the database";

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

/// The entry as it is now (`current`, perhaps changed on another device since
/// the editor opened it as `base`) with the editor's own changes (`edited`
/// against `base`) applied on top: what the editor did not touch stays as it
/// is now. Returns the result and the names of fields both sides changed
/// (where the editor's change wins; the other version goes to history).
pub fn merge3(current: &EntryData, base: &EntryData, edited: &EntryData) -> (EntryData, Vec<String>) {
    let mut conflicts = Vec::new();
    let title = pick(&mut conflicts, "Title", &current.title, &base.title, &edited.title);
    let username = pick(&mut conflicts, "User name", &current.username, &base.username, &edited.username);
    let password = pick(&mut conflicts, "Password", &current.password, &base.password, &edited.password);
    let url = pick(&mut conflicts, "URL", &current.url, &base.url, &edited.url);
    let notes = pick(&mut conflicts, "Notes", &current.notes, &base.notes, &edited.notes);
    let otp = pick(&mut conflicts, "TOTP", &current.otp, &base.otp, &edited.otp);
    let tags = pick(&mut conflicts, "Tags", &current.tags, &base.tags, &edited.tags);
    let group = pick(&mut conflicts, "Group", &current.group, &base.group, &edited.group);

    // Additional fields, by name: removed, added or changed in the editor
    // apply; the rest are as they are now.
    let find = |list: &[FieldData], name: &str| list.iter().find(|f| f.name == name).cloned();
    let mut fields: Vec<FieldData> = current.fields.clone();
    for old in &base.fields {
        if find(&edited.fields, &old.name).is_none() {
            if find(&current.fields, &old.name).is_some_and(|now| now != *old) {
                conflicts.push(old.name.clone());
            }
            fields.retain(|f| f.name != old.name);
        }
    }
    for new in &edited.fields {
        let before = find(&base.fields, &new.name);
        if before.as_ref() == Some(new) {
            continue;
        }
        let now = find(&current.fields, &new.name);
        if now != before && now.as_ref() != Some(new) {
            conflicts.push(new.name.clone());
        }
        match fields.iter_mut().find(|f| f.name == new.name) {
            Some(field) => *field = new.clone(),
            None => fields.push(new.clone()),
        }
    }
    let merged = EntryData { title, username, password, url, notes, otp, tags, group, fields };
    (merged, conflicts)
}

/// One value in a three-way merge: the edit if it changed it, what is there
/// now otherwise; a value both changed differently is noted in `conflicts`.
fn pick<T: PartialEq + Clone>(conflicts: &mut Vec<String>, name: &str, current: &T, base: &T, edited: &T) -> T {
    if edited == base {
        return current.clone();
    }
    if current != base && current != edited {
        conflicts.push(name.to_string());
    }
    edited.clone()
}

/// The groups entries do not live in: the recycle bin and the templates.
pub fn hidden_groups(db: &Database) -> HashSet<GroupId> {
    [db.meta.recyclebin_uuid, db.meta.entry_templates_group].into_iter().flatten().map(GroupId::from).collect()
}

/// Creates the entry (`id` is `None`) or changes it, and returns its id. An
/// edit that changes nothing leaves the entry and its history alone. An entry
/// deleted elsewhere since the editor opened comes back with its id: this
/// edit is the newer change.
/// `hidden` are the groups entries cannot be put in (recycle bin, templates).
pub fn apply(db: &mut Database, id: Option<EntryId>, data: &EntryData, hidden: &HashSet<GroupId>) -> Result<EntryId, String> {
    check_field_names(&data.fields)?;
    let existing = id.and_then(|id| db.entry(id));
    // A TOTP value the entry already has is kept as it is, even one this app
    // cannot read; a new one must work and is stored as a URI.
    let otp = match (data.otp.trim(), existing.as_ref().and_then(|e| e.get(fields::OTP))) {
        ("", _) => None,
        (_, Some(current)) if current == data.otp => Some(data.otp.clone()),
        (value, _) => Some(otp::normalize(value, &data.title)?),
    };
    // Group paths are names, which need not be unique or tidy: an entry whose
    // path did not change stays in its own group.
    let current_group = existing.as_ref().map(|e| e.parent().id()).filter(|g| path_of(db, *g) == data.group);
    let tags: Vec<String> = data.tags.iter().filter(|t| !t.trim().is_empty()).cloned().collect();
    let group = match current_group {
        Some(group) => group,
        None => group_at(db, &data.group, hidden)?,
    };

    let Some(id) = id.filter(|id| db.entry(*id).is_some()) else {
        let wanted = wanted_fields(db, None, data, otp)?;
        let mut parent = db.group_mut(group).ok_or("The group is gone")?;
        let mut entry = match id {
            Some(id) => parent.add_entry_with_id(id).map_err(|e| e.to_string())?,
            None => parent.add_entry(),
        };
        entry.fields = wanted;
        entry.tags = tags;
        let id = entry.id();
        db.deleted_objects.remove(&id.uuid());
        return Ok(id);
    };

    let entry = db.entry(id).expect("checked above");
    let wanted = wanted_fields(db, Some(&entry), data, otp)?;
    let moved = entry.parent().id() != group;
    if entry.fields == wanted && entry.tags == tags && !moved {
        return Ok(id);
    }
    let mut entry = db.entry_mut(id).expect("checked above");
    // Moved first and untracked: the file keeps no group for old versions, so
    // a version recorded before the move would not read back the same (and
    // KeePass does not record moves in history either).
    if moved {
        entry.move_to(group).map_err(|e| e.to_string())?;
        entry.times.location_changed = Some(Times::now());
    }
    if entry.fields != wanted || entry.tags != tags {
        let mut tracked = entry.track_changes();
        tracked.edit(|e| {
            e.fields = wanted;
            e.tags = tags;
        });
    } // dropping the tracker files the old version into the history
    trim_history(db, id);
    Ok(id)
}

/// Additional fields need a name that is neither a standard field's nor
/// taken; names are compared exactly, as KeePass does.
fn check_field_names(fields: &[FieldData]) -> Result<(), String> {
    let mut names = HashSet::new();
    for name in fields.iter().map(|f| f.name.as_str()) {
        if name.trim().is_empty() {
            return Err("A field needs a name".into());
        }
        if STANDARD.contains(&name) {
            return Err(format!("\"{name}\" is a standard field; use its own input"));
        }
        if !names.insert(name) {
            return Err(format!("There are two fields named \"{name}\""));
        }
    }
    Ok(())
}

/// The entry's fields as the editor wants them, each keeping whether it was
/// protected; a new password or TOTP secret is protected.
fn wanted_fields(
    db: &Database,
    entry: Option<&EntryRef<'_>>,
    data: &EntryData,
    otp: Option<String>,
) -> Result<HashMap<String, Value<String>>, String> {
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
        let protected = was_protected(name).unwrap_or(protect_by_default);
        wanted.insert(name.to_string(), value_of(value, protected));
    }
    if let Some(otp) = otp {
        let protected = was_protected(fields::OTP).unwrap_or(true);
        wanted.insert(fields::OTP.to_string(), value_of(&otp, protected));
    }
    for field in &data.fields {
        wanted.insert(field.name.clone(), value_of(&field.value, field.protected));
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

/// `group` and the groups above it, nearest first, ending with the root.
pub fn ancestors(db: &Database, group: GroupId) -> Vec<GroupId> {
    std::iter::successors(Some(group), |id| db.group(*id).and_then(|g| g.parent().map(|p| p.id()))).collect()
}

/// A group's names from the top, without the root group.
pub fn path_of(db: &Database, group: GroupId) -> Vec<String> {
    let mut chain = ancestors(db, group);
    chain.pop(); // the root group
    chain.iter().rev().filter_map(|id| db.group(*id).map(|g| g.name.clone())).collect()
}

/// The group at `path` below the root (the first of same-named siblings),
/// created where missing.
fn group_at(db: &mut Database, path: &[String], hidden: &HashSet<GroupId>) -> Result<GroupId, String> {
    let mut id = db.root().id();
    for name in path.iter().map(String::as_str).filter(|n| !n.is_empty()) {
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

/// Keeps what this device changed that a file read from disk does not have —
/// it came back older (a sync client delivered a stale copy, or a conflict
/// copy won). Every entry of `ours` changed later than in `theirs`, or missing
/// there with no deletion recorded at or after our change, is applied to
/// `theirs` (which keeps its own version in the entry's history); a newer
/// move — including one to the recycle bin — is kept too. Returns the ids of
/// the entries it kept.
pub fn keep_newer(theirs: &mut Database, ours: &Database) -> Vec<EntryId> {
    let hidden = hidden_groups(theirs);
    let our_bin = ours.meta.recyclebin_uuid.map(GroupId::from);
    let mut kept = Vec::new();
    for e in ours.iter_all_entries() {
        let id = e.id();
        let binned = our_bin.is_some_and(|bin| ancestors(ours, e.parent().id()).contains(&bin));
        // What to write: our content if it is newer, their content otherwise;
        // our place if the move is newer, their place otherwise.
        let (data, to_bin) = match theirs.entry(id) {
            Some(t) => {
                let newer_content = e.times.last_modification > t.times.last_modification;
                let newer_place = e.times.location_changed > t.times.location_changed;
                if !newer_content && !newer_place {
                    continue;
                }
                let their_place = path_of(theirs, t.parent().id());
                let mut data = read(if newer_content { &e } else { &t }, their_place);
                if newer_place && !binned {
                    data.group = path_of(ours, e.parent().id());
                }
                (data, newer_place && binned)
            }
            None => {
                let deleted_after = theirs.deleted_objects.get(&id.uuid()).is_some_and(|at| at.is_none() || *at >= e.times.last_modification);
                if deleted_after || binned {
                    continue;
                }
                (read(&e, path_of(ours, e.parent().id())), false)
            }
        };
        let before = theirs.entry(id).map(|t| (*t).clone());
        if apply(theirs, Some(id), &data, &hidden).is_err() || (to_bin && recycle(theirs, id).is_err()) {
            continue; // e.g. a TOTP value this app cannot read: their version stays
        }
        if theirs.entry(id).map(|t| (*t).clone()) != before {
            kept.push(id);
        }
    }
    kept
}

/// Merges this device's copy of the database (`ours`) into another device's
/// (`theirs`), both changed since they were last the same: `theirs` keeps
/// what it changed, gets what `ours` changed later ([keep_newer]), and every
/// version of an entry that only `ours` had goes into the entry's history —
/// including this device's edit where the other device's later edit won.
/// Returns the ids of the entries it changed in `theirs`.
pub fn merge(theirs: &mut Database, ours: &Database) -> Vec<EntryId> {
    let mut changed = keep_newer(theirs, ours);
    for id in join_history(theirs, ours) {
        if !changed.contains(&id) {
            changed.push(id);
        }
    }
    changed
}

/// Adds to `theirs`' entries the versions `ours` has (its current one and its
/// history) that `theirs` has neither as its current content nor in history,
/// newest first. Versions are told apart by their modification time; a
/// version is stored with the entry's own icon and attachments, which this
/// app never changes.
fn join_history(theirs: &mut Database, ours: &Database) -> Vec<EntryId> {
    let mut joined = Vec::new();
    for e in ours.iter_all_entries() {
        let id = e.id();
        let Some(t) = theirs.entry(id) else { continue };
        let history = t.history.as_ref().map(|h| h.get_entries().clone()).unwrap_or_default();
        let known: Vec<_> = history.iter().map(|v| v.times.last_modification).chain([t.times.last_modification]).collect();
        let ours_history = e.history.as_ref().map(|h| h.get_entries().as_slice()).unwrap_or_default();
        let missing: Vec<Entry> = std::iter::once(&*e)
            .chain(ours_history)
            .filter(|v| !known.contains(&v.times.last_modification) && (v.fields != t.fields || v.tags != t.tags))
            .map(|v| {
                let mut old = (*t).clone();
                old.fields = v.fields.clone();
                old.tags = v.tags.clone();
                old.times = v.times.clone();
                old
            })
            .collect();
        if missing.is_empty() {
            continue;
        }
        let mut all: Vec<Entry> = history.into_iter().chain(missing).collect();
        all.sort_by_key(|v| v.times.last_modification); // oldest first
        let mut joined_history = History::default();
        for version in all {
            joined_history.add_entry(version); // adds at the front: newest first
        }
        theirs.entry_mut(id).expect("checked above").history = Some(joined_history);
        trim_history(theirs, id);
        joined.push(id);
    }
    joined
}

/// True when an edit (`data` against the entry as the editor opened it,
/// `base`) changed only the tags.
pub fn only_tags_changed(base: &EntryData, data: &EntryData) -> bool {
    let mut same_tags = data.clone();
    same_tags.tags = base.tags.clone();
    base.tags != data.tags && same_tags == *base
}

/// Moves the entry to the recycle bin, creating the bin if the database has
/// none. A database that turned the bin off is refused: removing an entry
/// with keepass-rs can leave other entries' attachments pointing at the
/// wrong data.
pub fn recycle(db: &mut Database, id: EntryId) -> Result<(), String> {
    db.entry(id).ok_or(NOT_FOUND)?;
    if db.meta.recyclebin_enabled == Some(false) {
        return Err("This database has its recycle bin turned off; turn it on (in KeePassXC) to delete entries".into());
    }
    let bin = match db.recycle_bin().map(|g| g.id()) {
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
            d.tags = vec!["a".into(), " ".into()];
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
        for fields in [vec![field(" ")], vec![field("Password")], vec![field("A"), field("A")]] {
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
    fn an_entry_deleted_elsewhere_comes_back_when_edited() {
        let mut db = Database::new();
        let id = apply(&mut db, None, &data("x"), &HashSet::new()).unwrap();
        let edited = with("x", |d| d.password = "newer".into());
        db.root_mut().entry_mut(id).unwrap().track_changes().remove();
        assert!(db.deleted_objects.contains_key(&id.uuid()));

        assert_eq!(apply(&mut db, Some(id), &edited, &HashSet::new()).unwrap(), id);
        assert_eq!(db.entry(id).unwrap().get_password(), Some("newer"));
        assert!(!db.deleted_objects.contains_key(&id.uuid()));
    }

    fn field(name: &str, value: &str) -> FieldData {
        FieldData { name: name.into(), value: value.into(), protected: false }
    }

    #[test]
    fn merge3_keeps_what_the_editor_did_not_touch() {
        let base = with("x", |d| {
            d.group = vec!["Work".into()];
            d.fields = vec![field("PIN", "1"), field("Old", "o")];
        });
        // Another device changed the user name, moved the entry and added a field.
        let current = with("x", |d| {
            d.username = "phone".into();
            d.group = vec!["Job".into()];
            d.fields = vec![field("PIN", "1"), field("Old", "o"), field("Phone", "p")];
        });
        // The editor changed the password and removed a field.
        let edited = with("x", |d| {
            d.password = "pc".into();
            d.group = vec!["Work".into()];
            d.fields = vec![field("PIN", "1")];
        });
        let (merged, conflicts) = merge3(&current, &base, &edited);
        assert_eq!((merged.username.as_str(), merged.password.as_str()), ("phone", "pc"));
        assert_eq!(merged.group, ["Job"]);
        assert_eq!(merged.fields, [field("PIN", "1"), field("Phone", "p")]);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn merge3_reports_what_both_changed() {
        let base = with("x", |d| d.fields = vec![field("PIN", "1")]);
        let current = with("x", |d| {
            d.password = "phone".into();
            d.fields = vec![field("PIN", "2")];
        });
        let edited = with("x", |d| {
            d.password = "pc".into();
            d.fields = vec![field("PIN", "3")];
        });
        let (merged, conflicts) = merge3(&current, &base, &edited);
        assert_eq!(merged.password, "pc");
        assert_eq!(merged.fields, [field("PIN", "3")]);
        assert_eq!(conflicts, ["Password", "PIN"]);
    }

    #[test]
    fn keep_newer_restores_what_an_older_file_lacks() {
        let mut theirs = Database::new();
        let old = apply(&mut theirs, None, &data("old"), &HashSet::new()).unwrap();
        let removed = apply(&mut theirs, None, &data("removed there"), &HashSet::new()).unwrap();
        let mut ours = theirs.clone();
        // Changed and created here; the older file has neither.
        apply(&mut ours, Some(old), &with("old", |d| d.password = "newer".into()), &HashSet::new()).unwrap();
        let created = apply(&mut ours, None, &data("new here"), &HashSet::new()).unwrap();
        // Deleted there after our last change to it.
        theirs.root_mut().entry_mut(removed).unwrap().track_changes().remove();
        // Times have one-second precision: make their copy of `old` clearly older.
        theirs.entry_mut(old).unwrap().times.last_modification = Some(Times::epoch());

        let kept = keep_newer(&mut theirs, &ours);
        assert_eq!(kept.len(), 2);
        assert_eq!(theirs.entry(old).unwrap().get_password(), Some("newer"));
        assert_eq!(theirs.entry(created).unwrap().get_title(), Some("new here"));
        assert!(theirs.entry(removed).is_none());
        // Nothing newer the second time.
        assert!(keep_newer(&mut theirs, &ours).is_empty());
    }

    #[test]
    fn merging_keeps_the_losing_edit_in_history() {
        let mut ours = Database::new();
        let both = apply(&mut ours, None, &data("both"), &HashSet::new()).unwrap();
        let mut theirs = ours.clone();
        // Edited here first, then on the other device: theirs is newer and wins.
        apply(&mut ours, Some(both), &with("both", |d| d.password = "here".into()), &HashSet::new()).unwrap();
        ours.entry_mut(both).unwrap().times.last_modification = Some(Times::epoch());
        apply(&mut theirs, Some(both), &with("both", |d| d.password = "there".into()), &HashSet::new()).unwrap();
        let only_there = apply(&mut theirs, None, &data("new there"), &HashSet::new()).unwrap();

        let changed = merge(&mut theirs, &ours);
        assert_eq!(changed, [both]);
        let entry = theirs.entry(both).unwrap();
        assert_eq!(entry.get_password(), Some("there"));
        let history: Vec<_> = entry.history.as_ref().unwrap().get_entries().iter().map(|v| v.get_password().unwrap().to_string()).collect();
        assert_eq!(history, ["pw1", "here"]);
        assert!(theirs.entry(only_there).is_some());
        // Merging again finds nothing new.
        assert!(merge(&mut theirs, &ours).is_empty());
    }

    #[test]
    fn a_tags_only_edit_is_told_apart() {
        let base = data("x");
        assert!(only_tags_changed(&base, &with("x", |d| d.tags = vec!["a".into()])));
        assert!(!only_tags_changed(&base, &with("x", |d| {
            d.tags = vec!["a".into()];
            d.password = "new".into();
        })));
        assert!(!only_tags_changed(&base, &base.clone()));
    }

    #[test]
    fn deleting_without_a_recycle_bin_is_refused() {
        let mut db = Database::new();
        db.meta.recyclebin_enabled = Some(false);
        let id = apply(&mut db, None, &data("x"), &HashSet::new()).unwrap();
        assert!(recycle(&mut db, id).is_err());
        assert!(db.entry(id).is_some());
    }

    #[test]
    fn an_entry_stays_in_its_group_when_the_path_is_unchanged() {
        let mut db = Database::new();
        let mut root = db.root_mut();
        for name in ["Mail", "Mail", "TCP/IP", " spaced "] {
            root.add_group().name = name.into();
        }
        let groups: Vec<GroupId> = db.root().groups().map(|g| g.id()).collect();
        for group in groups {
            let id = db.group_mut(group).unwrap().add_entry().edit(|e| e.set_unprotected(fields::TITLE, "t")).id();
            let mut changed = read(&db.entry(id).unwrap(), path_of(&db, group));
            changed.password = "new".into();
            apply(&mut db, Some(id), &changed, &HashSet::new()).unwrap();
            assert_eq!(db.entry(id).unwrap().parent().id(), group);
        }
        assert_eq!(db.root().groups().count(), 4);
    }

    #[test]
    fn values_the_editor_did_not_touch_stay_as_they_were() {
        let mut db = Database::new();
        let id = db
            .root_mut()
            .add_entry()
            .edit(|e| {
                e.set_unprotected(fields::TITLE, "t");
                e.set_unprotected(fields::PASSWORD, "plain");
                e.set_unprotected(fields::OTP, "otpauth://hotp/x?secret=JBSWY3DP&counter=1");
                e.set_unprotected("OTP", "a field, not the standard one");
            })
            .id();
        let before = db.clone();
        let same = read(&db.entry(id).unwrap(), vec![]);
        apply(&mut db, Some(id), &same, &HashSet::new()).unwrap();
        assert_eq!(db, before);
    }
}
