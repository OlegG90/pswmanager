//! Find similar: entries alike in the criteria chosen (#267) — for the same
//! site (any of their URLs shares a registrable domain and port, or is the
//! same address), the same account (user name) and / or the same title — and
//! merging such entries into one.

use crate::edit::{self, passkey, EntryData, FieldData, FileChange, FileEdit, NOT_FOUND};
use crate::{icons, otp};
use keepass::db::{EntryId, EntryRef, GroupId};
use keepass::Database;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use zeroize::Zeroizing;

/// What makes entries similar (#267): they are alike in each criterion that
/// is on. Without any, nothing is similar.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Criteria {
    pub title: bool,
    pub user_name: bool,
    pub url: bool,
}

impl Default for Criteria {
    /// The same site and user name, as Find similar looked before it had a choice.
    fn default() -> Self {
        Criteria { title: false, user_name: true, url: true }
    }
}

/// Entries alike in the criteria chosen.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Similar {
    /// The site most of them share: the registrable domain (`google.com`,
    /// with a port when one is given), or the address itself when it is not
    /// a site's (`androidapp://…`, an IP address); empty when the URL is not
    /// a criterion.
    pub site: String,
    /// The title they share, as the most recently changed of them has it;
    /// empty when the title is not a criterion.
    pub title: String,
    /// The user name the entries share, as the most recently changed of them
    /// has it; empty when none has one or it is not a criterion.
    pub user_name: String,
    /// The most recently changed first.
    pub ids: Vec<String>,
}

/// What entries are grouped by: the registrable domain of a site's address
/// (mail.google.com and accounts.google.com are both `google.com`), the IP
/// address of a web address on one, else the address itself; `None` for an
/// empty URL. A port given in the address is part of it: one host serves
/// different things on different ports (#267).
pub fn site_of(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    let web = icons::link(url);
    let port = web.as_ref().and_then(url::Url::port).map(|port| format!(":{port}")).unwrap_or_default();
    Some(match web.as_ref().and_then(url::Url::host) {
        Some(url::Host::Domain(host)) if host.contains('.') => {
            let host = host.trim_end_matches('.');
            format!("{}{port}", psl::domain_str(host).unwrap_or(host))
        }
        Some(ip @ (url::Host::Ipv4(_) | url::Host::Ipv6(_))) => format!("{ip}{port}"),
        _ => url.trim_end_matches('/').to_string(),
    })
}

/// The sites of an entry's URL and of its further URLs (`KP2A_URL_n`), once each.
fn sites_of(entry: &EntryRef<'_>) -> Vec<String> {
    let mut sites: Vec<String> = entry
        .fields
        .iter()
        .filter(|(name, _)| name.as_str() == keepass::db::fields::URL || name.starts_with(MORE_URLS))
        .filter_map(|(_, value)| site_of(value.get()))
        .collect();
    sites.sort();
    sites.dedup();
    sites
}

/// The entries of `entries` alike in `criteria`, more than one each. By user
/// name (#227), entries with different ones are different accounts, never
/// offered to merge.
pub fn find<'a>(entries: impl Iterator<Item = EntryRef<'a>>, criteria: Criteria) -> Vec<Similar> {
    if !(criteria.title || criteria.user_name || criteria.url) {
        return Vec::new();
    }
    // By title first: one bucket when it does not count. An entry without
    // what a criterion compares is like no other.
    let mut by_title: BTreeMap<String, Vec<(EntryRef<'a>, Vec<String>)>> = BTreeMap::new();
    for entry in entries {
        let sites = if criteria.url { sites_of(&entry) } else { Vec::new() };
        if criteria.url && sites.is_empty() {
            continue;
        }
        let key = match (criteria.title, title(&entry)) {
            (false, _) => String::new(),
            (true, Some(title)) => title.to_lowercase(),
            (true, None) => continue,
        };
        by_title.entry(key).or_default().push((entry, sites));
    }
    let mut found = Vec::new();
    for entries in by_title.into_values() {
        let groups = if criteria.url { by_site(entries) } else { vec![(String::new(), entries.into_iter().map(|(e, _)| e).collect())] };
        for (site, entries) in groups {
            let accounts = if criteria.user_name { accounts(entries) } else { vec![entries] };
            for mut entries in accounts.into_iter().filter(|entries| entries.len() > 1) {
                entries.sort_by_key(|e| std::cmp::Reverse(e.times.last_modification));
                let newest = |on: bool, of: &dyn Fn(&EntryRef<'_>) -> Option<String>| {
                    if on { entries.iter().find_map(of).unwrap_or_default() } else { String::new() }
                };
                let title = newest(criteria.title, &|e| title(e).map(str::to_string));
                let user_name = newest(criteria.user_name, &|e| user_name(e).map(str::to_string));
                let ids = entries.iter().map(|e| e.id().uuid().to_string()).collect();
                found.push(Similar { site: site.clone(), title, user_name, ids });
            }
        }
    }
    found
}

/// Entries joined by the sites they share: one address in common is enough,
/// and an entry sharing one with each of two others joins all three. Each
/// group goes by the site most of its entries have, and they come in its order.
fn by_site<'a>(entries: Vec<(EntryRef<'a>, Vec<String>)>) -> Vec<(String, Vec<EntryRef<'a>>)> {
    fn root(parent: &mut [usize], mut at: usize) -> usize {
        while parent[at] != at {
            parent[at] = parent[parent[at]];
            at = parent[at];
        }
        at
    }
    let mut parent: Vec<usize> = (0..entries.len()).collect();
    let mut first: HashMap<&str, usize> = HashMap::new();
    for (at, (_, sites)) in entries.iter().enumerate() {
        for site in sites {
            match first.get(site.as_str()) {
                Some(&other) => {
                    let (a, b) = (root(&mut parent, at), root(&mut parent, other));
                    parent[a] = b;
                }
                None => {
                    first.insert(site, at);
                }
            }
        }
    }
    let roots: Vec<usize> = (0..entries.len()).map(|at| root(&mut parent, at)).collect();
    let mut groups: BTreeMap<usize, Vec<(EntryRef<'a>, Vec<String>)>> = BTreeMap::new();
    for (entry, at) in entries.into_iter().zip(roots) {
        groups.entry(at).or_default().push(entry);
    }
    let mut named: Vec<(String, Vec<EntryRef<'a>>)> = groups
        .into_values()
        .map(|group| {
            let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
            for site in group.iter().flat_map(|(_, sites)| sites) {
                *counts.entry(site).or_default() += 1;
            }
            // The most common; among as common ones, the first by name.
            let name = counts
                .iter()
                .max_by_key(|(site, n)| (**n, std::cmp::Reverse(**site)))
                .map(|(site, _)| site.to_string())
                .unwrap_or_default();
            (name, group.into_iter().map(|(entry, _)| entry).collect())
        })
        .collect();
    named.sort_by(|a, b| a.0.cmp(&b.0));
    named
}

/// One site's entries by account: the same user name, told apart without
/// regard to case. Entries without one go with the site's only user name;
/// when it has several, they are an account of their own, as no one can tell
/// whose they are.
fn accounts(entries: Vec<EntryRef<'_>>) -> Vec<Vec<EntryRef<'_>>> {
    let mut named: BTreeMap<String, Vec<EntryRef<'_>>> = BTreeMap::new();
    let mut unnamed = Vec::new();
    for entry in entries {
        match user_name(&entry) {
            Some(name) => named.entry(name.to_lowercase()).or_default().push(entry),
            None => unnamed.push(entry),
        }
    }
    let mut accounts: Vec<_> = named.into_values().collect();
    match accounts.as_mut_slice() {
        [only] => only.append(&mut unnamed),
        _ => accounts.push(unnamed),
    }
    accounts
}

/// The entry's user name, trimmed; `None` when it has none.
fn user_name<'e>(entry: &'e EntryRef<'_>) -> Option<&'e str> {
    entry.get(keepass::db::fields::USERNAME).map(str::trim).filter(|name| !name.is_empty())
}

/// The entry's title, trimmed; `None` when it has none.
fn title<'e>(entry: &'e EntryRef<'_>) -> Option<&'e str> {
    entry.get(keepass::db::fields::TITLE).map(str::trim).filter(|title| !title.is_empty())
}

/// Merges `others` into `keep`: its empty fields are filled from them, a
/// value that differs from its own becomes an additional field named after
/// the entry it came from, tags are joined and files carried over (the same
/// content once). Then `others` go to the recycle bin. One edit of `keep`:
/// its previous version goes into its history. An entry gone meanwhile is
/// left out.
pub fn merge(db: &mut Database, keep: EntryId, others: &[EntryId], hidden: &HashSet<GroupId>) -> Result<(), String> {
    merge_noting(db, keep, others, hidden).map(|_| ())
}

/// [merge], returning the titles of the entries whose passkey was left out
/// (see [take]).
pub(crate) fn merge_noting(db: &mut Database, keep: EntryId, others: &[EntryId], hidden: &HashSet<GroupId>) -> Result<Vec<String>, String> {
    let kept = db.entry(keep).ok_or(NOT_FOUND)?;
    let mut data = edit::read(&kept, edit::path_of(db, kept.parent().id()));
    let mut contents: Vec<Zeroizing<Vec<u8>>> = kept.attachments_named().map(|(_, a)| Zeroizing::new(a.data.get().clone())).collect();
    let mut files: Vec<FileEdit> = Vec::new();
    let mut passkeys_left = Vec::new();
    let others: Vec<EntryId> = others.iter().copied().filter(|&id| id != keep && db.entry(id).is_some()).collect();
    for &id in &others {
        let other = db.entry(id).expect("checked above");
        let from = edit::read(&other, Vec::new());
        if !take(&mut data, &from) {
            passkeys_left.push(titled(&from).to_string());
        }
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
    Ok(passkeys_left)
}

/// What a merge gives the kept entry, by field name (`UserName`,
/// `Password (Gmail)`), tag and file name, for the window to mark in the entry
/// as it would be.
#[derive(Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Added {
    /// Fields filled or added.
    pub fields: Vec<String>,
    pub tags: Vec<String>,
    /// By the names they will have.
    pub files: Vec<String>,
    /// It gets another's icon.
    pub icon: bool,
}

/// What `after` (the kept entry merged) has that `before` did not.
pub fn added(before: &EntryRef<'_>, after: &EntryRef<'_>) -> Added {
    let had = |name: &str| before.attachment_by_name(name).is_some();
    Added {
        fields: after
            .fields
            .iter()
            .filter(|(name, value)| before.fields.get(name.as_str()).map(|v| v.get()) != Some(value.get()))
            .map(|(name, _)| name.clone())
            .collect(),
        tags: after.tags.iter().filter(|t| !before.tags.contains(t)).cloned().collect(),
        files: after.attachments_named().map(|(name, _)| name).filter(|name| !had(name)).map(str::to_string).collect(),
        icon: edit::icon_choice(before) != edit::icon_choice(after),
    }
}

/// Adds what `from` has to `into` (see [merge]).
/// Returns false when `from`'s passkey is left out: `into` has another one.
fn take(into: &mut EntryData, from: &EntryData) -> bool {
    let source = titled(from);
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
    // A passkey is its fields together, and an entry has one: when both
    // have one, `from`'s is not taken, field by field or otherwise.
    let credential = |data: &EntryData| data.fields.iter().find(|f| f.name == passkey::CREDENTIAL_ID).map(|f| f.value.clone());
    let has_passkey = |data: &EntryData| data.fields.iter().any(|f| f.name.starts_with(passkey::PREFIX));
    let both_passkeys = has_passkey(into) && has_passkey(from);
    let passkey_taken = !both_passkeys || credential(into) == credential(from);
    for field in &from.fields {
        if field.name.starts_with(MORE_URLS) {
            urls.push(&field.value);
            continue;
        }
        if both_passkeys && field.name.starts_with(passkey::PREFIX) {
            continue;
        }
        match into.fields.iter().find(|f| f.name == field.name) {
            Some(ours) if ours.value == field.value => {}
            None => into.fields.push(field.clone()),
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
    passkey_taken
}

fn titled(data: &EntryData) -> &str {
    match data.title.trim() {
        "" => "another entry",
        title => title,
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
        // A port is part of the site; the default one is not given.
        assert_eq!(site_of("https://nas.example.com:5001/").as_deref(), Some("example.com:5001"));
        assert_eq!(site_of("nas.example.com:8443").as_deref(), Some("example.com:8443"));
        assert_eq!(site_of("https://example.com:443/").as_deref(), Some("example.com"));
        assert_eq!(site_of("192.168.1.10:5000").as_deref(), Some("192.168.1.10:5000"));
    }

    #[test]
    fn finds_entries_for_the_same_site() {
        let mut db = Database::new();
        add(&mut db, "Gmail", "https://mail.google.com", |_| {});
        add(&mut db, "Google", "accounts.google.com", |_| {});
        add(&mut db, "GitHub", "https://github.com", |_| {});
        add(&mut db, "No URL", "", |_| {});
        add(&mut db, "No URL 2", "", |_| {});
        let found = find(db.iter_all_entries(), Criteria::default());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].site, "google.com");
        assert_eq!(found[0].ids.len(), 2);
    }

    #[test]
    fn further_urls_join_entries_and_ports_keep_them_apart() {
        let more = |url: &'static str| move |e: &mut keepass::db::EntryMut<'_>| e.set_unprotected("KP2A_URL_1", url);
        let mut db = Database::new();
        add(&mut db, "Mail", "https://mail.example.org", more("https://shop.example.com"));
        add(&mut db, "Shop", "https://example.com", |_| {});
        add(&mut db, "Shop app", "androidapp://com.example.shop", more("https://www.example.com/cart"));
        add(&mut db, "NAS admin", "nas.example.net:5001", |_| {});
        add(&mut db, "NAS files", "https://nas.example.net:8443", |_| {});
        let found = find(db.iter_all_entries(), Criteria::default());
        // Mail and Shop app reach example.com through a further URL; the NAS's two ports are two sites.
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].site, "example.com");
        assert_eq!(found[0].ids.len(), 3);
    }

    #[test]
    fn the_criteria_chosen_decide() {
        let user = |name: &'static str| move |e: &mut keepass::db::EntryMut<'_>| e.set_unprotected(fields::USERNAME, name);
        let mut db = Database::new();
        add(&mut db, "Bank", "https://bank.example.com", user("alice"));
        add(&mut db, " bank ", "https://other.example.org", user("alice"));
        add(&mut db, "Bank", "https://bank.example.com", user("bob"));
        add(&mut db, "", "", user("alice"));
        let found = |title, user_name, url| {
            let mut found: Vec<(String, String, String, usize)> = find(db.iter_all_entries(), Criteria { title, user_name, url })
                .into_iter()
                .map(|s| (s.title.to_lowercase(), s.user_name, s.site, s.ids.len()))
                .collect();
            found.sort();
            found
        };
        let one = |title: &str, user: &str, site: &str, n| vec![(title.to_string(), user.to_string(), site.to_string(), n)];
        assert_eq!(found(true, false, false), one("bank", "", "", 3));
        assert_eq!(found(true, true, false), one("bank", "alice", "", 2));
        assert_eq!(found(false, true, false), one("", "alice", "", 3));
        assert_eq!(found(false, false, true), one("", "", "example.com", 2));
        assert_eq!(found(true, false, true), one("bank", "", "example.com", 2));
        assert!(found(true, true, true).is_empty());
        assert!(found(false, false, false).is_empty());
    }

    #[test]
    fn different_user_names_are_different_accounts() {
        let user = |name: &'static str| move |e: &mut keepass::db::EntryMut<'_>| e.set_unprotected(fields::USERNAME, name);
        let mut db = Database::new();
        add(&mut db, "Gmail", "https://mail.google.com", user("alice@example.com"));
        add(&mut db, "Google", "accounts.google.com", user(" Alice@Example.com "));
        add(&mut db, "Google (Bob)", "https://google.com", user("bob@example.com"));
        add(&mut db, "Google (shared)", "https://google.com", user("carol@example.com"));
        add(&mut db, "Google (?)", "https://google.com", |_| {});
        add(&mut db, "Google (??)", "https://google.com", |_| {});
        add(&mut db, "GitHub", "https://github.com", user("alice"));
        add(&mut db, "GitHub (old)", "https://github.com", |_| {});
        let found: Vec<(String, String, usize)> =
            find(db.iter_all_entries(), Criteria::default()).into_iter().map(|s| (s.site, s.user_name.to_lowercase(), s.ids.len())).collect();
        // The same name in any case is one account and Bob and Carol have one
        // entry each; with several names on a site the entries without one are
        // their own, with one name they join it.
        let expected = [("github.com", "alice", 2), ("google.com", "alice@example.com", 2), ("google.com", "", 2)];
        assert_eq!(found, expected.map(|(site, user, n)| (site.to_string(), user.to_string(), n)));
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
    fn added_names_what_a_merge_gives_the_kept_entry() {
        let mut db = Database::new();
        let keep = add(&mut db, "Google", "https://accounts.google.com", |e| e.set_protected(fields::PASSWORD, "new-pass"));
        let other = add(&mut db, "Gmail", "https://mail.google.com", |e| {
            e.set_unprotected(fields::USERNAME, "me@gmail.com");
            e.set_protected(fields::PASSWORD, "old-pass");
            e.tags = vec!["Mail".into()];
            e.add_attachment("codes.txt", Value::protected(b"1234".to_vec()));
        });
        let hidden = edit::hidden_groups(&db);
        let mut merged = db.clone();
        merge(&mut merged, keep, &[other], &hidden).unwrap();
        let mut added = added(&db.entry(keep).unwrap(), &merged.entry(keep).unwrap());
        added.fields.sort();
        assert_eq!(added.fields, ["KP2A_URL_1", "Password (Gmail)", fields::USERNAME]);
        assert_eq!(added.tags, ["Mail"]);
        assert_eq!(added.files, ["codes.txt"]);
        assert!(!added.icon);
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
        let names: Vec<String> = db.entry(keep).unwrap().fields.keys().cloned().collect();
        assert!(!names.iter().any(|n| n.contains("(Google passkey)")), "no part of it: {names:?}");
    }

    #[test]
    fn a_passkey_comes_whole_to_an_entry_without_one_and_one_left_out_is_named() {
        let mut db = Database::new();
        let keep = add(&mut db, "Google", "google.com", |_| {});
        let first = add(&mut db, "Passkey A", "google.com", |e| {
            e.set_unprotected(passkey::CREDENTIAL_ID, "cred-a");
            e.set_protected(passkey::PRIVATE_KEY, "key-a");
        });
        let same = add(&mut db, "Passkey A again", "google.com", |e| {
            e.set_unprotected(passkey::CREDENTIAL_ID, "cred-a");
            e.set_protected(passkey::PRIVATE_KEY, "key-a");
        });
        let second = add(&mut db, "Passkey B", "google.com", |e| {
            e.set_unprotected(passkey::CREDENTIAL_ID, "cred-b");
            e.set_protected(passkey::PRIVATE_KEY, "key-b");
        });
        let hidden = edit::hidden_groups(&db);
        assert_eq!(merge_noting(&mut db, keep, &[first, same, second], &hidden).unwrap(), ["Passkey B"]);
        assert_eq!(field(&db, keep, passkey::CREDENTIAL_ID).as_deref(), Some("cred-a"));
        assert_eq!(field(&db, keep, passkey::PRIVATE_KEY).as_deref(), Some("key-a"));
    }
}
