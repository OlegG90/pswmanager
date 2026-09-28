//! Changes to the database: an entry as the editor sees it, applied back with
//! its previous version kept in the entry's history.

use crate::otp;
use base64::Engine;
use keepass::db::{fields, CustomIconId, Entry, EntryId, EntryRef, GroupId, History, Icon, Times, Value};
use chrono::{NaiveDateTime, SecondsFormat, Timelike};
use keepass::Database;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// KeePass keeps this many versions when the database does not say.
const DEFAULT_HISTORY_ITEMS: usize = 10;
const RECYCLE_BIN_ICON: usize = 43;
/// The largest file that can be attached.
pub const MAX_ATTACHMENT: usize = 20 << 20;

pub const NOT_FOUND: &str = "That entry is no longer in the database";

/// The star is this tag, as `sic2kdbx` writes SafeInCloud's; other clients see a tag.
pub const FAVORITE: &str = "Favorite";

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
    #[zeroize(skip)]
    pub icon: IconChoice,
    /// When the entry expires (UTC, RFC 3339); `None` when it does not.
    #[zeroize(skip)]
    pub expires: Option<String>,
}

/// The entry's icon as the editor chooses it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum IconChoice {
    /// The site's icon when the URL has one, else the default: KeePass's
    /// standard icon 0 (the key), which other clients show as usual.
    #[default]
    Auto,
    /// One of KeePass's standard icons, by number; other clients show their
    /// own picture for it.
    Builtin { id: usize },
    /// An image kept in the database (base64), shown by every client.
    Custom { data: String },
}

/// The largest image an entry's own icon may be.
pub const MAX_ICON: usize = 256 << 10;

fn icon_choice(entry: &EntryRef<'_>) -> IconChoice {
    if let Some(custom) = entry.custom_icon() {
        return IconChoice::Custom { data: base64::engine::general_purpose::STANDARD.encode(&custom.data) };
    }
    match entry.icon() {
        Some(Icon::BuiltIn(id)) if *id != 0 => IconChoice::Builtin { id: *id },
        _ => IconChoice::Auto,
    }
}

/// The icon in the database's pool that holds `data`, if there is one.
fn pooled_icon(db: &Database, data: &[u8]) -> Option<CustomIconId> {
    db.iter_all_custom_icons().find(|icon| icon.data == data).map(|icon| icon.id())
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
        icon: icon_choice(entry),
        expires: expiry_of(&entry.times),
    }
}

/// When KeePass's *Expires* is on: the time, as the window gets it.
pub fn expiry_of(times: &Times) -> Option<String> {
    match (times.expires, times.expiry) {
        (Some(true), Some(at)) => Some(at.and_utc().to_rfc3339_opts(SecondsFormat::Secs, true)),
        _ => None,
    }
}

/// The time an expiry the window sent stands for, to the second (as the file keeps it).
fn parse_expiry(text: &str) -> Result<NaiveDateTime, String> {
    let at = chrono::DateTime::parse_from_rfc3339(text).map_err(|_| "The expiry date is not a date")?;
    Ok(at.naive_utc().with_nanosecond(0).expect("zero is a valid nanosecond"))
}

/// An expiry written the way [expiry_of] writes it; one that is not a time
/// stays as it is (saving it is refused).
fn same_form(expires: &Option<String>) -> Option<String> {
    let text = expires.as_deref()?;
    Some(parse_expiry(text).map_or_else(|_| text.to_string(), |at| at.and_utc().to_rfc3339_opts(SecondsFormat::Secs, true)))
}

/// The expiry the entry has, compared as times rather than as text.
fn expiry_time(times: &Times) -> Option<NaiveDateTime> {
    times.expiry.filter(|_| times.expires == Some(true))
}

/// Turns *Expires* on at `at`, or off (keeping the old time, as KeePass does).
fn set_expiry(times: &mut Times, at: Option<NaiveDateTime>) {
    times.expires = Some(at.is_some());
    if at.is_some() {
        times.expiry = at;
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
    let icon = pick(&mut conflicts, "Icon", &current.icon, &base.icon, &edited.icon);
    // Compared as times: the window writes them with milliseconds.
    let expires = pick(
        &mut conflicts,
        "Expires",
        &same_form(&current.expires),
        &same_form(&base.expires),
        &same_form(&edited.expires),
    );

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
    let merged = EntryData { title, username, password, url, notes, otp, tags, group, fields, icon, expires };
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
    let icon = IconSetting::of(db, &data.icon)?;
    let expiry = data.expires.as_deref().map(parse_expiry).transpose()?;
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
        set_expiry(&mut entry.times, expiry);
        let id = entry.id();
        db.deleted_objects.remove(&id.uuid());
        let mut entry = db.entry_mut(id).expect("just added");
        match icon {
            IconSetting::Builtin(0) => {} // no icon: the key
            IconSetting::Builtin(n) => entry.set_icon_builtin(n),
            IconSetting::Pooled(icon) => {
                let _ = entry.set_icon_custom(icon); // checked to be in the pool
            }
            IconSetting::New(bytes) => {
                entry.set_icon_custom_new(bytes);
            }
        }
        return Ok(id);
    };

    let entry = db.entry(id).expect("checked above");
    let wanted = wanted_fields(db, Some(&entry), data, otp)?;
    let moved = entry.parent().id() != group;
    let icon_changed = icon_choice(&entry) != data.icon;
    let expiry_changed = expiry_time(&entry.times) != expiry;
    if entry.fields == wanted && entry.tags == tags && !moved && !icon_changed && !expiry_changed {
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
    if entry.fields != wanted || entry.tags != tags || icon_changed || expiry_changed {
        let mut tracked = entry.track_changes();
        tracked.edit(|e| {
            e.fields = wanted;
            e.tags = tags;
            if icon_changed {
                icon.set(e);
            }
            if expiry_changed {
                set_expiry(&mut e.times, expiry);
            }
        });
    } // dropping the tracker files the old version into the history
    trim_history(db, id);
    Ok(id)
}

/// An icon choice checked and ready to set: an image already in the pool is
/// shared rather than stored twice.
enum IconSetting {
    Builtin(usize),
    Pooled(CustomIconId),
    New(Vec<u8>),
}

impl IconSetting {
    fn of(db: &Database, choice: &IconChoice) -> Result<IconSetting, String> {
        Ok(match choice {
            IconChoice::Auto => IconSetting::Builtin(0),
            IconChoice::Builtin { id } => IconSetting::Builtin(*id),
            IconChoice::Custom { data } => {
                let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(|_| "The icon image is damaged")?;
                check_icon_image(&bytes)?;
                match pooled_icon(db, &bytes) {
                    Some(id) => IconSetting::Pooled(id),
                    None => IconSetting::New(bytes),
                }
            }
        })
    }

    fn set(self, entry: &mut keepass::db::EntryTrack<'_>) {
        match self {
            IconSetting::Builtin(id) => entry.set_icon_builtin(id),
            IconSetting::Pooled(id) => {
                let _ = entry.set_icon_custom(id); // checked to be in the pool
            }
            IconSetting::New(bytes) => {
                entry.set_icon_custom_new(bytes);
            }
        }
    }
}

/// An entry's own icon must be an image every client can show: PNG, JPEG,
/// GIF or WebP (Keepass2Android cannot show ICO or SVG), and small.
pub fn check_icon_image(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_ICON {
        return Err(format!("An icon can be {} KB at most", MAX_ICON >> 10));
    }
    match crate::icons::sniff(bytes) {
        Some("image/png" | "image/jpeg" | "image/gif" | "image/webp") => Ok(()),
        _ => Err("An icon must be a PNG, JPEG, GIF or WebP image".into()),
    }
}

/// Attaches a file to the entry, its previous version kept in history, and
/// returns the name it got: a name the entry already uses gets a number
/// (`scan (2).pdf`), because keepass-rs replaces an attachment by removing
/// the old one from the database, taking it from the history too.
pub fn attach(db: &mut Database, id: EntryId, name: &str, data: &[u8], hidden: &HashSet<GroupId>) -> Result<String, String> {
    let entry = db.entry(id).ok_or(NOT_FOUND)?;
    if ancestors(db, entry.parent().id()).iter().any(|g| hidden.contains(g)) {
        return Err(NOT_FOUND.into());
    }
    if data.len() > MAX_ATTACHMENT {
        return Err(too_big());
    }
    let name = free_name(&entry, name);
    {
        let mut entry = db.entry_mut(id).expect("checked above");
        let mut tracked = entry.track_changes();
        tracked.add_attachment(name.clone(), Value::protected(data.to_vec()));
    } // dropping the tracker files the old version into the history
    trim_history(db, id);
    Ok(name)
}

pub fn too_big() -> String {
    format!("Files over {} MB cannot be attached: the whole database is synced on every change", MAX_ATTACHMENT >> 20)
}

/// `name`, or with a number before its extension if the entry has a file by that name.
fn free_name(entry: &EntryRef<'_>, name: &str) -> String {
    let name = match name.trim() {
        "" => "file",
        name => name,
    };
    if entry.attachment_by_name(name).is_none() {
        return name.to_string();
    }
    let (stem, extension) = match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name, ""),
    };
    (2..).map(|n| format!("{stem} ({n}){extension}")).find(|n| entry.attachment_by_name(n).is_none()).expect("a free name")
}

/// Removes the file `name` from the entry, its previous version (which
/// keeps the file) filed into the history. Nothing to do when the entry has
/// no such file (removed elsewhere already).
pub fn detach(db: &mut Database, id: EntryId, name: &str, hidden: &HashSet<GroupId>) -> Result<(), String> {
    let entry = db.entry(id).ok_or(NOT_FOUND)?;
    if ancestors(db, entry.parent().id()).iter().any(|g| hidden.contains(g)) {
        return Err(NOT_FOUND.into());
    }
    if entry.attachment_by_name(name).is_none() {
        return Ok(());
    }
    let before = (*entry).clone();
    drop_attachments(db, id, &[name.to_string()]);
    file_in_history(db, id, before);
    trim_history(db, id);
    Ok(())
}

/// Renames the entry's file `from` to `to` and returns the new name; the
/// previous version, with the old name, goes to history.
pub fn rename_attachment(db: &mut Database, id: EntryId, from: &str, to: &str, hidden: &HashSet<GroupId>) -> Result<String, String> {
    let to = to.trim();
    let entry = db.entry(id).ok_or(NOT_FOUND)?;
    if ancestors(db, entry.parent().id()).iter().any(|g| hidden.contains(g)) {
        return Err(NOT_FOUND.into());
    }
    let data = entry.attachment_by_name(from).ok_or("That file is no longer in the entry")?.data.clone();
    if to == from {
        return Ok(to.to_string());
    }
    if to.is_empty() {
        return Err("A file needs a name".into());
    }
    if entry.attachment_by_name(to).is_some() {
        return Err(format!("The entry already has a file named \"{to}\""));
    }
    swap_attachment(db, id, from, to, data);
    Ok(to.to_string())
}

/// Gives the entry's file `name` new content; the previous version, with the
/// old content, goes to history. Content that did not change changes nothing.
pub fn replace_attachment(db: &mut Database, id: EntryId, name: &str, data: &[u8], hidden: &HashSet<GroupId>) -> Result<(), String> {
    let entry = db.entry(id).ok_or(NOT_FOUND)?;
    if ancestors(db, entry.parent().id()).iter().any(|g| hidden.contains(g)) {
        return Err(NOT_FOUND.into());
    }
    if data.len() > MAX_ATTACHMENT {
        return Err(too_big());
    }
    let old = entry.attachment_by_name(name).ok_or("That file is no longer in the entry")?;
    if old.data.get() == data {
        return Ok(());
    }
    // Protected in memory as it was.
    let data = if old.data.is_protected() { Value::protected(data.to_vec()) } else { Value::unprotected(data.to_vec()) };
    swap_attachment(db, id, name, name, data);
    Ok(())
}

/// Replaces the entry's file `from` with `data` named `to`, the previous
/// version filed into history. keepass-rs can neither rename a file nor
/// change its content, so the entry lets go of the file (history keeps it,
/// see [drop_attachments]) and gets the content under the name anew.
fn swap_attachment(db: &mut Database, id: EntryId, from: &str, to: &str, data: Value<Vec<u8>>) {
    let before = (*db.entry(id).expect("checked by the caller")).clone();
    drop_attachments(db, id, &[from.to_string()]);
    db.entry_mut(id).expect("checked by the caller").add_attachment(to, data);
    file_in_history(db, id, before);
    trim_history(db, id);
}

/// Takes files off the entry's current version, leaving them in the
/// database for the versions in history that still have them.
/// keepass-rs cannot do that: it drops a file from the database as soon as
/// the current version lets go of it, even while history refers to it. So the
/// files are removed on a copy of the database and only the entry is taken
/// from it: it refers to the same files as before, minus these.
fn drop_attachments(db: &mut Database, id: EntryId, names: &[String]) {
    let mut scratch = db.clone();
    {
        let mut entry = scratch.entry_mut(id).expect("checked by the caller");
        for name in names {
            entry.remove_attachment_by_name(name);
        }
    }
    let updated = (*scratch.entry(id).expect("checked by the caller")).clone();
    *db.entry_mut(id).expect("checked by the caller") = updated;
}

/// Files `before` into the entry's history, as a tracked edit does.
fn file_in_history(db: &mut Database, id: EntryId, before: Entry) {
    let mut entry = db.entry_mut(id).expect("checked by the caller");
    entry.history.get_or_insert_default().add_entry(before);
    entry.times.last_modification = Some(Times::now());
}

/// Gives `theirs`' entry the files of `ours`, the newer version: files it
/// lacks are added, and files `ours` does not have leave its current version
/// (they stay in history). Returns true when anything changed.
fn take_attachments(theirs: &mut Database, ours: &EntryRef<'_>) -> bool {
    let id = ours.id();
    let Some(t) = theirs.entry(id) else { return false };
    let wanted: Vec<(String, Value<Vec<u8>>)> = ours.attachments_named().map(|(name, a)| (name.to_string(), a.data.clone())).collect();
    let has = |files: &[(String, Value<Vec<u8>>)], name: &str, data: &[u8]| files.iter().any(|(n, d)| n == name && d.get() == data);
    let extra: Vec<String> =
        t.attachments_named().filter(|(name, a)| !has(&wanted, name, a.data.get())).map(|(name, _)| name.to_string()).collect();
    let missing: Vec<(String, Value<Vec<u8>>)> =
        wanted.into_iter().filter(|(name, data)| t.attachment_by_name(name).is_none_or(|a| a.data.get() != data.get())).collect();
    if extra.is_empty() && missing.is_empty() {
        return false;
    }
    drop_attachments(theirs, id, &extra);
    // Every name is free now: a file by that name with other content was extra.
    for (name, data) in missing {
        theirs.entry_mut(id).expect("checked above").add_attachment(name, data);
    }
    true
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
/// move is kept too. A move to the recycle bin (a deletion) on either side
/// stands unless the other side changed the entry later. Returns the ids of
/// the entries it kept.
pub fn keep_newer(theirs: &mut Database, ours: &Database) -> Vec<EntryId> {
    let hidden = hidden_groups(theirs);
    let mut kept = Vec::new();
    for e in ours.iter_all_entries() {
        let id = e.id();
        let binned = is_binned(ours, &e);
        // What to write: our content if it is newer, their content otherwise;
        // our place if the move is newer, their place otherwise.
        let (data, to_bin, our_content) = match theirs.entry(id) {
            Some(t) => {
                let (ours_at, theirs_at) = (&e.times, &t.times);
                let newer_content = ours_at.last_modification > theirs_at.last_modification;
                let our_place = if binned {
                    ours_at.location_changed > theirs_at.location_changed
                        && ours_at.location_changed > theirs_at.last_modification
                } else {
                    let restored = is_binned(theirs, &t) && ours_at.last_modification > theirs_at.location_changed;
                    ours_at.location_changed > theirs_at.location_changed || restored
                };
                if !newer_content && !our_place {
                    continue;
                }
                let their_place = path_of(theirs, t.parent().id());
                let mut data = read(if newer_content { &e } else { &t }, their_place);
                if our_place && !binned {
                    data.group = path_of(ours, e.parent().id());
                }
                (data, our_place && binned, newer_content)
            }
            None => {
                let deleted_after = theirs.deleted_objects.get(&id.uuid()).is_some_and(|at| at.is_none() || *at >= e.times.last_modification);
                if deleted_after || binned {
                    continue;
                }
                (read(&e, path_of(ours, e.parent().id())), false, true)
            }
        };
        let before = theirs.entry(id).map(|t| (*t).clone());
        if apply(theirs, Some(id), &data, &hidden).is_err() || (to_bin && recycle(theirs, id).is_err()) {
            continue; // e.g. a TOTP value this app cannot read: their version stays
        }
        // Whether `apply` filed their version into history: it does when it
        // changes the fields or tags (a history already at its limit keeps
        // its length, so the length cannot tell).
        let filed = before.as_ref().is_some_and(|b| theirs.entry(id).is_some_and(|t| t.fields != b.fields || t.tags != b.tags));
        if our_content && take_attachments(theirs, &e) {
            // Their version goes to history, unless changing the fields already put it there.
            if let Some(before) = before.clone().filter(|_| !filed) {
                file_in_history(theirs, id, before);
                trim_history(theirs, id);
            }
        }
        if theirs.entry(id).map(|t| (*t).clone()) != before {
            kept.push(id);
        }
    }
    kept
}

/// True when the entry is in its database's recycle bin.
fn is_binned(db: &Database, entry: &EntryRef<'_>) -> bool {
    let bin = db.meta.recyclebin_uuid.map(GroupId::from);
    bin.is_some_and(|bin| ancestors(db, entry.parent().id()).contains(&bin))
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
/// version is stored with the entry's own icon and attachments (keepass-rs
/// cannot give an old version other files; this device's files are taken by
/// [keep_newer] when its version is the newer one).
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

/// Gives the entries the tag, or takes it off (the tag [FAVORITE] is the
/// star). Entries gone, templates and the recycle bin's (`hidden`) are left
/// out, and so are entries already as asked.
pub fn set_tag(db: &mut Database, ids: &[EntryId], tag: &str, on: bool, hidden: &HashSet<GroupId>) -> Result<(), String> {
    let tag = tag_name(tag)?;
    for &id in ids {
        let Some(entry) = db.entry(id) else { continue };
        if ancestors(db, entry.parent().id()).iter().any(|g| hidden.contains(g)) {
            continue;
        }
        retag(db, id, |tags| {
            let mut tags: Vec<String> = tags.iter().filter(|t| **t != tag).cloned().collect();
            if on {
                tags.push(tag.clone());
            }
            tags
        });
    }
    Ok(())
}

/// Renames the tag in every entry (templates and the recycle bin's too); to a
/// name another tag has, the two become one.
pub fn rename_tag(db: &mut Database, from: &str, to: &str) -> Result<(), String> {
    let to = tag_name(to)?;
    if to == FAVORITE {
        return Err(format!("{FAVORITE} is the star: choose another name"));
    }
    for id in all_entries(db) {
        retag(db, id, |tags| {
            let mut renamed: Vec<String> = Vec::with_capacity(tags.len());
            for tag in tags {
                let tag = if tag == from { to.clone() } else { tag.clone() };
                if !renamed.contains(&tag) {
                    renamed.push(tag);
                }
            }
            renamed
        });
    }
    Ok(())
}

/// Takes the tag off every entry (templates and the recycle bin's too); the entries stay.
pub fn remove_tag(db: &mut Database, tag: &str) {
    for id in all_entries(db) {
        retag(db, id, |tags| tags.iter().filter(|t| *t != tag).cloned().collect());
    }
}

fn all_entries(db: &Database) -> Vec<EntryId> {
    db.iter_all_entries().map(|e| e.id()).collect()
}

/// A tag as KeePass stores it: commas and semicolons separate tags.
fn tag_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("A tag needs a name".into());
    }
    if name.contains([',', ';']) {
        return Err("A tag cannot contain a comma or a semicolon".into());
    }
    Ok(name.to_string())
}

/// Gives the entry the tags `change` makes of its own, its previous version
/// kept in history. Nothing changes when they are the same.
fn retag(db: &mut Database, id: EntryId, change: impl FnOnce(&[String]) -> Vec<String>) {
    let Some(entry) = db.entry(id) else { return };
    let tags = change(&entry.tags);
    if tags == entry.tags {
        return;
    }
    db.entry_mut(id).expect("checked above").track_changes().edit(|e| e.tags = tags);
    // the tracker, dropped above, filed the old version into the history
    trim_history(db, id);
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
    fn expiry_is_set_kept_and_turned_off() {
        let mut db = Database::new();
        let id = apply(&mut db, None, &with("x", |d| d.expires = Some("2030-01-02T03:04:05.678Z".into())), &HashSet::new()).unwrap();
        let times = &db.entry(id).unwrap().times;
        assert_eq!(times.expires, Some(true));
        assert_eq!(expiry_of(times).as_deref(), Some("2030-01-02T03:04:05Z"));

        // The same time written another way changes nothing.
        let same = with("x", |d| d.expires = Some("2030-01-02T05:04:05+02:00".into()));
        apply(&mut db, Some(id), &same, &HashSet::new()).unwrap();
        assert_eq!(history_len(&db, id), 0);

        // Turned off: the old version keeps it; KeePass's time stays.
        apply(&mut db, Some(id), &data("x"), &HashSet::new()).unwrap();
        let entry = db.entry(id).unwrap();
        assert_eq!(read(&entry, vec![]).expires, None);
        assert!(entry.times.expiry.is_some());
        assert_eq!(expiry_of(&entry.historical(0).unwrap().times).as_deref(), Some("2030-01-02T03:04:05Z"));
        assert!(apply(&mut db, Some(id), &with("x", |d| d.expires = Some("soon".into())), &HashSet::new()).is_err());
    }

    #[test]
    fn merge3_compares_expiry_as_times() {
        let base = data("x");
        let current = with("x", |d| d.expires = Some("2030-01-01T22:00:00Z".into()));
        let edited = with("x", |d| d.expires = Some("2030-01-01T22:00:00.000Z".into()));
        let (merged, conflicts) = merge3(&current, &base, &edited);
        assert_eq!(merged.expires.as_deref(), Some("2030-01-01T22:00:00Z"));
        assert!(conflicts.is_empty());
    }

    #[test]
    fn tags_are_set_on_several_entries_with_history() {
        let mut db = Database::new();
        let a = apply(&mut db, None, &with("a", |d| d.tags = vec!["work".into()]), &HashSet::new()).unwrap();
        let b = apply(&mut db, None, &data("b"), &HashSet::new()).unwrap();
        let binned = apply(&mut db, None, &data("binned"), &HashSet::new()).unwrap();
        let bin = db.entry(binned).unwrap().parent().id();
        let tags = |db: &Database, id: EntryId| db.entry(id).unwrap().tags.clone();
        set_tag(&mut db, &[a, b], FAVORITE, true, &HashSet::new()).unwrap();
        assert_eq!((tags(&db, a), tags(&db, b)), (vec!["work".to_string(), FAVORITE.into()], vec![FAVORITE.to_string()]));
        assert_eq!((history_len(&db, a), history_len(&db, b)), (1, 1));
        // Already so: nothing changes.
        set_tag(&mut db, &[a], FAVORITE, true, &HashSet::new()).unwrap();
        assert_eq!(history_len(&db, a), 1);
        set_tag(&mut db, &[a, b], " work ", false, &HashSet::new()).unwrap();
        assert_eq!((tags(&db, a), history_len(&db, b)), (vec![FAVORITE.to_string()], 1));
        // Entries gone and hidden ones are left out.
        let gone = EntryId::from(uuid::Uuid::new_v4());
        set_tag(&mut db, &[gone, binned], "x", true, &HashSet::from([bin])).unwrap();
        assert!(tags(&db, binned).is_empty());
        assert!(set_tag(&mut db, &[a], "a,b", true, &HashSet::new()).is_err());
        assert!(set_tag(&mut db, &[a], " ", true, &HashSet::new()).is_err());
    }

    #[test]
    fn a_tag_is_renamed_merged_and_removed_everywhere() {
        let mut db = Database::new();
        let a = apply(&mut db, None, &with("a", |d| d.tags = vec!["old".into(), "x".into()]), &HashSet::new()).unwrap();
        let b = apply(&mut db, None, &with("b", |d| d.tags = vec!["new".into(), "old".into()]), &HashSet::new()).unwrap();
        let c = apply(&mut db, None, &data("c"), &HashSet::new()).unwrap();
        rename_tag(&mut db, "old", " new ").unwrap();
        let tags = |db: &Database, id: EntryId| db.entry(id).unwrap().tags.clone();
        assert_eq!(tags(&db, a), ["new", "x"]);
        assert_eq!(tags(&db, b), ["new"]);
        assert_eq!(history_len(&db, c), 0);
        assert!(rename_tag(&mut db, "new", FAVORITE).is_err());
        remove_tag(&mut db, "new");
        assert_eq!((tags(&db, a), tags(&db, b)), (vec!["x".to_string()], vec![]));
        assert_eq!(history_len(&db, b), 2);
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
    fn keep_newer_files_their_version_once_when_history_is_full() {
        let mut theirs = Database::new();
        theirs.meta.history_max_items = Some(2);
        let id = apply(&mut theirs, None, &data("x"), &HashSet::new()).unwrap();
        for n in 0..3 {
            attach(&mut theirs, id, &format!("{n}.txt"), b"x", &HashSet::new()).unwrap();
        }
        assert_eq!(history_len(&theirs, id), 2);
        let mut ours = theirs.clone();
        apply(&mut ours, Some(id), &with("x", |d| d.password = "newer".into()), &HashSet::new()).unwrap();
        attach(&mut ours, id, "new.txt", b"new", &HashSet::new()).unwrap();
        theirs.entry_mut(id).unwrap().times.last_modification = Some(Times::epoch());

        keep_newer(&mut theirs, &ours);
        let entry = theirs.entry(id).unwrap();
        let (newest, next) = (entry.historical(0).unwrap(), entry.historical(1).unwrap());
        // Their version once, then the one before it: not their version twice.
        assert_eq!(newest.attachments().count(), 3);
        assert_eq!(next.attachments().count(), 2);
    }

    fn files(db: &Database, id: EntryId) -> Vec<(String, Vec<u8>)> {
        let mut all: Vec<_> = db.entry(id).unwrap().attachments_named().map(|(n, a)| (n.to_string(), a.data.get().clone())).collect();
        all.sort();
        all
    }

    #[test]
    fn attaching_keeps_the_old_version_and_never_replaces_a_file() {
        let mut db = Database::new();
        let id = apply(&mut db, None, &data("x"), &HashSet::new()).unwrap();
        let attach = |db: &mut Database, name: &str, data: &[u8]| attach(db, id, name, data, &HashSet::new()).unwrap();
        assert_eq!(attach(&mut db, "scan.pdf", b"one"), "scan.pdf");
        assert_eq!(attach(&mut db, "scan.pdf", b"two"), "scan (2).pdf");
        assert_eq!(attach(&mut db, "scan.pdf", b"three"), "scan (3).pdf");
        assert_eq!(attach(&mut db, "README", b"r"), "README");
        assert_eq!(attach(&mut db, "README", b"r"), "README (2)");
        assert_eq!(attach(&mut db, ".env", b"e"), ".env");
        assert_eq!(attach(&mut db, ".env", b"e"), ".env (2)");
        assert_eq!(attach(&mut db, " ", b""), "file");
        assert_eq!(files(&db, id)[..3], [
            (".env".to_string(), b"e".to_vec()),
            (".env (2)".to_string(), b"e".to_vec()),
            ("README".to_string(), b"r".to_vec()),
        ]);
        assert_eq!(files(&db, id).iter().find(|(n, _)| n == "scan.pdf").unwrap().1, b"one");
        assert_eq!(history_len(&db, id), 8);
        assert_eq!(db.entry(id).unwrap().historical(0).unwrap().attachments().count(), 7);
    }

    #[test]
    fn attaching_is_refused_in_the_bin_and_for_big_files() {
        let mut db = Database::new();
        let id = apply(&mut db, None, &data("x"), &HashSet::new()).unwrap();
        assert!(attach(&mut db, id, "big", &vec![0; MAX_ATTACHMENT + 1], &HashSet::new()).is_err());
        let group = db.entry(id).unwrap().parent().id();
        assert!(attach(&mut db, id, "a", b"a", &HashSet::from([group])).is_err());
        assert_eq!(db.entry(id).unwrap().attachments().count(), 0);
    }

    #[test]
    fn keep_newer_takes_the_files_of_the_newer_version() {
        let mut theirs = Database::new();
        let old = apply(&mut theirs, None, &data("old"), &HashSet::new()).unwrap();
        attach(&mut theirs, old, "both.txt", b"same", &HashSet::new()).unwrap();
        attach(&mut theirs, old, "removed here.txt", b"gone", &HashSet::new()).unwrap();
        let mut ours = theirs.clone();
        attach(&mut ours, old, "here.txt", b"new here", &HashSet::new()).unwrap();
        attach(&mut ours, old, "clash.txt", b"ours", &HashSet::new()).unwrap();
        detach(&mut ours, old, "removed here.txt", &HashSet::new()).unwrap();
        let created = apply(&mut ours, None, &data("new here"), &HashSet::new()).unwrap();
        attach(&mut ours, created, "new.txt", b"with the entry", &HashSet::new()).unwrap();
        // Changed elsewhere, earlier: this device's version is newer.
        attach(&mut theirs, old, "there.txt", b"from the phone", &HashSet::new()).unwrap();
        attach(&mut theirs, old, "clash.txt", b"theirs", &HashSet::new()).unwrap();
        theirs.entry_mut(old).unwrap().times.last_modification = Some(Times::epoch());
        let their_versions = history_len(&theirs, old);

        assert_eq!(keep_newer(&mut theirs, &ours).len(), 2);
        let names = |files: Vec<(String, Vec<u8>)>| files.into_iter().map(|(n, d)| format!("{n}={}", String::from_utf8(d).unwrap())).collect::<Vec<_>>();
        assert_eq!(names(files(&theirs, old)), ["both.txt=same", "clash.txt=ours", "here.txt=new here"]);
        assert_eq!(names(files(&theirs, created)), ["new.txt=with the entry"]);
        // Their version, with its files, is in history.
        assert_eq!(history_len(&theirs, old), their_versions + 1);
        let previous = theirs.entry(old).unwrap().historical(0).unwrap().attachments_named().map(|(n, a)| (n.to_string(), a.data.get().clone())).collect::<Vec<_>>();
        let mut previous = names(previous);
        previous.sort();
        assert_eq!(previous, ["both.txt=same", "clash.txt=theirs", "removed here.txt=gone", "there.txt=from the phone"]);
        // Nothing newer the second time.
        assert!(keep_newer(&mut theirs, &ours).is_empty());
    }

    #[test]
    fn renaming_keeps_the_old_name_in_history() {
        let mut db = Database::new();
        let id = apply(&mut db, None, &data("x"), &HashSet::new()).unwrap();
        attach(&mut db, id, "a.txt", b"a", &HashSet::new()).unwrap();
        attach(&mut db, id, "b.txt", b"b", &HashSet::new()).unwrap();
        let rename = |db: &mut Database, from: &str, to: &str| rename_attachment(db, id, from, to, &HashSet::new());
        assert_eq!(rename(&mut db, "a.txt", " c.txt ").unwrap(), "c.txt");
        assert_eq!(files(&db, id), [("b.txt".to_string(), b"b".to_vec()), ("c.txt".to_string(), b"a".to_vec())]);
        let previous: Vec<String> = db.entry(id).unwrap().historical(0).unwrap().attachments_named().map(|(n, _)| n.to_string()).collect();
        assert!(previous.contains(&"a.txt".to_string()), "{previous:?}");
        // Refused: a name in use, no name, a file that is gone. The same name changes nothing.
        let before = db.clone();
        assert!(rename(&mut db, "c.txt", "b.txt").is_err());
        assert!(rename(&mut db, "c.txt", " ").is_err());
        assert!(rename(&mut db, "a.txt", "d.txt").is_err());
        assert_eq!(rename(&mut db, "c.txt", "c.txt").unwrap(), "c.txt");
        assert_eq!(db, before);
    }

    #[test]
    fn replacing_keeps_the_old_content_in_history() {
        let mut db = Database::new();
        let id = apply(&mut db, None, &data("x"), &HashSet::new()).unwrap();
        attach(&mut db, id, "a.txt", b"old", &HashSet::new()).unwrap();
        attach(&mut db, id, "b.txt", b"b", &HashSet::new()).unwrap();
        replace_attachment(&mut db, id, "a.txt", b"new", &HashSet::new()).unwrap();
        assert_eq!(files(&db, id), [("a.txt".to_string(), b"new".to_vec()), ("b.txt".to_string(), b"b".to_vec())]);
        let entry = db.entry(id).unwrap();
        assert_eq!(entry.historical(0).unwrap().attachment_by_name("a.txt").unwrap().data.get(), b"old");
        assert!(db.entry(id).unwrap().attachment_by_name("a.txt").unwrap().data.is_protected());
        // The same content, a file that is gone, a file too big: nothing changes.
        let before = db.clone();
        replace_attachment(&mut db, id, "a.txt", b"new", &HashSet::new()).unwrap();
        assert!(replace_attachment(&mut db, id, "gone.txt", b"x", &HashSet::new()).is_err());
        assert!(replace_attachment(&mut db, id, "a.txt", &vec![0; MAX_ATTACHMENT + 1], &HashSet::new()).is_err());
        assert_eq!(db, before);
    }

    #[test]
    fn detaching_keeps_the_file_for_history() {
        let mut db = Database::new();
        let id = apply(&mut db, None, &data("x"), &HashSet::new()).unwrap();
        attach(&mut db, id, "a.txt", b"a", &HashSet::new()).unwrap();
        attach(&mut db, id, "b.txt", b"b", &HashSet::new()).unwrap();
        let pool = db.num_attachments();
        detach(&mut db, id, "a.txt", &HashSet::new()).unwrap();
        assert_eq!(files(&db, id), [("b.txt".to_string(), b"b".to_vec())]);
        assert_eq!(db.num_attachments(), pool);
        assert_eq!(db.entry(id).unwrap().historical(0).unwrap().attachments().count(), 2);
        assert_eq!(history_len(&db, id), 3);
        // A file already gone changes nothing; one in the bin is out of reach.
        let before = db.clone();
        detach(&mut db, id, "a.txt", &HashSet::new()).unwrap();
        assert_eq!(db, before);
        let group = db.entry(id).unwrap().parent().id();
        assert!(detach(&mut db, id, "b.txt", &HashSet::from([group])).is_err());
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
    fn a_deletion_stands_unless_the_other_side_changed_the_entry_later() {
        let mut ours = Database::new();
        let binned_there = apply(&mut ours, None, &data("binned there"), &HashSet::new()).unwrap();
        let binned_here = apply(&mut ours, None, &data("binned here"), &HashSet::new()).unwrap();
        for id in [binned_there, binned_here] {
            ours.entry_mut(id).unwrap().times.last_modification = Some(Times::epoch());
            ours.entry_mut(id).unwrap().times.location_changed = Some(Times::epoch());
        }
        let mut theirs = ours.clone();
        // Deleted there, then edited here.
        recycle(&mut theirs, binned_there).unwrap();
        theirs.entry_mut(binned_there).unwrap().times.location_changed = Some(Times::epoch() + chrono::Duration::seconds(10));
        apply(&mut ours, Some(binned_there), &with("binned there", |d| d.password = "later".into()), &HashSet::new()).unwrap();
        // Deleted here, then edited there.
        recycle(&mut ours, binned_here).unwrap();
        ours.entry_mut(binned_here).unwrap().times.location_changed = Some(Times::epoch() + chrono::Duration::seconds(10));
        apply(&mut theirs, Some(binned_here), &with("binned here", |d| d.password = "later".into()), &HashSet::new()).unwrap();

        keep_newer(&mut theirs, &ours);
        let bin = theirs.meta.recyclebin_uuid.map(GroupId::from).unwrap();
        for id in [binned_there, binned_here] {
            let entry = theirs.entry(id).unwrap();
            assert_ne!(entry.parent().id(), bin, "{:?}", entry.get_title());
            assert_eq!(entry.get_password(), Some("later"));
        }
    }

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n....";

    fn custom(bytes: &[u8]) -> IconChoice {
        IconChoice::Custom { data: base64::engine::general_purpose::STANDARD.encode(bytes) }
    }

    #[test]
    fn the_icon_is_chosen_kept_and_shared() {
        let mut db = Database::new();
        let a = apply(&mut db, None, &with("a", |d| d.icon = IconChoice::Builtin { id: 37 }), &HashSet::new()).unwrap();
        assert_eq!(read(&db.entry(a).unwrap(), vec![]).icon, IconChoice::Builtin { id: 37 });

        // An image; a second entry with the same image shares it.
        apply(&mut db, Some(a), &with("a", |d| d.icon = custom(PNG)), &HashSet::new()).unwrap();
        let b = apply(&mut db, None, &with("b", |d| d.icon = custom(PNG)), &HashSet::new()).unwrap();
        assert_eq!(db.iter_all_custom_icons().count(), 1);
        assert_eq!(read(&db.entry(b).unwrap(), vec![]).icon, custom(PNG));
        // The version before the change keeps its icon in history.
        let entry = db.entry(a).unwrap();
        let old = &entry.history.as_ref().unwrap().get_entries()[0];
        assert!(matches!(old.icon(), Some(Icon::BuiltIn(37))));

        // Untouched: nothing changes; Auto: back to the key.
        let before = db.clone();
        let same = read(&db.entry(a).unwrap(), vec![]);
        apply(&mut db, Some(a), &same, &HashSet::new()).unwrap();
        assert_eq!(db, before);
        apply(&mut db, Some(a), &with("a", |d| d.icon = IconChoice::Auto), &HashSet::new()).unwrap();
        assert_eq!(read(&db.entry(a).unwrap(), vec![]).icon, IconChoice::Auto);
    }

    #[test]
    fn only_small_images_every_client_shows_are_icons() {
        let mut db = Database::new();
        for bad in [custom(b"<svg/>"), custom(&[0, 0, 1, 0]), custom(&[0x89; MAX_ICON + 1]), IconChoice::Custom { data: "!".into() }] {
            assert!(apply(&mut db, None, &with("x", |d| d.icon = bad), &HashSet::new()).is_err());
        }
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
