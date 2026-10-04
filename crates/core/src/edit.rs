//! Changes to the database: an entry as the editor sees it, applied back with
//! its previous version kept in the entry's history.

use crate::otp;
use base64::Engine;
use keepass::db::{fields, CustomIconId, Entry, EntryId, EntryRef, GroupId, History, Icon, Meta, Times, Value};
use chrono::{NaiveDateTime, SecondsFormat, Timelike};
use keepass::Database;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

/// KeePass keeps this many versions when the database does not say.
const DEFAULT_HISTORY_ITEMS: usize = 10;
/// KeePass keeps this much history per entry (in bytes) when the database does not say.
const DEFAULT_HISTORY_SIZE: usize = 6 << 20;
const RECYCLE_BIN_ICON: usize = 43;
/// The largest file that can be attached.
pub const MAX_ATTACHMENT: usize = 20 << 20;

pub const NOT_FOUND: &str = "That entry is no longer in the database";

/// The star is this tag, as `sic2kdbx` writes SafeInCloud's; other clients see a tag.
pub const FAVORITE: &str = "Favorite";

/// The attributes KeePassXC keeps a passkey in: their names all start with
/// [passkey::PREFIX].
pub(crate) mod passkey {
    pub const PREFIX: &str = "KPEX_PASSKEY_";
    pub const USERNAME: &str = "KPEX_PASSKEY_USERNAME";
    pub const CREDENTIAL_ID: &str = "KPEX_PASSKEY_CREDENTIAL_ID";
    pub const PRIVATE_KEY: &str = "KPEX_PASSKEY_PRIVATE_KEY_PEM";
    pub const RELYING_PARTY: &str = "KPEX_PASSKEY_RELYING_PARTY";
    pub const USER_HANDLE: &str = "KPEX_PASSKEY_USER_HANDLE";
}

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

pub fn icon_choice(entry: &EntryRef<'_>) -> IconChoice {
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

/// A change the editor makes to the entry's files, saved with the entry.
/// `C` is the new content: as the window sends it, the number of a file
/// staged in the backend ([crate::staged::Staged]), so content never passes
/// through the window; as applied, the bytes ([FileEdit]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FileChange<C = u64> {
    /// A new file; a name the entry already uses gets a number (`scan (2).pdf`).
    Add { name: String, content: C },
    /// One of the entry's files renamed to `to`.
    Rename { name: String, to: String },
    Remove { name: String },
}

/// A file change with its content.
pub type FileEdit = FileChange<Zeroizing<Vec<u8>>>;

/// The fields the editor has its own inputs for.
pub(crate) const STANDARD: [&str; 6] = [fields::TITLE, fields::USERNAME, fields::PASSWORD, fields::URL, fields::NOTES, fields::OTP];

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
        (Some(true), Some(at)) => Some(time_text(at)),
        _ => None,
    }
}

/// A time as the window gets it: UTC, RFC 3339, to the second.
pub fn time_text(at: NaiveDateTime) -> String {
    at.and_utc().to_rfc3339_opts(SecondsFormat::Secs, true)
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
    apply_tracked(db, id, data, hidden).map(|(id, _)| id)
}

/// [apply], and whether the entry's previous version went into its history.
fn apply_tracked(db: &mut Database, id: Option<EntryId>, data: &EntryData, hidden: &HashSet<GroupId>) -> Result<(EntryId, bool), String> {
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
        return Ok((id, false));
    };

    let entry = db.entry(id).expect("checked above");
    let wanted = wanted_fields(db, Some(&entry), data, otp)?;
    let moved = entry.parent().id() != group;
    let icon_changed = icon_choice(&entry) != data.icon;
    let expiry_changed = expiry_time(&entry.times) != expiry;
    if entry.fields == wanted && entry.tags == tags && !moved && !icon_changed && !expiry_changed {
        return Ok((id, false));
    }
    let mut entry = db.entry_mut(id).expect("checked above");
    // Moved first and untracked: the file keeps no group for old versions, so
    // a version recorded before the move would not read back the same (and
    // KeePass does not record moves in history either).
    if moved {
        entry.move_to(group).map_err(|e| e.to_string())?;
        entry.times.location_changed = Some(Times::now());
    }
    let tracked_edit = entry.fields != wanted || entry.tags != tags || icon_changed || expiry_changed;
    if tracked_edit {
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
    Ok((id, tracked_edit))
}

/// Saves the editor's `data` and file `changes` to the entry (a new one when
/// `id` is none) as one edit: one previous version goes into its history.
pub fn apply_with_files(
    db: &mut Database,
    id: Option<EntryId>,
    data: &EntryData,
    changes: &[FileEdit],
    hidden: &HashSet<GroupId>,
) -> Result<EntryId, String> {
    let before = id.and_then(|id| db.entry(id)).map(|e| (*e).clone());
    let (id, filed) = apply_tracked(db, id, data, hidden)?;
    if change_files(db, id, changes)? {
        if let Some(before) = before.filter(|_| !filed) {
            file_in_history(db, id, before);
        }
        trim_history(db, id);
    }
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

/// Makes `changes` to the entry's files, leaving its history to the caller;
/// true when anything changed. Removals go first, then renames (all at once,
/// so a chain like c→d, a→c or a swap works), then new files, so a name one
/// frees can be taken by another.
/// keepass-rs cannot rename a file: a renamed file is let go of (history
/// keeps it, see [drop_attachments]) and its content attached under the new
/// name. A new file never replaces one: keepass-rs would take the old one
/// from the history too.
fn change_files(db: &mut Database, id: EntryId, changes: &[FileEdit]) -> Result<bool, String> {
    let entry = db.entry(id).ok_or(NOT_FOUND)?;
    for change in changes {
        if let FileChange::Add { content, .. } = change {
            check_size(content.len() as u64)?;
        }
    }
    // A file removed on another device meanwhile needs nothing.
    let removed: Vec<String> = changes
        .iter()
        .filter_map(|c| match c {
            FileChange::Remove { name } if entry.attachment_by_name(name).is_some() => Some(name.clone()),
            _ => None,
        })
        .collect();
    let mut renamed: Vec<(String, String, Value<Vec<u8>>)> = Vec::new();
    for change in changes {
        let FileChange::Rename { name, to } = change else { continue };
        let to = to.trim();
        if to.is_empty() {
            return Err("A file needs a name".into());
        }
        // Removed on another device meanwhile: nothing left to name.
        let Some(old) = entry.attachment_by_name(name) else { continue };
        if to != name {
            renamed.push((name.clone(), to.to_string(), old.data.clone()));
        }
    }
    // Removed and renamed files are let go of together.
    let freed: Vec<String> = removed.into_iter().chain(renamed.iter().map(|(name, _, _)| name.clone())).collect();
    for (_, to, _) in &renamed {
        let taken = entry.attachment_by_name(to).is_some() && !freed.contains(to);
        if taken || renamed.iter().filter(|(_, other, _)| other == to).count() > 1 {
            return Err(format!("The entry already has a file named \"{to}\""));
        }
    }
    let mut changed = !freed.is_empty();
    drop_attachments(db, id, &freed);
    for (_, to, value) in renamed {
        db.entry_mut(id).expect("checked above").add_attachment(to, value);
    }
    for change in changes {
        let FileChange::Add { name, content } = change else { continue };
        let name = free_name(&db.entry(id).expect("checked above"), name);
        db.entry_mut(id).expect("checked above").add_attachment(name, Value::protected(content.to_vec()));
        changed = true;
    }
    Ok(changed)
}

/// A file of `len` bytes can be attached: up to [MAX_ATTACHMENT].
pub fn check_size(len: u64) -> Result<(), String> {
    if len > MAX_ATTACHMENT as u64 {
        return Err(format!("Files over {} MB cannot be attached: the whole database is synced on every change", MAX_ATTACHMENT >> 20));
    }
    Ok(())
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

/// Takes files off the entry's current version, leaving them in the
/// database for the versions in history that still have them.
/// keepass-rs cannot do that: it drops a file from the database as soon as
/// the current version lets go of it, even while history refers to it. So the
/// files are removed on a copy of the database and only the entry is taken
/// from it: it refers to the same files as before, minus these.
fn drop_attachments(db: &mut Database, id: EntryId, names: &[String]) {
    if names.is_empty() {
        return;
    }
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

/// How many old versions an entry keeps, and how large they may be together
/// (bytes), as the database says (KeePass's `HistoryMaxItems` and
/// `HistoryMaxSize`, KeePass's own defaults when it does not); `None` for no
/// limit (negative in the file).
pub fn history_limits(meta: &Meta) -> (Option<usize>, Option<usize>) {
    limits_of(stored_limits(meta))
}

/// The limits the stored values mean (see [history_limits]).
fn limits_of((max_items, max_size): (Option<isize>, Option<isize>)) -> (Option<usize>, Option<usize>) {
    let limit = |value: Option<isize>, default| match value {
        Some(n) if n < 0 => None,
        Some(n) => Some(n as usize),
        None => Some(default),
    };
    (limit(max_items, DEFAULT_HISTORY_ITEMS), limit(max_size, DEFAULT_HISTORY_SIZE))
}

/// A version's size as the history limit counts it: its fields, tags and
/// files (a file several versions share counts for each, as in KeePass).
fn version_size(version: &EntryRef<'_>) -> usize {
    let fields: usize = version.fields.iter().map(|(name, value)| name.len() + value.get().len()).sum();
    let tags: usize = version.tags.iter().map(String::len).sum();
    let files: usize = version.attachments_named().map(|(name, a)| name.len() + a.data.get().len()).sum();
    fields + tags + files
}

/// How many old versions the entry's history holds.
pub fn history_count(entry: &Entry) -> usize {
    entry.history.as_ref().map_or(0, |h| h.get_entries().len())
}

/// How many of the entry's old versions (newest first) these limits keep: as
/// many as the count allows, and no more than the size allows together, the
/// oldest going first (as KeePass does).
fn kept_versions(entry: &EntryRef<'_>, (max_items, max_size): (Option<usize>, Option<usize>)) -> usize {
    let limit = history_count(entry).min(max_items.unwrap_or(usize::MAX));
    let Some(max_size) = max_size else { return limit };
    (0..limit)
        .scan(0, |total, i| {
            *total += entry.historical(i).as_ref().map_or(0, version_size);
            Some(*total)
        })
        .take_while(|&total| total <= max_size)
        .count()
}

/// Keeps only the newest old versions the database's limits allow (see
/// [kept_versions]). The files only the dropped versions used stay in the
/// database until [trim_all_history] lets them go.
fn trim_history(db: &mut Database, id: EntryId) {
    let Some(entry) = db.entry(id) else { return };
    let (count, kept) = (history_count(&entry), kept_versions(&entry, history_limits(&db.meta)));
    if kept == count {
        return;
    }
    let Some(mut entry) = db.entry_mut(id) else { return };
    let Some(history) = &entry.history else { return };
    let kept: Vec<_> = history.get_entries().iter().take(kept).cloned().collect();
    let mut trimmed = History::default();
    for old in kept.into_iter().rev() {
        trimmed.add_entry(old); // adds at the front, so oldest first keeps the order
    }
    entry.history = Some(trimmed);
}

/// Trims every entry's history to the database's limits and lets go of the
/// files no version uses any more, so they leave the file too. Every save
/// does this ([crate::dbfile::DbFile::save]).
pub fn trim_all_history(db: &mut Database) {
    for id in all_entries(db) {
        trim_history(db, id);
    }
    db.remove_unused_attachments();
}

/// The history limits as the file stores them (negative: no limit).
fn stored_limits(meta: &Meta) -> (Option<isize>, Option<isize>) {
    (meta.history_max_items, meta.history_max_size)
}

/// Sets the history limits (negative: no limit), and when they changed
/// (KeePass's `SettingsChanged`); saving trims every entry's history to them.
pub fn set_history_limits(db: &mut Database, max_items: isize, max_size: isize) {
    let limits = (Some(max_items), Some(max_size));
    if stored_limits(&db.meta) != limits {
        (db.meta.history_max_items, db.meta.history_max_size) = limits;
        db.meta.settings_changed = Some(Times::now());
    }
}

/// How many old versions these history limits would remove (also when the
/// histories are over the limits already, as another client may leave them).
pub fn versions_over_limits(db: &Database, max_items: isize, max_size: isize) -> usize {
    let limits = limits_of((Some(max_items), Some(max_size)));
    db.iter_all_entries().map(|e| history_count(&e) - kept_versions(&e, limits)).sum()
}

/// Keeps what this device changed that a file read from disk does not have —
/// it came back older (a sync client delivered a stale copy, or a conflict
/// copy won). Every entry of `ours` changed later than in `theirs`, or missing
/// there with no deletion recorded at or after our change, is applied to
/// `theirs` (which keeps its own version in the entry's history); a newer
/// move is kept too. A move to the recycle bin (a deletion) on either side
/// stands unless the other side changed the entry later, and so does a
/// deletion for good this device recorded ([keep_deletions]). Returns the ids
/// of the entries it kept or removed.
pub fn keep_newer(theirs: &mut Database, ours: &Database) -> Vec<EntryId> {
    // Only the recycle bin: a template made here goes among the templates there.
    let hidden: HashSet<GroupId> = theirs.meta.recyclebin_uuid.map(GroupId::from).into_iter().collect();
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
    kept.extend(keep_deletions(theirs, ours));
    keep_templates_group(theirs, ours);
    kept
}

/// A templates' group made here (for a template made here) is one there too:
/// the group at the same place, which keeping the template made.
fn keep_templates_group(theirs: &mut Database, ours: &Database) {
    if theirs.meta.entry_templates_group.is_some_and(|g| theirs.group(GroupId::from(g)).is_some()) {
        return;
    }
    let Some(path) = ours.meta.entry_templates_group.map(GroupId::from).filter(|g| ours.group(*g).is_some()).map(|g| path_of(ours, g)) else {
        return;
    };
    let found = theirs.iter_all_groups().find(|g| g.parent().is_some() && path_of(theirs, g.id()) == path).map(|g| g.id());
    if let Some(group) = found {
        theirs.meta.entry_templates_group = Some(group.uuid());
        theirs.meta.entry_templates_group_changed = ours.meta.entry_templates_group_changed;
    }
}

/// A setting of the database itself, kept in the file (KeePass's `Meta`)
/// with the time it was last changed, so other clients see and change it too.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Setting {
    Name,
    Description,
    DefaultUsername,
}

impl Setting {
    const ALL: [Setting; 3] = [Setting::Name, Setting::Description, Setting::DefaultUsername];

    /// The setting's value in `meta`, and when it was last changed.
    fn in_meta(self, meta: &mut Meta) -> (&mut Option<String>, &mut Option<NaiveDateTime>) {
        match self {
            Setting::Name => (&mut meta.database_name, &mut meta.database_name_changed),
            Setting::Description => (&mut meta.database_description, &mut meta.database_description_changed),
            Setting::DefaultUsername => (&mut meta.default_username, &mut meta.default_username_changed),
        }
    }

    /// Like [Setting::in_meta], to read.
    fn of(self, meta: &Meta) -> (&Option<String>, Option<NaiveDateTime>) {
        match self {
            Setting::Name => (&meta.database_name, meta.database_name_changed),
            Setting::Description => (&meta.database_description, meta.database_description_changed),
            Setting::DefaultUsername => (&meta.default_username, meta.default_username_changed),
        }
    }
}

/// Sets a database setting (trimmed) and when it changed; the value it
/// already has changes nothing.
pub fn set_setting(db: &mut Database, setting: Setting, value: &str) {
    let (field, changed) = setting.in_meta(&mut db.meta);
    let value = value.trim();
    if field.as_deref().unwrap_or_default() != value {
        *field = Some(value.to_string());
        *changed = Some(Times::now());
    }
}

/// How this device takes a copy of the database from elsewhere.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Taking {
    /// The file on this PC, changed by another program: taken as it is now,
    /// keeping only what this device changed later.
    Adopted,
    /// Merged with this device's copy ([merge]): a sync where both changed,
    /// or a copy on an older key.
    Merged,
}

/// Keeps in `theirs` — a copy of the database this device takes, [Taking]
/// how — what stays this device's: its entries ([keep_newer] or [merge]),
/// then the database's own metadata ([keep_our_meta]). True when `theirs` is
/// to be written again: it kept any, or it is on an `older_key`.
pub fn keep_ours(theirs: &mut Database, ours: &Database, taking: Taking, older_key: bool) -> bool {
    // Entries first: they are trimmed to the history limits of the copy as it came.
    let entries = match taking {
        Taking::Adopted => keep_newer(theirs, ours),
        Taking::Merged => merge(theirs, ours),
    };
    let meta = keep_our_meta(theirs, ours, taking, older_key);
    !entries.is_empty() || meta || older_key
}

/// The database's own metadata that stays this device's: each setting `ours`
/// changed later ([keep_newer_settings]); on a copy with an `older_key`, this
/// device's key time; and the encryption, which goes with the key
/// ([crate::encryption::keep_ours]), unless the copy was [Taking::Adopted] on
/// the same key. True when it kept any setting or encryption.
fn keep_our_meta(theirs: &mut Database, ours: &Database, taking: Taking, older_key: bool) -> bool {
    let settings = keep_newer_settings(theirs, ours);
    if older_key {
        theirs.meta.master_key_changed = ours.meta.master_key_changed;
    }
    // After the key time: a copy on an older key now has this device's.
    let encryption = (taking == Taking::Merged || older_key) && crate::encryption::keep_ours(theirs, ours);
    settings || encryption
}

/// Keeps the database settings `ours` changed later than `theirs` did, each
/// by its own time (the history limits by `SettingsChanged`), as KeePass
/// merges them. Returns true when it kept any.
fn keep_newer_settings(theirs: &mut Database, ours: &Database) -> bool {
    let newer_limits =
        ours.meta.settings_changed > theirs.meta.settings_changed && stored_limits(&ours.meta) != stored_limits(&theirs.meta);
    if newer_limits {
        (theirs.meta.history_max_items, theirs.meta.history_max_size) = stored_limits(&ours.meta);
        theirs.meta.settings_changed = ours.meta.settings_changed;
    }
    Setting::ALL.into_iter().fold(newer_limits, |kept, setting| {
        let (value, at) = setting.of(&ours.meta);
        let (their_value, their_at) = setting.in_meta(&mut theirs.meta);
        let newer = at > *their_at && value != their_value;
        if newer {
            (*their_value, *their_at) = (value.clone(), at);
        }
        kept | newer
    })
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

/// Makes an older version (`index` in the history, newest first) the entry's
/// current content: fields, tags, icon, expiry and files. The version it
/// replaces goes into the history, as with an edit; the entry stays in its
/// group. Not for an entry in the trash.
/// `saved` is when the version shown was saved: the file is read again before
/// a change, and another device's edit may have shifted the history since.
pub fn restore_version(db: &mut Database, id: EntryId, index: usize, saved: Option<&str>, hidden: &HashSet<GroupId>) -> Result<(), String> {
    let snapshot = db.clone();
    let entry = snapshot.entry(id).ok_or(NOT_FOUND)?;
    if is_binned(&snapshot, &entry) {
        return Err("An entry in the trash is not changed; restore it first".into());
    }
    let version = entry
        .historical(index)
        .filter(|v| v.times.last_modification.map(time_text).as_deref() == saved)
        .ok_or("The entry's history changed meanwhile; open it again")?;
    let data = read(&version, path_of(&snapshot, entry.parent().id()));
    // As the tracker files it: without its own history.
    let mut before = (*entry).clone();
    before.history = None;
    apply(db, Some(id), &data, hidden)?;
    // When only the files differ, apply changed nothing and filed nothing.
    let filed = db.entry(id).and_then(|e| e.history.as_ref().and_then(|h| h.get_entries().first().cloned())).is_some_and(|newest| newest == before);
    if take_attachments(db, &version) && !filed {
        file_in_history(db, id, before);
        trim_history(db, id);
    }
    Ok(())
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
        retag(db, id, |tags| match (on, tags.contains(&tag)) {
            (true, false) => [tags, std::slice::from_ref(&tag)].concat(),
            (false, true) => tags.iter().filter(|t| **t != tag).cloned().collect(),
            _ => tags.to_vec(), // already as asked, in its place
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
            // Entries without the tag stay exactly as they are.
            if !tags.iter().any(|t| t == from) {
                return tags.to_vec();
            }
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
/// Whether saving `data` over `base` (the entry as the editor opened it; none
/// for a new entry) with file `changes` is sent soon: tags alone wait for the
/// next sync, and an entry saved unchanged has nothing to send.
pub fn needs_upload<C>(base: Option<&EntryData>, data: &EntryData, changes: &[FileChange<C>]) -> bool {
    !changes.is_empty() || base.is_none_or(|base| base != data && !only_tags_changed(base, data))
}

pub fn only_tags_changed(base: &EntryData, data: &EntryData) -> bool {
    let mut same_tags = data.clone();
    same_tags.tags = base.tags.clone();
    base.tags != data.tags && same_tags == *base
}

/// Moves the entry to the recycle bin, creating the bin if the database has
/// none. A database that turned the bin off is refused: entries are removed
/// for good only from the trash.
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

/// The group templates are kept in (KeePass's `EntryTemplatesGroup`), made
/// at the top when the database has none, or only one in the recycle bin.
pub fn templates_group(db: &mut Database) -> GroupId {
    let bin = db.meta.recyclebin_uuid.map(GroupId::from);
    let usable = |g: &GroupId| db.group(*g).is_some() && !bin.is_some_and(|bin| ancestors(db, *g).contains(&bin));
    if let Some(group) = db.meta.entry_templates_group.map(GroupId::from).filter(usable) {
        return group;
    }
    let mut root = db.root_mut();
    let mut group = root.add_group();
    group.name = "Templates".into();
    group.enable_autotype = Some(false);
    group.enable_searching = Some(false);
    let group = group.id();
    db.meta.entry_templates_group = Some(group.uuid());
    db.meta.entry_templates_group_changed = Some(Times::now());
    group
}

/// Moves the entry into the templates' group unless it is in it already.
pub fn put_among_templates(db: &mut Database, id: EntryId) -> Result<(), String> {
    let group = templates_group(db);
    let entry = db.entry(id).ok_or(NOT_FOUND)?;
    if ancestors(db, entry.parent().id()).contains(&group) {
        return Ok(());
    }
    let mut entry = db.entry_mut(id).expect("checked above");
    entry.move_to(group).map_err(|e| e.to_string())?;
    entry.times.location_changed = Some(Times::now());
    Ok(())
}

fn in_bin(db: &Database, id: EntryId) -> bool {
    db.entry(id).is_some_and(|e| is_binned(db, &e))
}

/// Puts entries from the recycle bin back in the group each was deleted from
/// (KDBX 4.1 keeps it; a template goes back among the templates). The top
/// group when that group is gone or itself in the bin, and for an entry in a
/// group deleted into the bin: its previous group is from an older move.
/// Entries not in the bin are left alone.
pub fn restore(db: &mut Database, ids: &[EntryId]) -> Result<(), String> {
    let Some(bin) = db.meta.recyclebin_uuid.map(GroupId::from) else { return Ok(()) };
    for &id in ids {
        if !in_bin(db, id) {
            continue;
        }
        let entry = db.entry(id).expect("in the bin");
        let deleted_itself = entry.parent().id() == bin;
        let back = entry.previous_parent().map(|g| g.id()).filter(|g| deleted_itself && !ancestors(db, *g).contains(&bin));
        let target = back.unwrap_or_else(|| db.root().id());
        let mut entry = db.entry_mut(id).expect("in the bin");
        entry.move_to(target).map_err(|e| e.to_string())?;
        entry.times.location_changed = Some(Times::now());
    }
    Ok(())
}

/// Removes entries in the recycle bin for good, with their history and the
/// files only they used, and records the deletion for other devices
/// (`DeletedObjects`). Entries not in the bin are left alone.
pub fn delete_for_good(db: &mut Database, ids: &[EntryId]) {
    for &id in ids {
        if in_bin(db, id) {
            remove_for_good(db, id);
        }
    }
}

/// Removes the entry, its history and the files only it used (in any
/// version: the fork's fix, offered upstream as sseemayer/keepass-rs#375),
/// recording the deletion.
fn remove_for_good(db: &mut Database, id: EntryId) {
    if let Some(mut entry) = db.entry_mut(id) {
        entry.track_changes().remove();
    }
}

/// Removes a group for good with everything in it, each deletion recorded.
fn remove_group_for_good(db: &mut Database, group: GroupId) {
    let inside: Vec<EntryId> = db.iter_all_entries().filter(|e| ancestors(db, e.parent().id()).contains(&group)).map(|e| e.id()).collect();
    for id in inside {
        remove_for_good(db, id);
    }
    if let Some(mut group) = db.group_mut(group) {
        let _ = group.track_changes().remove(); // refused for the root only, which the callers never pass
    }
}

/// Removes everything in the recycle bin for good: its entries and the
/// groups deleted into it, each deletion recorded for other devices.
pub fn empty_bin(db: &mut Database) {
    let Some(bin) = db.meta.recyclebin_uuid.map(GroupId::from) else { return };
    let Some(group) = db.group(bin) else { return };
    let (entries, groups): (Vec<EntryId>, Vec<GroupId>) = (group.entry_ids().collect(), group.group_ids().collect());
    delete_for_good(db, &entries);
    for group in groups {
        remove_group_for_good(db, group);
    }
}

/// Deletions this device recorded (`ours`: deleting for good in the trash)
/// that `theirs` lacks: the entries and groups go from `theirs` too, unless it
/// changed or moved them later, and the records are kept for other devices.
/// Returns the entries removed.
fn keep_deletions(theirs: &mut Database, ours: &Database) -> Vec<EntryId> {
    let later = |at: Option<NaiveDateTime>, times: &Times| {
        at.is_some_and(|at| times.last_modification.is_some_and(|t| t > at) || times.location_changed.is_some_and(|t| t > at))
    };
    let mut removed = Vec::new();
    for (&uuid, &at) in &ours.deleted_objects {
        let (entry, group) = (EntryId::from(uuid), GroupId::from(uuid));
        if let Some(e) = theirs.entry(entry) {
            if later(at, &e.times) {
                continue;
            }
            remove_for_good(theirs, entry);
            removed.push(entry);
        } else if let Some(g) = theirs.group(group) {
            if later(at, &g.times) || g.parent().is_none() {
                continue;
            }
            removed.extend(theirs.iter_all_entries().filter(|e| ancestors(theirs, e.parent().id()).contains(&group)).map(|e| e.id()));
            remove_group_for_good(theirs, group);
        }
        theirs.deleted_objects.insert(uuid, at);
    }
    removed
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

    /// Changes the entry's files as one edit, as saving from the editor does
    /// (refused in a hidden group, like the editor's saves).
    fn edit_files(db: &mut Database, id: EntryId, changes: &[FileEdit], hidden: &HashSet<GroupId>) -> Result<(), String> {
        let entry = db.entry(id).ok_or(NOT_FOUND)?;
        if ancestors(db, entry.parent().id()).iter().any(|g| hidden.contains(g)) {
            return Err(NOT_FOUND.into());
        }
        let data = read(&entry, path_of(db, entry.parent().id()));
        apply_with_files(db, Some(id), &data, changes, hidden).map(|_| ())
    }

    fn content(data: &[u8]) -> Zeroizing<Vec<u8>> {
        Zeroizing::new(data.to_vec())
    }

    /// Attaches a file; the name it got.
    fn attach(db: &mut Database, id: EntryId, name: &str, data: &[u8], hidden: &HashSet<GroupId>) -> Result<String, String> {
        let given = db.entry(id).map(|e| free_name(&e, name)).unwrap_or_default();
        edit_files(db, id, &[FileChange::Add { name: name.into(), content: content(data) }], hidden)?;
        Ok(given)
    }

    fn detach(db: &mut Database, id: EntryId, name: &str, hidden: &HashSet<GroupId>) -> Result<(), String> {
        edit_files(db, id, &[FileChange::Remove { name: name.into() }], hidden)
    }

    fn rename_attachment(db: &mut Database, id: EntryId, from: &str, to: &str, hidden: &HashSet<GroupId>) -> Result<String, String> {
        edit_files(db, id, &[FileChange::Rename { name: from.into(), to: to.into() }], hidden)?;
        Ok(to.trim().to_string())
    }

    fn history_len(db: &Database, id: EntryId) -> usize {
        history_count(&db.entry(id).unwrap())
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
        // Already so, even not last: nothing changes.
        set_tag(&mut db, &[a], "work", true, &HashSet::new()).unwrap();
        set_tag(&mut db, &[b], "work", false, &HashSet::new()).unwrap();
        assert_eq!((history_len(&db, a), history_len(&db, b)), (1, 1));
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
        // Tags another client wrote twice stay so when another tag is renamed.
        let c = apply(&mut db, None, &with("c", |d| d.tags = vec!["y".into(), "y".into()]), &HashSet::new()).unwrap();
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
    fn restoring_puts_entries_back_where_they_were() {
        let mut db = Database::new();
        let mail = apply(&mut db, None, &with("mail", |d| d.group = vec!["Work".into()]), &HashSet::new()).unwrap();
        let top = apply(&mut db, None, &data("top"), &HashSet::new()).unwrap();
        let gone = apply(&mut db, None, &with("gone", |d| d.group = vec!["Old".into()]), &HashSet::new()).unwrap();
        for id in [mail, top, gone] {
            recycle(&mut db, id).unwrap();
        }
        // The group "gone" was in is deleted meanwhile.
        let old = db.iter_all_groups().find(|g| g.name == "Old").unwrap().id();
        db.group_mut(old).unwrap().track_changes().remove().unwrap();
        restore(&mut db, &[mail, top, gone]).unwrap();
        let group = |db: &Database, id: EntryId| db.entry(id).unwrap().parent().name.clone();
        let root = db.root().name.clone();
        assert_eq!(group(&db, mail), "Work");
        assert_eq!((group(&db, top), group(&db, gone)), (root.clone(), root));
        // Not in the bin: nothing happens.
        restore(&mut db, &[mail]).unwrap();
        assert_eq!(group(&db, mail), "Work");
    }

    #[test]
    fn a_template_restores_among_the_templates_and_an_entry_of_a_deleted_group_at_the_top() {
        let mut db = Database::new();
        let mut root = db.root_mut();
        let mut templates = root.add_group();
        templates.name = "Templates".into();
        let templates = templates.id();
        let card = db.group_mut(templates).unwrap().add_entry().id();
        db.meta.entry_templates_group = Some(templates.uuid());
        recycle(&mut db, card).unwrap();
        // An entry moved from Work to Personal, then Personal deleted into the bin.
        let moved = apply(&mut db, None, &with("moved", |d| d.group = vec!["Work".into()]), &HashSet::new()).unwrap();
        let personal = group_at(&mut db, &["Personal".to_string()], &HashSet::new()).unwrap();
        db.entry_mut(moved).unwrap().move_to(personal).unwrap();
        let bin = GroupId::from(db.meta.recyclebin_uuid.unwrap());
        db.group_mut(personal).unwrap().move_to(bin).unwrap();
        restore(&mut db, &[card, moved]).unwrap();
        assert_eq!(db.entry(card).unwrap().parent().id(), templates);
        assert_eq!(db.entry(moved).unwrap().parent().id(), db.root().id());
    }

    #[test]
    fn files_only_the_deleted_entrys_history_used_go_too() {
        let mut db = Database::new();
        let id = apply(&mut db, None, &data("x"), &HashSet::new()).unwrap();
        attach(&mut db, id, "old.txt", b"secret", &HashSet::new()).unwrap();
        detach(&mut db, id, "old.txt", &HashSet::new()).unwrap(); // history keeps it
        assert_eq!(db.num_attachments(), 1);
        recycle(&mut db, id).unwrap();
        delete_for_good(&mut db, &[id]);
        assert_eq!(db.num_attachments(), 0);
    }

    #[test]
    fn a_deletion_for_good_survives_a_merge_unless_changed_later() {
        let mut ours = Database::new();
        let gone = apply(&mut ours, None, &data("gone"), &HashSet::new()).unwrap();
        let edited = apply(&mut ours, None, &data("edited there"), &HashSet::new()).unwrap();
        recycle(&mut ours, gone).unwrap();
        recycle(&mut ours, edited).unwrap();
        // Another device still has both in its trash.
        let mut theirs = ours.clone();
        let bin = GroupId::from(ours.meta.recyclebin_uuid.unwrap());
        let mut group = ours.group_mut(bin).unwrap();
        let old = group.add_group().id();
        theirs = { let mut t = theirs; t.group_mut(GroupId::from(t.meta.recyclebin_uuid.unwrap())).unwrap().add_group_with_id(old).unwrap(); t };
        delete_for_good(&mut ours, &[gone, edited]);
        empty_bin(&mut ours);
        // ...and changed one of them after this device deleted it.
        theirs.entry_mut(edited).unwrap().times.last_modification = Some(Times::now() + chrono::Duration::seconds(60));

        let removed = merge(&mut theirs, &ours);
        assert!(removed.contains(&gone));
        assert!(theirs.entry(gone).is_none() && theirs.group(old).is_none());
        assert!(theirs.deleted_objects.contains_key(&gone.uuid()) && theirs.deleted_objects.contains_key(&old.uuid()));
        assert!(theirs.entry(edited).is_some());
    }

    #[test]
    fn deleting_for_good_records_it_and_keeps_other_entries_files() {
        let mut db = Database::new();
        let keep = apply(&mut db, None, &data("keep"), &HashSet::new()).unwrap();
        let first = apply(&mut db, None, &data("first"), &HashSet::new()).unwrap();
        let second = apply(&mut db, None, &data("second"), &HashSet::new()).unwrap();
        attach(&mut db, first, "a.txt", b"first's", &HashSet::new()).unwrap();
        attach(&mut db, keep, "b.txt", b"kept", &HashSet::new()).unwrap();
        attach(&mut db, second, "c.txt", b"second's", &HashSet::new()).unwrap();
        recycle(&mut db, first).unwrap();
        recycle(&mut db, second).unwrap();
        // Not in the bin: left alone.
        delete_for_good(&mut db, &[first, keep]);
        assert!(db.entry(first).is_none() && db.entry(keep).is_some());
        assert!(db.deleted_objects.contains_key(&first.uuid()));

        // A group deleted into the bin goes with it when it is emptied.
        let bin = GroupId::from(db.meta.recyclebin_uuid.unwrap());
        let mut group = db.group_mut(bin).unwrap();
        let mut sub = group.add_group();
        sub.name = "Old".into();
        let sub = sub.id();
        empty_bin(&mut db);
        assert!(db.entry(second).is_none() && db.group(sub).is_none());
        assert!(db.deleted_objects.contains_key(&sub.uuid()));
        assert_eq!(db.group(bin).unwrap().entry_ids().count(), 0);

        // The file keeps the other entry's file as it was.
        let key = crate::dbfile::tests::key();
        let mut bytes = Vec::new();
        db.save(&mut bytes, key.clone()).unwrap();
        let reopened = Database::parse(&bytes, key).unwrap();
        assert_eq!(files(&reopened, keep), [("b.txt".to_string(), b"kept".to_vec())]);
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
    fn a_setting_is_trimmed_and_timed_and_the_same_value_changes_nothing() {
        let mut db = Database::new();
        set_setting(&mut db, Setting::Name, "  Home  ");
        assert_eq!(db.meta.database_name.as_deref(), Some("Home"));
        assert!(db.meta.database_name_changed.is_some());
        let before = db.clone();
        set_setting(&mut db, Setting::Name, "Home");
        assert_eq!(db, before);
    }

    /// An entry edited `versions` times, each old version holding `notes`.
    fn edited(db: &mut Database, versions: usize, notes: &str) -> EntryId {
        let id = apply(db, None, &data("Mail"), &HashSet::new()).unwrap();
        for i in 0..versions {
            apply(db, Some(id), &with("Mail", |d| d.notes = format!("{i} {notes}")), &HashSet::new()).unwrap();
        }
        id
    }

    #[test]
    fn history_keeps_the_newest_versions_within_the_size_limit() {
        let mut db = Database::new();
        db.meta.history_max_size = Some(3000);
        let id = edited(&mut db, 5, &"x".repeat(1000));
        // Each version is about 1 KB: the newest two fit, a third would not.
        assert_eq!(history_len(&db, id), 2);
        let newest = db.entry(id).unwrap().historical(0).unwrap().get("Notes").unwrap().to_string();
        assert!(newest.starts_with("3 "), "{newest}");
    }

    #[test]
    fn lowering_a_limit_trims_every_entry_and_says_how_many_go_first() {
        let mut db = Database::new();
        let (a, b) = (edited(&mut db, 4, "a"), edited(&mut db, 2, "b"));
        assert_eq!(versions_over_limits(&db, 1, -1), 3 + 1);
        assert_eq!(versions_over_limits(&db, 10, -1), 0);
        assert_eq!((history_len(&db, a), history_len(&db, b)), (4, 2), "a preview changes nothing");

        set_history_limits(&mut db, 1, -1);
        trim_all_history(&mut db); // as saving does
        assert_eq!((history_len(&db, a), history_len(&db, b)), (1, 1));
        assert_eq!(history_limits(&db.meta), (Some(1), None));
        assert!(db.meta.settings_changed.is_some());
    }

    #[test]
    fn history_limits_default_to_keepass_and_negative_means_none() {
        let mut meta = Meta::default();
        assert_eq!(history_limits(&meta), (Some(10), Some(6 << 20)));
        (meta.history_max_items, meta.history_max_size) = (Some(-1), Some(-1));
        assert_eq!(history_limits(&meta), (None, None));
    }

    #[test]
    fn the_preview_counts_versions_already_over_the_limits() {
        let mut db = Database::new();
        edited(&mut db, 4, "a");
        // Another client lowered the limit without trimming.
        db.meta.history_max_items = Some(1);
        assert_eq!(versions_over_limits(&db, 1, -1), 3);
    }

    #[test]
    fn keep_newer_settings_takes_the_newer_history_limits() {
        let mut theirs = Database::new();
        let mut ours = theirs.clone();
        set_history_limits(&mut ours, 3, 1 << 20);
        assert!(keep_newer_settings(&mut theirs, &ours));
        assert_eq!(history_limits(&theirs.meta), (Some(3), Some(1 << 20)));
        // Changed there later: theirs stay.
        set_history_limits(&mut theirs, 5, -1);
        theirs.meta.settings_changed = Some(Times::now() + chrono::Duration::seconds(5));
        assert!(!keep_newer_settings(&mut theirs, &ours));
        assert_eq!(history_limits(&theirs.meta), (Some(5), None));
    }

    #[test]
    fn a_file_only_trimmed_versions_used_leaves_the_database() {
        let mut db = Database::new();
        let id = apply(&mut db, None, &data("Mail"), &HashSet::new()).unwrap();
        attach(&mut db, id, "old.txt", b"old file", &HashSet::new()).unwrap();
        let replace = [FileChange::Remove { name: "old.txt".into() }, FileChange::Add { name: "old.txt".into(), content: content(b"new file") }];
        edit_files(&mut db, id, &replace, &HashSet::new()).unwrap();
        assert_eq!(db.num_attachments(), 2, "history keeps the old content");

        set_history_limits(&mut db, 0, -1);
        trim_all_history(&mut db); // as saving does
        assert_eq!(history_len(&db, id), 0);
        assert_eq!(db.num_attachments(), 1);
        let files: Vec<_> = db.entry(id).unwrap().attachments().map(|a| a.data.get().clone()).collect();
        assert_eq!(files, [b"new file".to_vec()]);
    }

    #[test]
    fn keep_newer_settings_takes_each_newer_one() {
        let mut theirs = Database::new();
        set_setting(&mut theirs, Setting::Name, "Named there");
        set_setting(&mut theirs, Setting::Description, "Described there");
        let mut ours = theirs.clone();
        set_setting(&mut ours, Setting::Name, "Named here");
        set_setting(&mut ours, Setting::Description, "Described here");
        set_setting(&mut ours, Setting::DefaultUsername, "me");
        // Times have one-second precision: set which side is newer by hand.
        let (earlier, later) = (Some(Times::epoch()), Some(Times::now()));
        (ours.meta.database_name_changed, theirs.meta.database_name_changed) = (later, earlier);
        (ours.meta.database_description_changed, theirs.meta.database_description_changed) = (earlier, later);

        assert!(keep_newer_settings(&mut theirs, &ours));
        assert_eq!(theirs.meta.database_name.as_deref(), Some("Named here"));
        assert_eq!(theirs.meta.database_description.as_deref(), Some("Described there"));
        // A setting the other side never had: this device's.
        assert_eq!(theirs.meta.default_username.as_deref(), Some("me"));
        assert!(!keep_newer_settings(&mut theirs, &ours));
    }

    #[test]
    fn keep_newer_keeps_a_template_made_here() {
        let mut theirs = Database::new();
        apply(&mut theirs, None, &data("entry"), &HashSet::new()).unwrap();
        let mut ours = theirs.clone();
        let card = apply(&mut ours, None, &data("Card"), &HashSet::new()).unwrap();
        put_among_templates(&mut ours, card).unwrap();
        let templates = GroupId::from(ours.meta.entry_templates_group.unwrap());

        keep_newer(&mut theirs, &ours);
        let there = theirs.entry(card).unwrap();
        let group = GroupId::from(theirs.meta.entry_templates_group.expect("carried over"));
        assert!(ancestors(&theirs, there.parent().id()).contains(&group));
        assert_eq!(path_of(&theirs, group), path_of(&ours, templates));
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
        // Refused: a name in use, no name. A file that is gone (removed
        // elsewhere) and the same name change nothing.
        let before = db.clone();
        assert!(rename(&mut db, "c.txt", "b.txt").is_err());
        assert!(rename(&mut db, "c.txt", " ").is_err());
        rename(&mut db, "a.txt", "d.txt").unwrap();
        assert_eq!(rename(&mut db, "c.txt", "c.txt").unwrap(), "c.txt");
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
    fn fields_and_files_saved_together_are_one_version() {
        let mut db = Database::new();
        let id = attach_to_new(&mut db, &[("a.txt", b"a"), ("b.txt", b"b"), ("c.txt", b"c")]);
        let versions = history_len(&db, id);
        let edited = with("x", |d| d.password = "new".into());
        let changes = [
            FileChange::Remove { name: "a.txt".into() },
            FileChange::Rename { name: "b.txt".into(), to: "a.txt".into() },
            FileChange::Remove { name: "c.txt".into() },
            FileChange::Add { name: "c.txt".into(), content: content(b"c2") },
            FileChange::Add { name: "c.txt".into(), content: content(b"another") },
        ];
        apply_with_files(&mut db, Some(id), &edited, &changes, &HashSet::new()).unwrap();
        assert_eq!(history_len(&db, id), versions + 1);
        assert_eq!(files(&db, id), [
            ("a.txt".to_string(), b"b".to_vec()),
            ("c (2).txt".to_string(), b"another".to_vec()),
            ("c.txt".to_string(), b"c2".to_vec()),
        ]);
        {
            let entry = db.entry(id).unwrap();
            let previous = entry.historical(0).unwrap();
            assert_eq!(previous.attachments().count(), 3);
            assert_ne!(previous.get_password(), Some("new"));
        }
        // Files alone are one version too; nothing to change adds none.
        apply_with_files(&mut db, Some(id), &edited, &[FileChange::Remove { name: "a.txt".into() }], &HashSet::new()).unwrap();
        assert_eq!(history_len(&db, id), versions + 2);
        apply_with_files(&mut db, Some(id), &edited, &[FileChange::Remove { name: "a.txt".into() }], &HashSet::new()).unwrap();
        assert_eq!(history_len(&db, id), versions + 2);
        // A new entry gets its files with it, without history.
        let new = apply_with_files(&mut db, None, &data("new"), &[FileChange::Add { name: "n.txt".into(), content: content(b"n") }], &HashSet::new()).unwrap();
        assert_eq!(files(&db, new), [("n.txt".to_string(), b"n".to_vec())]);
        assert_eq!(history_len(&db, new), 0);
    }

    /// A new entry `x` with `files`, attached one by one.
    fn attach_to_new(db: &mut Database, files: &[(&str, &[u8])]) -> EntryId {
        let id = apply(db, None, &data("x"), &HashSet::new()).unwrap();
        for (name, data) in files {
            attach(db, id, name, data, &HashSet::new()).unwrap();
        }
        id
    }

    #[test]
    fn renames_are_made_together() {
        let mut db = Database::new();
        let id = attach_to_new(&mut db, &[("a.txt", b"a"), ("b.txt", b"b"), ("c.txt", b"c")]);
        let rename = |name: &str, to: &str| FileChange::Rename { name: name.into(), to: to.into() };
        // A chain listed in the "wrong" order, and a swap.
        edit_files(&mut db, id, &[rename("a.txt", "c.txt"), rename("c.txt", "d.txt")], &HashSet::new()).unwrap();
        edit_files(&mut db, id, &[rename("b.txt", "c.txt"), rename("c.txt", "b.txt")], &HashSet::new()).unwrap();
        assert_eq!(files(&db, id), [
            ("b.txt".to_string(), b"a".to_vec()),
            ("c.txt".to_string(), b"b".to_vec()),
            ("d.txt".to_string(), b"c".to_vec()),
        ]);
        // Two files to one name, or onto a file that stays: refused.
        assert!(edit_files(&mut db, id, &[rename("b.txt", "x.txt"), rename("c.txt", "x.txt")], &HashSet::new()).is_err());
        assert!(edit_files(&mut db, id, &[rename("b.txt", "d.txt")], &HashSet::new()).is_err());
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
    fn only_a_new_entry_or_a_change_beyond_tags_goes_up_soon() {
        let base = data("x");
        let none: [FileChange; 0] = [];
        assert!(needs_upload(None, &base, &none));
        assert!(needs_upload(Some(&base), &with("x", |d| d.password = "new".into()), &none));
        assert!(!needs_upload(Some(&base), &with("x", |d| d.tags = vec!["a".into()]), &none));
        assert!(!needs_upload(Some(&base), &base.clone(), &none));
        assert!(needs_upload(Some(&base), &base.clone(), &[FileChange::<u64>::Remove { name: "a".into() }]));
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
