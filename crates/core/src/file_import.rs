//! Importing a file another password manager exported (#238): CSV as
//! browsers and managers write it (the columns found by their names),
//! Bitwarden's unencrypted JSON (passkeys included), a CXF JSON file, or an
//! authenticator's: Aegis' export, plain or encrypted (#282, [crate::aegis]). It
//! is read into the same [Import] as Android's Credential Transfer, so the
//! entries are added the same way: new ones, in a group of their own.
//!
//! Such a file holds every secret in clear: its content is wiped once read,
//! and never logged.

use crate::aegis;
use crate::cxf::{self, list, text, Fields, Import, Passkey, MORE_URLS};
use crate::edit::{EntryData, FAVORITE};
use base64::Engine;
use chrono::{DateTime, NaiveDateTime};
use serde_json::Value;
use std::collections::HashMap;
use zeroize::Zeroizing;

/// Reads the file `name` (its name is what the entries' group is called
/// after) with `content`. The caller wipes `content` itself.
pub fn parse(name: &str, content: &[u8]) -> Result<Import, String> {
    parse_with_password(name, content, None)
}

/// [parse], for a file that needs its `password` to be read ([needs_password]).
pub fn parse_with_password(name: &str, content: &[u8], password: Option<&str>) -> Result<Import, String> {
    let text = std::str::from_utf8(content).map_err(|_| "The file is not text (UTF-8): export it again as CSV or JSON".to_string())?;
    let text = text.trim_start_matches('\u{feff}');
    let mut import = if text.trim_start().starts_with('{') { json(text, password)? } else { csv(text)? };
    import.exporter = name.to_string();
    Ok(import)
}

/// Whether the file is one only its password opens: an encrypted Aegis
/// export. The app asks for it, then reads the file with
/// [parse_with_password].
pub fn needs_password(content: &[u8]) -> bool {
    let Ok(mut root) = serde_json::from_slice::<Value>(content) else { return false };
    let locked = aegis::is_export(&root) && aegis::is_encrypted(&root);
    cxf::wipe(&mut root);
    locked
}

/// A Bitwarden export, a CXF one, or Aegis'.
fn json(text: &str, password: Option<&str>) -> Result<Import, String> {
    let mut root: Value = serde_json::from_str(text).map_err(|e| format!("Not a JSON file PswManager knows: {e}"))?;
    let import = if aegis::is_export(&root) {
        aegis::read(&root, password)
    } else if root.get("encrypted").and_then(Value::as_bool) == Some(true) {
        Err("An encrypted Bitwarden export: export it again as JSON, not the encrypted kind".into())
    } else if root.get("items").is_some_and(Value::is_array) {
        Ok(bitwarden(&root))
    } else if root.get("accounts").is_some() {
        cxf::read_header(&root)
    } else {
        Err("Not a JSON file PswManager knows (Bitwarden's export, Credential Exchange, or Aegis')".into())
    };
    cxf::wipe(&mut root);
    import
}

// ------------------------------------------------------------------ Bitwarden

/// Bitwarden's item types.
const LOGIN: u64 = 1;
const SECURE_NOTE: u64 = 2;
const CARD: u64 = 3;
const IDENTITY: u64 = 4;
const SSH_KEY: u64 = 5;

fn bitwarden(root: &Value) -> Import {
    let folders: HashMap<&str, Vec<String>> = list(root, "folders")
        .filter_map(|f| Some((text(f, "id")?, path(text(f, "name")?))))
        .collect();
    let mut import = Import { exporter: String::new(), entries: Vec::new(), skipped: Vec::new() };
    for item in list(root, "items") {
        let mut data = EntryData::default();
        data.title = text(item, "name").unwrap_or("").trim().to_string();
        data.group = text(item, "folderId").and_then(|id| folders.get(id)).cloned().unwrap_or_default();
        data.notes = text(item, "notes").unwrap_or("").to_string();
        if item.get("favorite").and_then(Value::as_bool) == Some(true) {
            data.tags.push(FAVORITE.to_string());
        }
        let (mut fields, mut passkeys, mut why) = (Fields::default(), Vec::new(), Vec::new());
        match item.get("type").and_then(Value::as_u64) {
            Some(LOGIN) => {
                if let Some(login) = item.get("login") {
                    login_of(login, &mut data, &mut fields, &mut passkeys, &mut why);
                }
            }
            Some(SECURE_NOTE) => {}
            Some(CARD) => values(item.get("card"), "Card", &["number", "code"], &mut fields),
            Some(IDENTITY) => values(item.get("identity"), "", &["ssn", "passportNumber", "licenseNumber"], &mut fields),
            Some(SSH_KEY) => {
                let key = item.get("sshKey");
                fields.add("SSH private key", key.and_then(|k| text(k, "privateKey")).unwrap_or(""), true);
                fields.add("SSH public key", key.and_then(|k| text(k, "publicKey")).unwrap_or(""), false);
                fields.add("SSH key fingerprint", key.and_then(|k| text(k, "keyFingerprint")).unwrap_or(""), false);
            }
            kind => why.push(format!("Something of a kind PswManager does not know ({})", kind.map_or("?".into(), |k| k.to_string()))),
        }
        for field in list(item, "fields") {
            let name = text(field, "name").unwrap_or("Field");
            match field.get("type").and_then(Value::as_u64) {
                // Linked: a name for another of the item's values, no value of its own.
                Some(3) => {}
                kind => fields.add(name, text(field, "value").unwrap_or(""), kind == Some(1)),
            }
        }
        let times = (time(item, "creationDate"), time(item, "revisionDate"));
        cxf::finish(&mut import, data, fields, passkeys, why, times);
    }
    import
}

/// A login's user name, password, addresses, TOTP and passkeys.
fn login_of(login: &Value, data: &mut EntryData, fields: &mut Fields, passkeys: &mut Vec<Passkey>, why: &mut Vec<String>) {
    data.username = text(login, "username").unwrap_or("").to_string();
    data.password = text(login, "password").unwrap_or("").to_string();
    data.otp = text(login, "totp").unwrap_or("").trim().to_string();
    urls(list(login, "uris").filter_map(|u| text(u, "uri")), data, fields);
    for credential in list(login, "fido2Credentials") {
        match bitwarden_passkey(credential) {
            Ok(passkey) => passkeys.push(passkey),
            Err(e) => why.push(format!("A passkey: {e}")),
        }
    }
}

/// The first address is the entry's URL, more go in fields as Keepass2Android
/// keeps them (`KP2A_URL_n`).
fn urls<'a>(urls: impl Iterator<Item = &'a str>, data: &mut EntryData, fields: &mut Fields) {
    let mut urls = urls.map(str::trim).filter(|u| !u.is_empty());
    data.url = urls.next().unwrap_or("").to_string();
    for (n, url) in urls.enumerate() {
        fields.add(&format!("{MORE_URLS}_{}", n + 1), url, false);
    }
}

/// A Bitwarden passkey: its credential id is a GUID (one Bitwarden made) or
/// `b64.` and base64url; the user handle and the PKCS#8 key are base64url.
fn bitwarden_passkey(credential: &Value) -> Result<Passkey, String> {
    let need = |key| text(credential, key).filter(|v| !v.is_empty()).ok_or_else(|| format!("its {key} is missing"));
    let id = need("credentialId")?;
    let id = match id.strip_prefix("b64.") {
        Some(base64url) => base64url.trim_end_matches('=').to_string(),
        None => {
            let hex = id.replace('-', "");
            let bytes = (0..hex.len())
                .step_by(2)
                .map(|i| hex.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok()))
                .collect::<Option<Vec<u8>>>()
                .filter(|b| b.len() == 16)
                .ok_or("its credential id is neither a GUID nor base64url")?;
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
        }
    };
    let username = text(credential, "userName").unwrap_or("");
    Passkey::new(need("rpId")?, username, &id, need("userHandle")?.trim_end_matches('='), need("keyValue")?)
}

/// The string members of `values` as fields named after them (`Card number`;
/// `prefix` and the member's name as words), the `secret` ones protected.
fn values(values: Option<&Value>, prefix: &str, secret: &[&str], fields: &mut Fields) {
    let Some(members) = values.and_then(Value::as_object) else { return };
    for (member, value) in members {
        let Some(value) = value.as_str() else { continue };
        let words = cxf::words(member);
        let name = if prefix.is_empty() { words } else { format!("{prefix} {}", words.to_lowercase()) };
        fields.add(&name, value, secret.contains(&member.as_str()));
    }
}

/// A Bitwarden time (RFC 3339).
fn time(value: &Value, key: &str) -> Option<NaiveDateTime> {
    DateTime::parse_from_rfc3339(text(value, key)?).ok().map(|t| t.naive_utc())
}

/// A folder's path (`Work/Mail`, or LastPass's `Work\Mail`) as group names.
fn path(name: &str) -> Vec<String> {
    name.split(['/', '\\']).map(str::trim).filter(|n| !n.is_empty()).map(str::to_string).collect()
}

// ------------------------------------------------------------------------ CSV

/// What a column holds, by its name in the first line.
#[derive(Clone, Copy, PartialEq)]
enum Column {
    Title,
    Url,
    Username,
    Password,
    Notes,
    Totp,
    /// A folder or group path.
    Group,
    /// KeePass's group path, which starts with the top group's name.
    KeePassGroup,
    Favorite,
    Tags,
    /// Bitwarden's custom fields, one `name: value` a line.
    BitwardenFields,
    /// What another app keeps for itself (ids, times, its icon).
    Ignored,
    /// Anything else: an additional field under the column's name.
    Other,
}

fn column(name: &str) -> Column {
    match name.trim().to_lowercase().as_str() {
        "name" | "title" | "account" => Column::Title,
        "url" | "login_uri" | "website" | "web site" | "uri" | "web address" => Column::Url,
        "username" | "login_username" | "user name" | "login" | "user" => Column::Username,
        "password" | "login_password" => Column::Password,
        "note" | "notes" | "extra" | "comments" => Column::Notes,
        "totp" | "login_totp" | "otpauth" | "otp" | "one-time password" => Column::Totp,
        "folder" | "grouping" => Column::Group,
        "group" => Column::KeePassGroup,
        "favorite" | "fav" => Column::Favorite,
        "tags" => Column::Tags,
        "fields" => Column::BitwardenFields,
        "guid" | "id" | "type" | "reprompt" | "icon" | "archived" | "httprealm" | "formactionorigin" | "timecreated"
        | "timelastused" | "timepasswordchanged" | "created" | "last modified" | "modified" => Column::Ignored,
        _ => Column::Other,
    }
}

fn csv(text: &str) -> Result<Import, String> {
    let mut rows = rows(text).into_iter();
    let header = rows.next().ok_or("The file is empty")?;
    // KeePassXC's `Group` starts with the top group (its file has `Last Modified`);
    // anyone else's is a plain path.
    let keepass = header.iter().any(|name| name.trim().eq_ignore_ascii_case("last modified"));
    let columns: Vec<Column> = header
        .iter()
        .map(|name| match column(name) {
            Column::KeePassGroup if !keepass => Column::Group,
            column => column,
        })
        .collect();
    let known = [Column::Url, Column::Username, Column::Password];
    if !columns.iter().any(|c| known.contains(c)) {
        return Err("Not a CSV file PswManager knows: its first line should name the columns (url, username, password…)".into());
    }
    let mut import = Import { exporter: String::new(), entries: Vec::new(), skipped: Vec::new() };
    for row in rows {
        if row.iter().all(|cell| cell.trim().is_empty()) {
            continue;
        }
        let (mut data, mut fields) = (EntryData::default(), Fields::default());
        for ((name, column), cell) in header.iter().zip(&columns).zip(row.iter()) {
            let value = cell.as_str();
            match column {
                Column::Title => data.title = value.trim().to_string(),
                // Bitwarden puts every address in the cell, one a line.
                Column::Url => urls(value.lines(), &mut data, &mut fields),
                Column::Username => data.username = value.to_string(),
                Column::Password => data.password = value.to_string(),
                Column::Notes => data.notes = value.to_string(),
                Column::Totp => data.otp = value.trim().to_string(),
                Column::Group => data.group = path(value),
                Column::KeePassGroup => data.group = path(value).into_iter().skip(1).collect(),
                Column::Favorite => {
                    if ["1", "true", "yes"].contains(&value.trim().to_lowercase().as_str()) && !data.tags.iter().any(|t| t == FAVORITE) {
                        data.tags.push(FAVORITE.to_string());
                    }
                }
                Column::Tags => data.tags.extend(value.split([',', ';']).map(str::trim).filter(|t| !t.is_empty()).map(str::to_string)),
                Column::BitwardenFields => {
                    for line in value.lines() {
                        let (name, value) = line.split_once(": ").unwrap_or(("Field", line));
                        fields.add(name, value, false);
                    }
                }
                Column::Ignored => {}
                Column::Other => fields.add(name.trim(), value, false),
            }
        }
        cxf::finish(&mut import, data, fields, Vec::new(), Vec::new(), (None, None));
    }
    Ok(import)
}

/// The rows of a CSV file (RFC 4180: quoted cells may hold commas, quotes
/// doubled and line breaks). Each cell is copied once, at its size, and wiped
/// when dropped: no copy of a secret is left behind as a cell grows.
fn rows(text: &str) -> Vec<Vec<Zeroizing<String>>> {
    let bytes = text.as_bytes();
    let next_break = |from: usize| text[from..].find([',', '\n', '\r']).map_or(text.len(), |n| from + n);
    let (mut rows, mut row, mut at) = (Vec::new(), Vec::new(), 0);
    while at < text.len() {
        let cell = if bytes[at] == b'"' {
            // Up to the closing quote; a doubled quote is one.
            let mut end = at + 1;
            while end < text.len() && !(bytes[end] == b'"' && bytes.get(end + 1) != Some(&b'"')) {
                end += if bytes[end] == b'"' { 2 } else { 1 };
            }
            let raw = &text[at + 1..end.min(text.len())];
            let mut cell = Zeroizing::new(String::with_capacity(raw.len()));
            let mut chars = raw.chars().peekable();
            while let Some(c) = chars.next() {
                match c {
                    '"' => {
                        chars.next();
                        cell.push('"');
                    }
                    // A line break is `\n` in a cell too.
                    '\r' if chars.peek() == Some(&'\n') => {}
                    c => cell.push(c),
                }
            }
            // Anything between the closing quote and the next comma is not the cell's.
            at = next_break((end + 1).min(text.len()));
            cell
        } else {
            let end = next_break(at);
            let cell = Zeroizing::new(text[at..end].to_string());
            at = end;
            cell
        };
        row.push(cell);
        match bytes.get(at) {
            Some(b',') => at += 1,
            _ => {
                at += if text[at..].starts_with("\r\n") { 2 } else { 1 };
                rows.push(std::mem::take(&mut row));
            }
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field<'a>(data: &'a EntryData, name: &str) -> Option<&'a str> {
        data.fields.iter().find(|f| f.name == name).map(|f| f.value.as_str())
    }

    #[test]
    fn chrome_csv() {
        let csv = "name,url,username,password,note\r\nExample,https://example.com/login,alice@example.com,\"pa,ss\"\"word\",\"two\nlines\"\r\n,https://bookmark.example.org,,,\r\n";
        let import = parse("Chrome Passwords.csv", csv.as_bytes()).unwrap();
        assert_eq!(import.exporter, "Chrome Passwords.csv");
        assert_eq!(import.entries.len(), 2);
        let data = &import.entries[0].data;
        assert_eq!((data.title.as_str(), data.url.as_str()), ("Example", "https://example.com/login"));
        assert_eq!((data.username.as_str(), data.password.as_str()), ("alice@example.com", "pa,ss\"word"));
        assert_eq!(data.notes, "two\nlines");
        // A row with an address only is kept as a bookmark, titled after its host.
        assert_eq!(import.entries[1].data.title, "bookmark.example.org");
    }

    #[test]
    fn firefox_csv_titles_by_host_and_drops_its_own_columns() {
        let csv = "\u{feff}\"url\",\"username\",\"password\",\"httpRealm\",\"formActionOrigin\",\"guid\",\"timeCreated\"\n\"https://www.example.net\",\"bob\",\"secret\",,\"https://www.example.net\",\"{1234}\",\"1700000000000\"\n";
        let import = parse("logins.csv", csv.as_bytes()).unwrap();
        let data = &import.entries[0].data;
        assert_eq!(data.title, "www.example.net");
        assert!(data.fields.is_empty());
    }

    #[test]
    fn bitwarden_and_keepass_csv() {
        let csv = "folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\nWork/Mail,1,login,Mail,,\"PIN: 1234\nRecovery: abc\",0,https://mail.example.com,carol,pw,JBSWY3DPEHPK3PXP\n";
        let data = &parse("bitwarden.csv", csv.as_bytes()).unwrap().entries[0].data;
        assert_eq!(data.group, ["Work", "Mail"]);
        assert_eq!(data.tags, [FAVORITE]);
        assert_eq!(data.otp, "JBSWY3DPEHPK3PXP");
        assert_eq!(field(data, "PIN"), Some("1234"));
        assert_eq!(field(data, "Recovery"), Some("abc"));
        let csv = "\"Group\",\"Title\",\"Username\",\"Password\",\"URL\",\"Notes\",\"TOTP\",\"Icon\",\"Last Modified\",\"Created\"\n\"Root/Shop\",\"Store\",\"dave\",\"pw\",\"https://shop.example.com\",\"\",\"\",\"0\",\"2026-01-01T00:00:00Z\",\"2026-01-01T00:00:00Z\"\n";
        let data = &parse("keepass.csv", csv.as_bytes()).unwrap().entries[0].data;
        assert_eq!(data.group, ["Shop"]);
        assert!(data.fields.is_empty());
    }

    #[test]
    fn csv_cells_with_several_addresses_and_line_breaks() {
        let csv = "name,login_uri,login_username,notes,grouping\r\nMail,\"https://a.example.com\r\nhttps://b.example.com\",x,\"one\r\ntwo\",Work\\Mail\r\n";
        let data = &parse("export.csv", csv.as_bytes()).unwrap().entries[0].data;
        assert_eq!(data.url, "https://a.example.com");
        assert_eq!(field(data, "KP2A_URL_1"), Some("https://b.example.com"));
        assert_eq!(data.notes, "one\ntwo");
        assert_eq!(data.group, ["Work", "Mail"]);
    }

    #[test]
    fn a_csv_without_known_columns_is_refused() {
        assert!(parse("other.csv", b"a,b,c\n1,2,3\n").is_err());
        assert!(parse("binary.csv", &[0xff, 0xfe, 0x00]).is_err());
    }

    #[test]
    fn aegis_exports_are_recognised() {
        let plain = r#"{"version": 1, "header": {"slots": null, "params": null}, "db": {"version": 3, "entries": [
            {"type": "totp", "name": "alice", "issuer": "Example", "info": {"secret": "JBSWY3DP", "algo": "SHA1", "digits": 6, "period": 30}}],
            "groups": []}}"#;
        assert!(!needs_password(plain.as_bytes()));
        let import = parse("aegis.json", plain.as_bytes()).unwrap();
        assert_eq!(import.entries[0].data.title, "Example");
        let locked = r#"{"version": 1, "header": {"slots": [], "params": {}}, "db": "AAAA"}"#;
        assert!(needs_password(locked.as_bytes()));
        assert!(parse("aegis.json", locked.as_bytes()).err().unwrap().contains("password is needed"));
        assert!(!needs_password(b"name,url
"));
    }

    #[test]
    fn bitwarden_json() {
        // A made-up P-256 key (PKCS#8, base64url) and ids.
        let key = "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgevZzL1gdAFr88hb2OF_2NxApJCzGCEDdfSp6VQO30hyhRANCAAQRWz-jn65BtOMvdyHKcvjBeBSDZH2r1RTwjmYSi9R_zpBnuQ4EiMnCqfMPWiZqB4QdbAd0E7oH50VpuZ1P087G";
        let json = format!(
            r#"{{"encrypted": false,
               "folders": [{{"id": "f1", "name": "Personal"}}],
               "items": [
                 {{"type": 1, "name": "Example", "folderId": "f1", "favorite": true, "notes": "n",
                   "creationDate": "2026-01-02T03:04:05.000Z", "revisionDate": "2026-02-03T04:05:06.000Z",
                   "fields": [{{"name": "PIN", "value": "1234", "type": 1}}, {{"name": "Link", "value": null, "type": 3}}],
                   "login": {{"username": "erin", "password": "pw", "totp": "otpauth://totp/Example:erin?secret=JBSWY3DPEHPK3PXP",
                     "uris": [{{"uri": "https://example.com"}}, {{"uri": "https://login.example.com"}}],
                     "fido2Credentials": [{{"credentialId": "2e8c0b5a-1d2e-4f3a-9b8c-7d6e5f4a3b2c", "keyType": "public-key",
                       "keyAlgorithm": "ECDSA", "keyCurve": "P-256", "keyValue": "{key}", "rpId": "example.com",
                       "userHandle": "dXNlci1oYW5kbGU", "userName": "erin", "counter": "0", "discoverable": "true"}}]}}}},
                 {{"type": 2, "name": "Note", "notes": "a secure note", "secureNote": {{"type": 0}}}},
                 {{"type": 3, "name": "Visa", "card": {{"cardholderName": "Erin", "number": "4111111111111111", "code": "123"}}}}
               ]}}"#
        );
        let import = parse("bitwarden.json", json.as_bytes()).unwrap();
        assert!(import.skipped.is_empty(), "{:?}", import.skipped);
        let data = &import.entries[0].data;
        assert_eq!(data.group, ["Personal"]);
        assert_eq!(data.tags, [FAVORITE]);
        assert!(data.otp.starts_with("otpauth://"));
        assert_eq!(field(data, "KP2A_URL_1"), Some("https://login.example.com"));
        assert!(data.fields.iter().any(|f| f.name == "PIN" && f.protected));
        assert!(field(data, "Link").is_none());
        assert_eq!(field(data, "KPEX_PASSKEY_RELYING_PARTY"), Some("example.com"));
        assert_eq!(field(data, "KPEX_PASSKEY_CREDENTIAL_ID"), Some("LowLWh0uTzqbjH1uX0o7LA"));
        assert!(import.entries[0].modified.is_some());
        assert_eq!(import.entries[1].data.notes, "a secure note");
        let card = &import.entries[2].data;
        assert!(card.fields.iter().any(|f| f.name == "Card number" && f.protected));
        assert_eq!(field(card, "Card cardholder name"), Some("Erin"));
    }

    #[test]
    fn an_encrypted_bitwarden_export_is_refused() {
        assert!(parse("bw.json", br#"{"encrypted": true, "items": []}"#).err().unwrap().contains("encrypted"));
    }
}
