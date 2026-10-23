//! Browser integration (#273): what the KeePassXC-Browser extension asks of the
//! database, kept where KeePassXC keeps it, so that either app serves the
//! extension with the same associations and entry settings, and sync carries
//! them. The protocol itself (keys, encryption, messages) is the Windows app's.
//!
//! - An association is the extension's identification key in the database's
//!   custom data, `KPXC_BROWSER_<name>`, with when it was made in `_CREATED_<name>`.
//! - The extension knows the database by [database_hash]: the same for a file
//!   in KeePassXC and in PswManager, so an extension associated in one works
//!   in the other.
//! - An entry's `KeePassXC-Browser Settings` (JSON: hosts it is allowed for or
//!   denied to) and `BrowserHideEntry` (on the entry or a group) keep an entry
//!   from a site: honoured, never written here.

use crate::{edit, otp};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use keepass::db::{fields, CustomDataItem, CustomDataValue, EntryRef, GroupId, GroupRef, Times};
use keepass::Database;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

/// The custom data an association's key is kept in, followed by its name.
pub const KEY_PREFIX: &str = "KPXC_BROWSER_";
/// The custom data saying when an association was made, followed by its name.
const CREATED_PREFIX: &str = "_CREATED_";
/// This app's mark of an association removed ([forget]).
const REMOVED_PREFIX: &str = "PSWM_BROWSER_REMOVED_";
/// An entry's own settings for the extension (JSON).
const ENTRY_SETTINGS: &str = "KeePassXC-Browser Settings";
/// `true` on an entry, or on a group (for its entries, the nearest group that
/// says): the extension never gets it.
const HIDE_ENTRY: &str = "BrowserHideEntry";
/// `true` / `false` on an entry or a group: the extension does not submit the form after filling it.
const SKIP_AUTO_SUBMIT: &str = "BrowserSkipAutoSubmit";
/// Where a login the extension saves goes when it names no group, as in KeePassXC.
pub const NEW_LOGINS_GROUP: &str = "KeePassXC-Browser Passwords";

/// Where a passkey the extension makes goes when it names no group, as in KeePassXC (#285).
pub const NEW_PASSKEYS_GROUP: &str = "KeePassXC-Browser Passkeys";

/// Why a passkey request from a page is refused: KeePassXC's error codes,
/// which the extension knows (and then falls back to the browser's own WebAuthn).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasskeyRefusal {
    OriginNotAllowed = 26,
    DomainNotValid = 27,
    RpIdMismatch = 28,
    NoSupportedAlgorithms = 29,
    InvalidChallenge = 32,
    InvalidUserId = 33,
}

/// Checks a passkey request (WebAuthn's `publicKey` options, as the extension
/// sends them) from a page at `origin` as KeePassXC does, and returns the site
/// (RP id) it is for, which the options then name if they did not, and the
/// page's origin as the client data names it:
///
/// - the origin is an https page on a named host;
/// - the RP id is a registrable domain (never a public suffix such as
///   `co.uk`), the page's host or a parent of it (WebAuthn's rule), or any
///   when the page is one of the site's `related` origins (which the
///   extension reads from the site, when making one);
/// - to make one (`creating`): a challenge of 16 characters or more, a user id
///   of 1 to 64 bytes, and ES256 among the algorithms when the site lists some.
pub fn check_passkey_request(options: &mut serde_json::Value, origin: &str, related: &[String], creating: bool) -> Result<(String, String), PasskeyRefusal> {
    let page = url::Url::parse(origin).map_err(|_| PasskeyRefusal::OriginNotAllowed)?;
    let origin = page.origin().ascii_serialization();
    if page.scheme() != "https" {
        return Err(PasskeyRefusal::OriginNotAllowed);
    }
    let host = match page.host() {
        Some(url::Host::Domain(name)) if name.contains('.') => name.to_string(),
        _ => return Err(PasskeyRefusal::DomainNotValid),
    };
    // The options are written to below: a page's odd shapes are refused, not trusted.
    if !options.is_object() || (creating && !options["rp"].is_object()) {
        return Err(PasskeyRefusal::RpIdMismatch);
    }
    let named = if creating { options["rp"]["id"].as_str() } else { options["rpId"].as_str() };
    let rp_id = named.filter(|id| !id.is_empty()).unwrap_or(&host).to_lowercase();
    let related = related.iter().any(|r| r.trim_end_matches('/') == origin);
    if psl::domain_str(&rp_id).is_none() || (!related && !crate::webauthn::on_site(&host, &rp_id)) {
        return Err(PasskeyRefusal::RpIdMismatch);
    }
    if creating {
        if options["challenge"].as_str().is_none_or(|c| c.len() < 16) {
            return Err(PasskeyRefusal::InvalidChallenge);
        }
        let user_id = options["user"]["id"].as_str().and_then(crate::webauthn::credential_id_bytes);
        if !user_id.is_some_and(|id| (1..=64).contains(&id.len())) {
            return Err(PasskeyRefusal::InvalidUserId);
        }
        let algorithms = options["pubKeyCredParams"].as_array().map(Vec::as_slice).unwrap_or_default();
        if !algorithms.is_empty() && !algorithms.iter().any(|a| a["alg"].as_i64() == Some(-7)) {
            return Err(PasskeyRefusal::NoSupportedAlgorithms);
        }
        options["rp"]["id"] = rp_id.clone().into();
    } else {
        if options["challenge"].as_str().is_none_or(str::is_empty) {
            return Err(PasskeyRefusal::InvalidChallenge);
        }
        options["rpId"] = rp_id.clone().into();
    }
    Ok((rp_id, origin))
}

/// A page a browser shows, whose logins are found as KeePassXC finds them:
/// the extension and Auto-Type give the same entries KeePassXC would. An
/// entry's URL (or one of its further URLs, `KP2A_URL_n`) fits the page when
///
/// - its scheme is the page's (a URL without one is https), unless the page's
///   scheme is not known (an address bar leaves it out);
/// - the port it names, if any, is the page's;
/// - the page's host is its host, or below it on the same site
///   (`login.example.com` for `example.com`; an IP address only itself),
///   `www.` aside.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    url: url::Url,
    scheme_known: bool,
}

impl Page {
    /// The page of a browser tab, as the extension gives it: an http or https address.
    pub fn of_tab(url: &str) -> Option<Page> {
        Page::web(url::Url::parse(url.trim()).ok()?, true)
    }

    /// The page an address bar shows. Browsers leave out the scheme, so an
    /// address without one fits entries of either.
    pub fn of_address(address: &str) -> Option<Page> {
        let address = address.trim();
        if address.contains(char::is_whitespace) {
            return None; // words being searched for, not an address
        }
        match address.contains("://") {
            true => Page::web(url::Url::parse(address).ok()?, true),
            false => Page::web(url::Url::parse(&format!("https://{address}")).ok()?, false),
        }
    }

    fn web(url: url::Url, scheme_known: bool) -> Option<Page> {
        let web = matches!(url.scheme(), "http" | "https") && url.host_str().is_some_and(|h| !h.is_empty());
        web.then_some(Page { url, scheme_known })
    }

    /// The page's address.
    pub fn url(&self) -> &str {
        self.url.as_str()
    }

    /// The page's host, as KeePassXC's per-entry settings name it.
    pub fn host(&self) -> &str {
        self.url.host_str().unwrap_or_default()
    }

    /// The host with the port when the address names one: `192.168.1.10:2283`.
    pub fn place(&self) -> String {
        match self.url.port() {
            Some(port) => format!("{}:{port}", self.host()),
            None => self.host().to_string(),
        }
    }

    /// Whether `other` is on the same host and port, and scheme where both know it.
    pub fn same_place(&self, other: &Page) -> bool {
        let schemes = !(self.scheme_known && other.scheme_known) || self.url.scheme() == other.url.scheme();
        schemes && self.place() == other.place()
    }

    /// Whether one of the entry's URLs fits the page.
    pub fn fits_entry(&self, entry: &EntryRef<'_>) -> bool {
        entry.fields.iter().filter(|(name, _)| crate::vault::holds_url(name)).any(|(_, value)| self.fits(value.get()))
    }

    /// Whether an entry's URL fits the page.
    pub fn fits(&self, entry_url: &str) -> bool {
        let entry_url = entry_url.trim();
        // KeePassXC's: characters no address holds.
        if entry_url.is_empty() || entry_url.contains(['<', '>', '^', '`', '{', '|', '}']) {
            return false;
        }
        let parsed = match entry_url.contains("://") {
            true => url::Url::parse(entry_url),
            false => url::Url::parse(&format!("https://{entry_url}")),
        };
        let Ok(entry) = parsed else { return false };
        let (Some(entry_host), Some(page_host)) = (entry.host(), self.url.host()) else { return false };
        if self.scheme_known && entry.scheme() != self.url.scheme() {
            return false;
        }
        if entry.port().is_some_and(|port| Some(port) != self.url.port()) {
            return false;
        }
        let (entry_name, page_name) = (entry_host.to_string(), page_host.to_string());
        let entry_name = entry_name.strip_prefix("www.").unwrap_or(&entry_name);
        let below = page_name == entry_name || page_name.ends_with(&format!(".{entry_name}"));
        below && base_domain(&entry_host) == base_domain(&page_host)
    }
}

/// The site a host is part of: its registrable domain (`example.com` for
/// `login.example.com`), else the host itself (an IP address, a local name).
fn base_domain(host: &url::Host<&str>) -> String {
    match host {
        url::Host::Domain(name) => {
            let name = name.trim_end_matches('.');
            psl::domain_str(name).unwrap_or(name).trim_start_matches("www.").to_string()
        }
        ip => ip.to_string(),
    }
}

/// How the extension tells databases apart: SHA-256 of the root group's
/// UUID (32 hex digits), in hex, as KeePassXC makes it.
pub fn database_hash(db: &Database) -> String {
    crate::dbfile::hex(&Sha256::digest(db.root().id().uuid().simple().to_string()))
}

/// The custom data keeping association `name`'s key, and when it was made.
fn key_name(name: &str) -> String {
    format!("{KEY_PREFIX}{name}")
}

fn created_name(name: &str) -> String {
    format!("{CREATED_PREFIX}{name}")
}

/// When association `name` was removed: a mark of this app's (KeePassXC
/// passes it over) so a sync does not bring the association back from
/// another copy that still has it ([merge_custom_data]).
fn removed_name(name: &str) -> String {
    format!("{REMOVED_PREFIX}{name}")
}

/// An association, as the settings list it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Association {
    pub name: String,
    /// When it was made, as the app that made it wrote it.
    pub created: Option<String>,
}

/// A custom data item's text. keepass-rs reads any text that is valid base64
/// (`true`, an association's key) as bytes: those are its base64 again.
fn text(item: &CustomDataItem) -> Option<String> {
    match &item.value {
        Some(CustomDataValue::String(text)) => Some(text.clone()),
        Some(CustomDataValue::Binary(bytes)) => Some(B64.encode(bytes)),
        None => None,
    }
}

/// A custom data item holding `value`, changed now; kept as keepass-rs reads
/// it back (see [text]), so the file reads back the same.
fn item(value: &str) -> CustomDataItem {
    let value = match B64.decode(value) {
        Ok(bytes) => CustomDataValue::Binary(bytes),
        Err(_) => CustomDataValue::String(value.to_string()),
    };
    CustomDataItem { value: Some(value), last_modification_time: Some(Times::now()) }
}

/// The identification key associated as `name`.
pub fn key(db: &Database, name: &str) -> Option<String> {
    db.meta.custom_data.get(&key_name(name)).and_then(text)
}

/// Keeps the extension's identification `key` as `name`, replacing one of that name.
pub fn associate(db: &mut Database, name: &str, key: &str) {
    let created = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    db.meta.custom_data.insert(key_name(name), item(key));
    db.meta.custom_data.insert(created_name(name), item(&created));
    db.meta.custom_data.remove(&removed_name(name));
}

/// Removes the association `name`, and marks it removed for the syncs to come.
pub fn forget(db: &mut Database, name: &str) {
    db.meta.custom_data.remove(&key_name(name));
    db.meta.custom_data.remove(&created_name(name));
    let removed = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    db.meta.custom_data.insert(removed_name(name), item(&removed));
}

/// The database's associations, by name.
pub fn associations(db: &Database) -> Vec<Association> {
    let mut all: Vec<Association> = db
        .meta
        .custom_data
        .keys()
        .filter_map(|k| k.strip_prefix(KEY_PREFIX))
        .map(|name| Association {
            name: name.to_string(),
            created: db.meta.custom_data.get(&created_name(name)).and_then(text),
        })
        .collect();
    all.sort_by_key(|a| a.name.to_lowercase());
    all
}

/// Takes into `theirs` the database custom data `ours` has that `theirs`
/// lacks or has older (associations made on this device), as a sync merges
/// two copies; then an association removed on either side ([forget]) after
/// it was made goes, so the other copy does not bring it back. True when
/// `theirs` changed.
pub fn merge_custom_data(theirs: &mut Database, ours: &Database) -> bool {
    let mut changed = false;
    for (key, mine) in &ours.meta.custom_data {
        let newer = theirs.meta.custom_data.get(key).is_none_or(|other| mine.last_modification_time > other.last_modification_time);
        if newer {
            theirs.meta.custom_data.insert(key.clone(), mine.clone());
            changed = true;
        }
    }
    let data = &mut theirs.meta.custom_data;
    let removed: Vec<(String, Option<chrono::NaiveDateTime>)> =
        data.iter().filter_map(|(key, mark)| Some((key.strip_prefix(REMOVED_PREFIX)?.to_string(), mark.last_modification_time))).collect();
    for (name, removed_at) in removed {
        if data.get(&key_name(&name)).is_some_and(|made| made.last_modification_time <= removed_at) {
            data.remove(&key_name(&name));
            data.remove(&created_name(&name));
            changed = true;
        }
    }
    changed
}

/// A login as `get-logins` gives it, with KeePassXC's names.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Login {
    pub login: String,
    pub password: String,
    /// The entry's title.
    pub name: String,
    /// The entry's UUID, 32 hex digits.
    pub uuid: String,
    /// The name of its group.
    pub group: String,
    /// The current code, when the entry has a TOTP secret.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub totp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expired: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_auto_submit: Option<String>,
}

impl Login {
    pub fn of(entry: &EntryRef<'_>) -> Login {
        let value = |name: &str| entry.get(name).unwrap_or_default().to_string();
        // In whatever layout the entry keeps it (KeePassXC's, KeePass 2's, …).
        let totp = otp::find(&entry.fields).and_then(|v| otp::Totp::parse(&v).ok()).map(|t| t.code_now().code);
        let expired = entry.times.expires == Some(true) && entry.times.expiry < Times::now();
        Login {
            login: value(fields::USERNAME),
            password: value(fields::PASSWORD),
            name: value(fields::TITLE),
            uuid: entry.id().uuid().simple().to_string(),
            group: entry.parent().name.clone(),
            totp,
            expired: expired.then_some("true"),
            skip_auto_submit: option(entry, SKIP_AUTO_SUBMIT),
        }
    }
}

/// An option KeePassXC keeps for the extension: the entry's own, else its
/// nearest group's that says.
fn option(entry: &EntryRef<'_>, name: &str) -> Option<String> {
    let db = entry.database();
    let own = entry.custom_data.get(name).and_then(text);
    own.or_else(|| edit::ancestors(db, entry.parent().id()).into_iter().find_map(|g| db.group(g)?.custom_data.get(name).and_then(text)))
}

/// Whether the extension may have the entry on a page of `host`: not hidden
/// from it, and not denied to the host in KeePassXC.
pub fn offered(entry: &EntryRef<'_>, host: &str) -> bool {
    if option(entry, HIDE_ENTRY).as_deref() == Some("true") {
        return false;
    }
    let settings = entry.custom_data.get(ENTRY_SETTINGS).and_then(text).and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
    let denied = settings.as_ref().and_then(|s| s.get("Deny")).and_then(|d| d.as_array()).is_some_and(|hosts| hosts.iter().any(|h| h.as_str() == Some(host)));
    !denied
}

/// A group as `get-database-groups` lists it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Group {
    pub name: String,
    pub uuid: String,
    pub children: Vec<Group>,
}

/// The database's groups from the root, without the recycle bin and the templates.
pub fn groups(db: &Database, hidden: &HashSet<GroupId>) -> Group {
    fn of(group: GroupRef<'_>, hidden: &HashSet<GroupId>) -> Group {
        Group {
            name: group.name.clone(),
            uuid: group.id().uuid().simple().to_string(),
            children: group.groups().filter(|g| !hidden.contains(&g.id())).map(|g| of(g, hidden)).collect(),
        }
    }
    of(db.root(), hidden)
}

/// The group with this UUID (any form), if the database has it.
pub fn group_by_uuid(db: &Database, uuid: &str) -> Option<GroupId> {
    let uuid = uuid::Uuid::parse_str(uuid.trim()).ok()?;
    let id = GroupId::from(uuid);
    db.group(id).is_some().then_some(id)
}

/// Group names from the top, without the root group, for a path the
/// extension sends (`Parent/Child`).
pub fn split_path(path: &str) -> Vec<String> {
    path.split('/').map(str::trim).filter(|n| !n.is_empty()).map(String::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passkey_requests_are_checked_as_in_keepassxc() {
        use serde_json::json;
        let create = || json!({ "rp": { "name": "Example" }, "challenge": "c2l4dGVlbi1ieXRlcy0tLQ", "user": { "id": "dXNlcg", "name": "me" }, "pubKeyCredParams": [{ "type": "public-key", "alg": -7 }] });
        let mut options = create();
        let page = |rp: &str| Ok((rp.to_string(), "https://login.example.com".to_string()));
        assert_eq!(check_passkey_request(&mut options, "https://login.example.com/", &[], true), page("login.example.com"), "the origin as the client data names it");
        assert_eq!(options["rp"]["id"], "login.example.com", "the options name the site now");
        let mut parent = create();
        parent["rp"]["id"] = "example.com".into();
        assert_eq!(check_passkey_request(&mut parent, "https://login.example.com", &[], true), page("example.com"));
        let mut suffix = create();
        suffix["rp"]["id"] = "co.uk".into();
        assert_eq!(check_passkey_request(&mut suffix, "https://shop.co.uk", &[], true), Err(PasskeyRefusal::RpIdMismatch), "not a public suffix");
        let mut odd = create();
        odd["rp"] = "x".into();
        assert_eq!(check_passkey_request(&mut odd, "https://example.com", &[], true), Err(PasskeyRefusal::RpIdMismatch), "refused, not a panic");
        let mut other = create();
        other["rp"]["id"] = "evil.com".into();
        assert_eq!(check_passkey_request(&mut other.clone(), "https://example.com", &[], true), Err(PasskeyRefusal::RpIdMismatch));
        assert!(check_passkey_request(&mut other, "https://example.com", &["https://example.com".into()], true).is_ok(), "a related origin");
        assert_eq!(check_passkey_request(&mut create(), "http://example.com", &[], true), Err(PasskeyRefusal::OriginNotAllowed));
        assert_eq!(check_passkey_request(&mut create(), "https://192.168.1.10", &[], true), Err(PasskeyRefusal::DomainNotValid));
        let mut short = create();
        short["challenge"] = "c2hvcnQ".into();
        assert_eq!(check_passkey_request(&mut short, "https://example.com", &[], true), Err(PasskeyRefusal::InvalidChallenge));
        let mut rsa = create();
        rsa["pubKeyCredParams"] = json!([{ "type": "public-key", "alg": -257 }]);
        assert_eq!(check_passkey_request(&mut rsa, "https://example.com", &[], true), Err(PasskeyRefusal::NoSupportedAlgorithms));
        let mut get = json!({ "challenge": "Y2hhbGxlbmdl" });
        assert_eq!(check_passkey_request(&mut get, "https://example.com", &[], false), Ok(("example.com".into(), "https://example.com".into())));
        assert_eq!(get["rpId"], "example.com");
    }

    #[test]
    fn pages_fit_entries_as_in_keepassxc() {
        let tab = |url: &str| Page::of_tab(url).unwrap();
        // A site's own host and the hosts below it.
        assert!(tab("https://login.example.com/x").fits("example.com"));
        assert!(tab("https://example.com/").fits("https://www.example.com"));
        assert!(!tab("https://example.com/").fits("login.example.com"), "not the hosts above");
        assert!(!tab("https://evilexample.com/").fits("example.com"));
        assert!(!tab("https://example.com.evil.net/").fits("example.com"));
        // The scheme: an entry without one is https.
        assert!(!tab("http://example.com/").fits("example.com"));
        assert!(tab("http://example.com/").fits("http://example.com"));
        // A local service: its address and port.
        assert!(tab("http://192.168.1.10:2283/auth/login").fits("http://192.168.1.10:2283"));
        assert!(!tab("http://192.168.1.10:2283/").fits("http://192.168.1.10:8080"));
        assert!(tab("http://192.168.1.10:2283/").fits("http://192.168.1.10"), "an entry without a port fits any");
        assert!(!tab("http://192.168.1.11:2283/").fits("http://192.168.1.10:2283"));
        assert!(tab("http://nas:5000/").fits("http://nas:5000"));
        assert!(!tab("https://example.com/").fits("ex<ample.com"));
        assert!(Page::of_tab("chrome://settings").is_none());
    }

    #[test]
    fn an_address_bar_s_page_fits_either_scheme() {
        let page = Page::of_address("192.168.1.10:2283/auth/login").unwrap();
        assert!(page.fits("http://192.168.1.10:2283") && page.fits("192.168.1.10:2283"));
        assert_eq!(page.place(), "192.168.1.10:2283");
        assert!(Page::of_address("search words").is_none());
        let known = Page::of_address("http://example.com/").unwrap();
        assert!(!known.fits("example.com"), "a scheme the address bar shows counts");
        assert!(page.same_place(&Page::of_tab("http://192.168.1.10:2283/other").unwrap()));
        assert!(!page.same_place(&Page::of_tab("http://192.168.1.10:8080/").unwrap()));
    }

    #[test]
    fn the_hash_is_keepassxc_s() {
        let root = uuid::Uuid::parse_str("01234567-89ab-cdef-0123-456789abcdef").unwrap();
        let db = Database::new_with_root_id(GroupId::from(root));
        // SHA-256 of "0123456789abcdef0123456789abcdef".
        assert_eq!(database_hash(&db), "3eb1bd439947eb762998e566ccc2e099c791118b2f40579cc4f7da2b5061b7f9");
    }

    #[test]
    fn associations_are_kept_listed_and_forgotten() {
        let mut db = Database::new();
        associate(&mut db, "Chrome", "a2V5LTE=");
        associate(&mut db, "firefox", "a2V5LTI=");
        assert_eq!(key(&db, "Chrome").as_deref(), Some("a2V5LTE="));
        assert_eq!(associations(&db).iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Chrome", "firefox"]);
        assert!(associations(&db)[0].created.is_some());
        forget(&mut db, "Chrome");
        assert_eq!(key(&db, "Chrome"), None);
        assert_eq!(associations(&db).len(), 1);
    }

    #[test]
    fn custom_data_reads_back_as_written() {
        let mut db = Database::new();
        associate(&mut db, "Chrome", "a2V5LTE=");
        db.root_mut().custom_data.insert(HIDE_ENTRY.into(), item("true"));
        let password = keepass::DatabaseKey::new().with_password("pw");
        let mut bytes = Vec::new();
        db.save(&mut bytes, password.clone()).unwrap();
        let back = Database::parse(&bytes, password).unwrap();
        assert_eq!(back.meta.custom_data, db.meta.custom_data);
        assert_eq!(text(&back.root().custom_data[HIDE_ENTRY]).as_deref(), Some("true"));
        assert_eq!(key(&back, "Chrome").as_deref(), Some("a2V5LTE="));
    }

    #[test]
    fn a_merge_keeps_both_copies_associations() {
        let mut theirs = Database::new();
        let mut ours = theirs.clone();
        associate(&mut theirs, "Edge", "a2V5LTE=");
        associate(&mut ours, "Chrome", "a2V5LTI=");
        merge_custom_data(&mut theirs, &ours);
        assert_eq!(associations(&theirs).len(), 2);
    }

    #[test]
    fn a_removed_association_does_not_come_back_from_the_other_copy() {
        let mut theirs = Database::new();
        associate(&mut theirs, "Chrome", "a2V5LTE=");
        let mut ours = theirs.clone();
        // Removed here a second later; the other copy still has it.
        let mut later = item("x");
        later.last_modification_time = later.last_modification_time.map(|t| t + chrono::Duration::seconds(1));
        forget(&mut ours, "Chrome");
        ours.meta.custom_data.get_mut(&removed_name("Chrome")).unwrap().last_modification_time = later.last_modification_time;
        let mut other = theirs.clone();
        merge_custom_data(&mut theirs, &ours);
        assert!(associations(&theirs).is_empty());
        merge_custom_data(&mut ours, &other);
        assert!(associations(&ours).is_empty());
        // Associated again after the removal, it stays.
        associate(&mut other, "Chrome", "a2V5LTI=");
        other.meta.custom_data.get_mut(&key_name("Chrome")).unwrap().last_modification_time = later.last_modification_time.map(|t| t + chrono::Duration::seconds(1));
        merge_custom_data(&mut ours, &other);
        assert_eq!(key(&ours, "Chrome").as_deref(), Some("a2V5LTI="));
    }

    #[test]
    fn keepassxc_s_settings_keep_an_entry_from_a_site() {
        let mut db = Database::new();
        let mut root = db.root_mut();
        let mut e = root.add_entry();
        e.set_unprotected(fields::TITLE, "Mail");
        e.custom_data.insert(ENTRY_SETTINGS.into(), item(r#"{"Allow":["mail.example"],"Deny":["evil.example"],"Realm":""}"#));
        let mut hidden_group = root.add_group();
        hidden_group.custom_data.insert(HIDE_ENTRY.into(), item("true"));
        hidden_group.add_entry().set_unprotected(fields::TITLE, "Hidden");
        let entry = |title: &str| db.iter_all_entries().find(|e| e.get(fields::TITLE) == Some(title)).unwrap();
        assert!(offered(&entry("Mail"), "mail.example"));
        assert!(!offered(&entry("Mail"), "evil.example"));
        assert!(!offered(&entry("Hidden"), "mail.example"));
    }

    #[test]
    fn a_login_has_keepassxc_s_fields() {
        let mut db = Database::new();
        let mut root = db.root_mut();
        root.name = "Root".into();
        let mut e = root.add_entry();
        e.set_unprotected(fields::TITLE, "Mail");
        e.set_unprotected(fields::USERNAME, "me");
        e.set_protected(fields::PASSWORD, "pw");
        e.custom_data.insert(SKIP_AUTO_SUBMIT.into(), item("true"));
        let login = Login::of(&db.iter_all_entries().next().unwrap());
        let json = serde_json::to_value(&login).unwrap();
        assert_eq!(json["login"], "me");
        assert_eq!(json["name"], "Mail");
        assert_eq!(json["group"], "Root");
        assert_eq!(json["skipAutoSubmit"], "true");
        assert_eq!(json["uuid"].as_str().unwrap().len(), 32);
        assert!(json.get("totp").is_none() && json.get("expired").is_none());

        // A TOTP kept as KeePass 2 keeps it gives a code too.
        let mut db = Database::new();
        db.root_mut().add_entry().set_protected("TimeOtp-Secret-Base32", "JBSWY3DPEHPK3PXP");
        let login = Login::of(&db.iter_all_entries().next().unwrap());
        assert_eq!(login.totp.map(|code| code.len()), Some(6));
    }

    #[test]
    fn groups_leave_out_the_hidden_ones() {
        let mut db = Database::new();
        let mut root = db.root_mut();
        root.add_group().name = "Work".into();
        let bin = root.add_group().id();
        let tree = groups(&db, &HashSet::from([bin]));
        assert_eq!(tree.children.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Work"]);
        assert_eq!(split_path(" Work / Mail /"), ["Work", "Mail"]);
    }
}
