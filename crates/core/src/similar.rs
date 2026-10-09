//! Find similar: entries for the same site (their URLs share a registrable
//! domain, or are the same address), and merging such entries into one.

use crate::edit::{self, passkey, EntryData, FieldData, FileChange, FileEdit, NOT_FOUND};
use crate::{icons, otp};
use keepass::db::{EntryId, EntryRef, GroupId};
use keepass::Database;
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use zeroize::Zeroizing;

/// Entries for one site.
#[derive(Debug, PartialEq, Serialize)]
pub struct Similar {
    /// The registrable domain (`google.com`), or the address itself when it
    /// is not a site's (`androidapp://…`, an IP address).
    pub site: String,
    /// The most recently changed first.
    pub ids: Vec<String>,
}

/// What entries are grouped by: the registrable domain of a site's address
/// (mail.google.com and accounts.google.com are both `google.com`), the IP
/// address of a web address on one, else the address itself; `None` for an
/// entry without a URL.
pub fn site_of(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    // A site's address, or a web address on an IP address (web_url takes named hosts only).
    let web = icons::web_url(url).or_else(|| url::Url::parse(url).ok().filter(|u| matches!(u.scheme(), "http" | "https")));
    Some(match web.as_ref().and_then(url::Url::host) {
        Some(url::Host::Domain(host)) if host.contains('.') => {
            let host = host.trim_end_matches('.');
            psl::domain_str(host).unwrap_or(host).to_string()
        }
        Some(ip @ (url::Host::Ipv4(_) | url::Host::Ipv6(_))) => ip.to_string(),
        _ => url.trim_end_matches('/').to_string(),
    })
}

/// The sites more than one of `entries` is for, by name.
pub fn find<'a>(entries: impl Iterator<Item = EntryRef<'a>>) -> Vec<Similar> {
    let mut sites: BTreeMap<String, Vec<EntryRef<'a>>> = BTreeMap::new();
    for entry in entries {
        if let Some(site) = entry.get(keepass::db::fields::URL).and_then(site_of) {
            sites.entry(site).or_default().push(entry);
        }
    }
    sites
        .into_iter()
        .filter(|(_, entries)| entries.len() > 1)
        .map(|(site, mut entries)| {
            entries.sort_by_key(|e| std::cmp::Reverse(e.times.last_modification));
            Similar { site, ids: entries.iter().map(|e| e.id().uuid().to_string()).collect() }
        })
        .collect()
}

/// Merges `others` into `keep`: its empty fields are filled from them, a
/// value that differs from its own becomes an additional field named after
/// the entry it came from, tags are joined and files carried over (the same
/// content once). Then `others` go to the recycle bin. One edit of `keep`:
/// its previous version goes into its history. An entry gone meanwhile is
/// left out.
pub fn merge(db: &mut Database, keep: EntryId, others: &[EntryId], hidden: &HashSet<GroupId>) -> Result<(), String> {
    let kept = db.entry(keep).ok_or(NOT_FOUND)?;
    let mut data = edit::read(&kept, edit::path_of(db, kept.parent().id()));
    let mut contents: Vec<Zeroizing<Vec<u8>>> = kept.attachments_named().map(|(_, a)| Zeroizing::new(a.data.get().clone())).collect();
    let mut files: Vec<FileEdit> = Vec::new();
    let others: Vec<EntryId> = others.iter().copied().filter(|&id| id != keep && db.entry(id).is_some()).collect();
    for &id in &others {
        let other = db.entry(id).expect("checked above");
        take(&mut data, &edit::read(&other, Vec::new()));
        for (name, file) in other.attachments_named() {
            let content = Zeroizing::new(file.data.get().clone());
            if !contents.contains(&content) {
                contents.push(content.clone());
                files.push(FileChange::Add { name: name.to_string(), content });
            }
        }
    }
    edit::apply_with_files(db, Some(keep), &data, &files, hidden)?;
    for id in others {
        edit::recycle(db, id)?;
    }
    Ok(())
}

/// What a merge would change in the kept entry, for the user to see first.
#[derive(Debug, Default, PartialEq, Serialize)]
pub struct Preview {
    /// Its empty fields that are filled, then the additional fields it gets.
    pub fields: Vec<PreviewField>,
    pub tags: Vec<String>,
    /// The files it gets, by the names they will have.
    pub files: Vec<String>,
    /// It gets another's icon.
    pub icon: bool,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct PreviewField {
    pub name: String,
    /// `None` for a protected value (a password, a TOTP secret): it stays here.
    pub value: Option<String>,
    /// One of its own fields, empty until now.
    pub fills: bool,
}

/// What [merge] would change in `keep`: worked out by merging on a copy.
pub fn preview(db: &Database, keep: EntryId, others: &[EntryId], hidden: &HashSet<GroupId>) -> Result<Preview, String> {
    let read = |db: &Database| db.entry(keep).map(|e| (edit::read(&e, Vec::new()), e.attachments_named().map(|(n, _)| n.to_string()).collect::<Vec<_>>()));
    let (mut before, files_before) = read(db).ok_or(NOT_FOUND)?;
    let mut merged = db.clone();
    merge(&mut merged, keep, others, hidden)?;
    let (mut after, files_after) = read(&merged).ok_or(NOT_FOUND)?;

    let mut preview = Preview::default();
    for ((name, was, protected), (_, is, _)) in standard(&mut before).into_iter().zip(standard(&mut after)) {
        if was != is {
            preview.fields.push(PreviewField { name: name.to_string(), value: (!protected).then(|| is.clone()), fills: true });
        }
    }
    for field in after.fields.iter().filter(|f| !before.fields.iter().any(|b| b.name == f.name)) {
        preview.fields.push(PreviewField { name: field.name.clone(), value: (!field.protected).then(|| field.value.clone()), fills: false });
    }
    preview.tags = after.tags.iter().filter(|t| !before.tags.contains(t)).cloned().collect();
    preview.files = files_after.into_iter().filter(|f| !files_before.contains(f)).collect();
    preview.icon = before.icon != after.icon;
    Ok(preview)
}

/// Adds what `from` has to `into` (see [merge]).
fn take(into: &mut EntryData, from: &EntryData) {
    let source = match from.title.trim() {
        "" => "another entry",
        title => title,
    };
    // Values that differ from `into`'s own, with the name they go under.
    let mut differing: Vec<FieldData> = Vec::new();
    // Other addresses: `into`'s further URLs.
    let mut urls: Vec<&String> = Vec::new();
    // A TOTP secret this app cannot read is kept, but not as the entry's.
    let otp_works = otp::normalize(&from.otp, &into.title).is_ok();
    let theirs = [&from.username, &from.password, &from.url, &from.notes, &from.otp];
    for ((label, ours, protected), theirs) in standard(into).into_iter().zip(theirs) {
        if theirs.is_empty() || ours == theirs {
            continue;
        }
        let fills = label != "TOTP" || otp_works;
        if ours.is_empty() && fills {
            *ours = theirs.clone();
        } else if label == "URL" {
            urls.push(theirs);
        } else {
            differing.push(FieldData { name: label.to_string(), value: theirs.clone(), protected });
        }
    }
    // A passkey is its fields together: one does not fill in another's.
    let has_passkey = |data: &EntryData| data.fields.iter().any(|f| f.name.starts_with(passkey::PREFIX));
    let both_passkeys = has_passkey(into) && has_passkey(from);
    for field in &from.fields {
        if field.name.starts_with(MORE_URLS) {
            urls.push(&field.value);
            continue;
        }
        match into.fields.iter().find(|f| f.name == field.name) {
            Some(ours) if ours.value == field.value => {}
            None if !(both_passkeys && field.name.starts_with(passkey::PREFIX)) => into.fields.push(field.clone()),
            _ => differing.push(field.clone()),
        }
    }
    for field in differing {
        add_differing(into, field, source);
    }
    for url in urls {
        add_url(into, url);
    }
    for tag in &from.tags {
        if !into.tags.contains(tag) {
            into.tags.push(tag.clone());
        }
    }
    if into.icon == edit::IconChoice::Auto {
        into.icon = from.icon.clone();
    }
}

/// Further URLs of an entry, as Keepass2Android and KeePassXC keep them:
/// `KP2A_URL_1`, `KP2A_URL_2`…
const MORE_URLS: &str = "KP2A_URL";

/// `url` as the next of `into`'s further URLs, unless it has it already.
fn add_url(into: &mut EntryData, url: &str) {
    let has = |f: &FieldData| f.name.starts_with(MORE_URLS) && f.value == url;
    if into.url == url || into.fields.iter().any(has) {
        return;
    }
    let name = (1..).map(|n| format!("{MORE_URLS}_{n}")).find(|name| !into.fields.iter().any(|f| f.name == *name)).expect("a free name");
    into.fields.push(FieldData { name, value: url.to_string(), protected: false });
}

/// The fields the editor has its own inputs for, but the title: their names
/// as an additional field takes them, the values, and whether they are protected.
fn standard(data: &mut EntryData) -> [(&'static str, &mut String, bool); 5] {
    [
        ("User name", &mut data.username, false),
        ("Password", &mut data.password, true),
        ("URL", &mut data.url, false),
        ("Notes", &mut data.notes, false),
        ("TOTP", &mut data.otp, true),
    ]
}

/// `field`, a value `into` has another one for, as `<name> (<source>)`
/// (numbered when taken); not when `into` already has the value in a field.
fn add_differing(into: &mut EntryData, mut field: FieldData, source: &str) {
    let standard = [&into.username, &into.password, &into.url, &into.notes, &into.otp];
    if standard.contains(&&field.value) || into.fields.iter().any(|f| f.value == field.value) {
        return;
    }
    let label = std::mem::take(&mut field.name);
    let taken = |name: &str| into.fields.iter().any(|f| f.name == name);
    field.name = std::iter::once(format!("{label} ({source})"))
        .chain((2..).map(|n| format!("{label} ({source} {n})")))
        .find(|name| !taken(name))
        .expect("a free name");
    into.fields.push(field);
}

#[cfg(test)]
mod tests {
    use super::*;
    use keepass::db::{fields, Value};

    fn add(db: &mut Database, title: &str, url: &str, set: impl FnOnce(&mut keepass::db::EntryMut<'_>)) -> EntryId {
        let mut root = db.root_mut();
        let mut entry = root.add_entry();
        entry.set_unprotected(fields::TITLE, title);
        entry.set_unprotected(fields::URL, url);
        set(&mut entry);
        entry.id()
    }

    fn field(db: &Database, id: EntryId, name: &str) -> Option<String> {
        db.entry(id).unwrap().fields.get(name).map(|v| v.get().clone())
    }

    #[test]
    fn sites_are_registrable_domains_or_the_address() {
        assert_eq!(site_of("https://mail.google.com/mail/u/0").as_deref(), Some("google.com"));
        assert_eq!(site_of("accounts.google.com").as_deref(), Some("google.com"));
        assert_eq!(site_of("https://www.bbc.co.uk/").as_deref(), Some("bbc.co.uk"));
        assert_eq!(site_of("http://router.lan").as_deref(), Some("router.lan"));
        assert_eq!(site_of("http://192.168.1.1/").as_deref(), Some("192.168.1.1"));
        assert_eq!(site_of("https://192.168.1.1/admin").as_deref(), Some("192.168.1.1"));
        assert_eq!(site_of("androidapp://com.example").as_deref(), Some("androidapp://com.example"));
        assert_eq!(site_of("  "), None);
    }

    #[test]
    fn finds_entries_for_the_same_site() {
        let mut db = Database::new();
        add(&mut db, "Gmail", "https://mail.google.com", |_| {});
        add(&mut db, "Google", "accounts.google.com", |_| {});
        add(&mut db, "GitHub", "https://github.com", |_| {});
        add(&mut db, "No URL", "", |_| {});
        add(&mut db, "No URL 2", "", |_| {});
        let found = find(db.iter_all_entries());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].site, "google.com");
        assert_eq!(found[0].ids.len(), 2);
    }

    #[test]
    fn merge_fills_empty_fields_and_keeps_differing_values() {
        let mut db = Database::new();
        let keep = add(&mut db, "Google", "https://accounts.google.com", |e| {
            e.set_unprotected(fields::USERNAME, "me@gmail.com");
            e.set_protected(fields::PASSWORD, "new-pass");
            e.tags = vec!["Work".into()];
        });
        let other = add(&mut db, "Gmail", "https://mail.google.com", |e| {
            e.set_unprotected(fields::USERNAME, "me@gmail.com");
            e.set_protected(fields::PASSWORD, "old-pass");
            e.set_unprotected(fields::NOTES, "recovery codes in the safe");
            e.set_unprotected("Recovery email", "me@example.com");
            e.tags = vec!["Work".into(), "Mail".into()];
            e.add_attachment("codes.txt", Value::protected(b"1234".to_vec()));
        });
        let hidden = edit::hidden_groups(&db);
        merge(&mut db, keep, &[other], &hidden).unwrap();

        assert_eq!(field(&db, keep, fields::PASSWORD).as_deref(), Some("new-pass"));
        assert_eq!(field(&db, keep, "Password (Gmail)").as_deref(), Some("old-pass"));
        assert!(db.entry(keep).unwrap().fields["Password (Gmail)"].is_protected());
        assert_eq!(field(&db, keep, "KP2A_URL_1").as_deref(), Some("https://mail.google.com"));
        assert_eq!(field(&db, keep, fields::NOTES).as_deref(), Some("recovery codes in the safe"));
        assert_eq!(field(&db, keep, "Recovery email").as_deref(), Some("me@example.com"));
        assert_eq!(field(&db, keep, "User name (Gmail)"), None, "the same value is not added");
        let kept = db.entry(keep).unwrap();
        assert_eq!(kept.tags, ["Work", "Mail"]);
        assert_eq!(kept.attachment_by_name("codes.txt").unwrap().data.get(), b"1234");
        assert_eq!(edit::history_count(&kept), 1, "one edit");
        let bin = db.recycle_bin().unwrap().id();
        assert_eq!(db.entry(other).unwrap().parent().id(), bin);
    }

    #[test]
    fn merge_names_differing_values_apart_and_adds_each_once() {
        let mut db = Database::new();
        let keep = add(&mut db, "Shop", "shop.com", |e| e.set_protected(fields::PASSWORD, "one"));
        let a = add(&mut db, "Shop", "shop.com", |e| e.set_protected(fields::PASSWORD, "two"));
        let b = add(&mut db, "Shop", "shop.com", |e| e.set_protected(fields::PASSWORD, "three"));
        let c = add(&mut db, "Shop", "shop.com", |e| e.set_protected(fields::PASSWORD, "two"));
        let hidden = edit::hidden_groups(&db);
        merge(&mut db, keep, &[a, b, c], &hidden).unwrap();
        assert_eq!(field(&db, keep, "Password (Shop)").as_deref(), Some("two"));
        assert_eq!(field(&db, keep, "Password (Shop 2)").as_deref(), Some("three"));
        assert_eq!(field(&db, keep, "Password (Shop 3)"), None);
    }

    #[test]
    fn a_value_the_kept_entry_has_under_another_name_is_not_added() {
        let mut db = Database::new();
        let keep = add(&mut db, "Mail", "mail.com", |e| {
            e.set_unprotected(fields::NOTES, "old notes");
            e.set_unprotected("Recovery email", "me@example.com");
        });
        let other = add(&mut db, "Mail", "mail.com", |e| e.set_unprotected(fields::NOTES, "me@example.com"));
        let hidden = edit::hidden_groups(&db);
        merge(&mut db, keep, &[other], &hidden).unwrap();
        let names: Vec<String> = db.entry(keep).unwrap().fields.keys().cloned().collect();
        assert!(!names.iter().any(|f| f.starts_with("Notes (")), "{names:?}");
    }

    #[test]
    fn merge_carries_files_over_once_and_numbers_taken_names() {
        let mut db = Database::new();
        let keep = add(&mut db, "Bank", "bank.com", |e| {
            e.add_attachment("scan.pdf", Value::protected(b"mine".to_vec()));
        });
        let other = add(&mut db, "Bank", "bank.com", |e| {
            e.add_attachment("scan.pdf", Value::protected(b"theirs".to_vec()));
            e.add_attachment("copy.pdf", Value::protected(b"mine".to_vec()));
        });
        let hidden = edit::hidden_groups(&db);
        merge(&mut db, keep, &[other], &hidden).unwrap();
        let kept = db.entry(keep).unwrap();
        let mut names: Vec<&str> = kept.attachments_named().map(|(name, _)| name).collect();
        names.sort();
        assert_eq!(names, ["scan (2).pdf", "scan.pdf"]);
        assert_eq!(kept.attachment_by_name("scan (2).pdf").unwrap().data.get(), b"theirs");
    }

    #[test]
    fn preview_shows_what_a_merge_adds_without_protected_values() {
        let mut db = Database::new();
        let keep = add(&mut db, "Google", "https://accounts.google.com", |e| e.set_protected(fields::PASSWORD, "new-pass"));
        let other = add(&mut db, "Gmail", "https://mail.google.com", |e| {
            e.set_unprotected(fields::USERNAME, "me@gmail.com");
            e.set_protected(fields::PASSWORD, "old-pass");
            e.tags = vec!["Mail".into()];
            e.add_attachment("codes.txt", Value::protected(b"1234".to_vec()));
        });
        let hidden = edit::hidden_groups(&db);
        let before = db.clone();
        let preview = preview(&db, keep, &[other], &hidden).unwrap();
        assert_eq!(db, before, "nothing changes");
        let shown: Vec<(&str, Option<&str>, bool)> = preview.fields.iter().map(|f| (f.name.as_str(), f.value.as_deref(), f.fills)).collect();
        assert_eq!(shown, [
            ("User name", Some("me@gmail.com"), true),
            ("KP2A_URL_1", Some("https://mail.google.com"), false),
            ("Password (Gmail)", None, false),
        ]);
        assert_eq!(preview.tags, ["Mail"]);
        assert_eq!(preview.files, ["codes.txt"]);
    }

    #[test]
    fn other_addresses_become_further_urls() {
        let mut db = Database::new();
        let keep = add(&mut db, "Shop", "https://shop.com", |e| e.set_unprotected("KP2A_URL_1", "https://m.shop.com"));
        let a = add(&mut db, "Shop EU", "https://eu.shop.com", |e| e.set_unprotected("KP2A_URL_1", "https://m.shop.com"));
        let b = add(&mut db, "Shop app", "https://shop.com", |e| e.set_unprotected("KP2A_URL_1", "androidapp://com.shop"));
        let hidden = edit::hidden_groups(&db);
        merge(&mut db, keep, &[a, b], &hidden).unwrap();
        assert_eq!(field(&db, keep, "KP2A_URL_1").as_deref(), Some("https://m.shop.com"));
        assert_eq!(field(&db, keep, "KP2A_URL_2").as_deref(), Some("https://eu.shop.com"));
        assert_eq!(field(&db, keep, "KP2A_URL_3").as_deref(), Some("androidapp://com.shop"));
        assert_eq!(field(&db, keep, "KP2A_URL_4"), None);
    }

    #[test]
    fn a_passkey_does_not_fill_in_another() {
        let mut db = Database::new();
        let keep = add(&mut db, "Google", "google.com", |e| {
            e.set_protected(passkey::PRIVATE_KEY, "key-a");
        });
        let login = add(&mut db, "Google login", "google.com", |e| {
            e.set_protected(fields::PASSWORD, "secret");
        });
        let other = add(&mut db, "Google passkey", "google.com", |e| {
            e.set_protected(passkey::PRIVATE_KEY, "key-b");
            e.set_unprotected(passkey::USER_HANDLE, "handle-b");
        });
        let hidden = edit::hidden_groups(&db);
        merge(&mut db, keep, &[login, other], &hidden).unwrap();
        assert_eq!(field(&db, keep, fields::PASSWORD).as_deref(), Some("secret"));
        assert_eq!(field(&db, keep, passkey::PRIVATE_KEY).as_deref(), Some("key-a"));
        assert_eq!(field(&db, keep, passkey::USER_HANDLE), None);
        assert_eq!(field(&db, keep, &format!("{} (Google passkey)", passkey::USER_HANDLE)).as_deref(), Some("handle-b"));
    }
}
