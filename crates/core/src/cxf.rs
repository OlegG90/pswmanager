//! Importing from another password manager: a FIDO Credential Exchange
//! Format (CXF 1.0) export, as Android's Credential Transfer hands it over,
//! read into new entries (see `docs/cxp-research.md` for the mapping). Every
//! item becomes a new entry; nothing in the database is changed or merged.

use crate::edit::{self, EntryData, FieldData, FAVORITE};
use crate::otp;
use base64::Engine;
use chrono::{DateTime, NaiveDate, NaiveDateTime};
use keepass::db::GroupId;
use keepass::Database;
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use zeroize::Zeroize;

/// What an export holds, read and ready to add.
pub struct Import {
    /// The app it came from, as it names itself.
    pub exporter: String,
    pub entries: Vec<Imported>,
    /// What could not be brought over, and why.
    pub skipped: Vec<Skipped>,
}

/// An entry to add, with the times the other app kept for it.
pub struct Imported {
    pub data: EntryData,
    pub created: Option<NaiveDateTime>,
    pub modified: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Skipped {
    /// The item's title.
    pub title: String,
    pub why: String,
}

impl Import {
    /// The group the entries go in: `Imported from <app> <date>`.
    pub fn group_name(&self, day: NaiveDate) -> String {
        format!("Imported from {} {}", self.exporter, day.format("%Y-%m-%d"))
    }
}

/// The attributes KeePassXC keeps a passkey in.
const PASSKEY_USERNAME: &str = "KPEX_PASSKEY_USERNAME";
const PASSKEY_CREDENTIAL_ID: &str = "KPEX_PASSKEY_CREDENTIAL_ID";
const PASSKEY_PRIVATE_KEY: &str = "KPEX_PASSKEY_PRIVATE_KEY_PEM";
const PASSKEY_RELYING_PARTY: &str = "KPEX_PASSKEY_RELYING_PARTY";
const PASSKEY_USER_HANDLE: &str = "KPEX_PASSKEY_USER_HANDLE";

/// Reads a CXF 1.x export. The JSON's secrets are wiped once read; the
/// caller wipes `json` itself.
pub fn parse(json: &str) -> Result<Import, String> {
    let mut root: Value = serde_json::from_str(json).map_err(|e| format!("Not a Credential Exchange export: {e}"))?;
    let import = read_header(&root);
    wipe(&mut root);
    import
}

/// Adds the entries under the group at `under` (made where missing), with the
/// collections they were in as groups below it. Returns how many were added.
pub fn add(db: &mut Database, import: &Import, under: &[String], hidden: &HashSet<GroupId>) -> Result<usize, String> {
    for imported in &import.entries {
        let mut data = imported.data.clone();
        data.group = under.iter().chain(&imported.data.group).cloned().collect();
        let id = edit::apply(db, None, &data, hidden)?;
        let mut entry = db.entry_mut(id).expect("just added");
        if let Some(created) = imported.created {
            entry.times.creation = Some(created);
        }
        if let Some(modified) = imported.modified {
            entry.times.last_modification = Some(modified);
        }
    }
    Ok(import.entries.len())
}

fn read_header(root: &Value) -> Result<Import, String> {
    let major = root.get("version").and_then(|v| v.get("major")).and_then(Value::as_u64);
    if major != Some(1) {
        return Err("This export is in a Credential Exchange version PswManager cannot read".into());
    }
    let exporter = ["exporterDisplayName", "exporterRpId"]
        .iter()
        .find_map(|k| text(root, k).filter(|v| !v.trim().is_empty()))
        .unwrap_or("another app")
        .to_string();
    let accounts = root.get("accounts").and_then(Value::as_array).ok_or("The export holds no accounts")?;
    let mut import = Import { exporter, entries: Vec::new(), skipped: Vec::new() };
    for account in accounts {
        // Several accounts (a family's, say) each get a group of their own.
        let top: Vec<String> = match accounts.len() {
            1 => Vec::new(),
            _ => vec![["fullName", "username", "email"].iter().find_map(|k| text(account, k).filter(|v| !v.trim().is_empty())).unwrap_or("Account").to_string()],
        };
        let mut groups = HashMap::new();
        for collection in list(account, "collections") {
            index_collection(collection, &top, &mut groups);
        }
        for item in list(account, "items") {
            let group = text(item, "id").and_then(|id| groups.get(id)).cloned().unwrap_or_else(|| top.clone());
            read_item(item, group, &mut import);
        }
    }
    Ok(import)
}

/// Notes the group path of each item in `collection` and the collections
/// below it; an item in several keeps the first.
fn index_collection(collection: &Value, above: &[String], groups: &mut HashMap<String, Vec<String>>) {
    let mut path = above.to_vec();
    path.push(text(collection, "title").unwrap_or("Collection").to_string());
    for linked in list(collection, "items") {
        if let Some(item) = text(linked, "item") {
            groups.entry(item.to_string()).or_insert_with(|| path.clone());
        }
    }
    for sub in list(collection, "subCollections") {
        index_collection(sub, &path, groups);
    }
}

fn read_item(item: &Value, group: Vec<String>, import: &mut Import) {
    let title = text(item, "title").unwrap_or("").trim().to_string();
    let mut data = EntryData::default();
    data.title = title.clone();
    data.group = group;
    let mut fields = Fields::default();
    data.tags = list(item, "tags").filter_map(Value::as_str).map(str::to_string).collect();
    if item.get("favorite").and_then(Value::as_bool) == Some(true) && !data.tags.iter().any(|t| t == FAVORITE) {
        data.tags.push(FAVORITE.to_string());
    }
    let scope = item.get("scope");
    let mut urls = scope
        .into_iter()
        .flat_map(|s| list(s, "urls"))
        .filter_map(Value::as_str)
        .map(str::to_string)
        // Android apps as Keepass2Android keeps them.
        .chain(scope.into_iter().flat_map(|s| list(s, "androidApps")).filter_map(|a| text(a, "bundleId")).map(|id| format!("androidapp://{id}")));
    data.url = urls.next().unwrap_or_default();
    for (n, url) in urls.enumerate() {
        fields.add(&format!("KP2A_URL_{}", n + 1), &url, false);
    }

    let skip = |import: &mut Import, why: String| import.skipped.push(Skipped { title: title.clone(), why });
    let (mut has_login, mut has_otp) = (false, false);
    let mut passkeys = Vec::new();
    for credential in list(item, "credentials") {
        let kind = text(credential, "type").unwrap_or("");
        match kind {
            "basic-auth" => {
                let username = editable(credential, "username").map_or("", |f| f.value);
                let password = editable(credential, "password").map_or("", |f| f.value);
                if has_login {
                    fields.add("Username", username, false);
                    fields.add("Password", password, true);
                } else {
                    data.username = username.to_string();
                    data.password = password.to_string();
                    has_login = true;
                }
            }
            "totp" => match totp_uri(credential, &title) {
                Ok(uri) if has_otp => fields.add("TOTP", &uri, true),
                Ok(uri) => {
                    data.otp = uri;
                    has_otp = true;
                }
                Err(e) => skip(import, format!("A TOTP secret: {e}")),
            },
            "passkey" => match passkey_fields(credential) {
                Ok(passkey) => passkeys.push(passkey),
                Err(e) => skip(import, format!("A passkey: {e}")),
            },
            "note" => {
                if let Some(note) = editable(credential, "content").map(|f| f.value).filter(|n| !n.is_empty()) {
                    if !data.notes.is_empty() {
                        data.notes.push_str("\n\n");
                    }
                    data.notes.push_str(note);
                }
            }
            "file" => skip(import, format!("The file {}: its content is not in the export", text(credential, "name").unwrap_or("?"))),
            "item-reference" => skip(import, "A link to another item".into()),
            "custom-fields" => {
                let label = text(credential, "label").unwrap_or("Field");
                for field in list(credential, "fields").filter_map(as_editable) {
                    fields.add(field.label.unwrap_or(label), field.value, field.concealed);
                }
            }
            "generated-password" => fields.add("Generated password", text(credential, "password").unwrap_or(""), true),
            "ssh-key" => match text(credential, "privateKey").map(|key| pem("PRIVATE KEY", key)) {
                Some(Ok(key)) => {
                    fields.add("SSH private key", &key, true);
                    fields.add("SSH key type", text(credential, "keyType").unwrap_or(""), false);
                    fields.add("SSH key comment", text(credential, "keyComment").unwrap_or(""), false);
                    other_fields(credential, kind, &["keyType", "privateKey", "keyComment"], &mut fields);
                }
                _ => skip(import, "An SSH key that is not a valid key".into()),
            },
            // Cards, addresses, Wi-Fi, documents and anything newer: their
            // values as fields, named after the kind and the value.
            _ => {
                if !other_fields(credential, kind, &[], &mut fields) {
                    skip(import, format!("Something of a kind PswManager does not know ({kind})"));
                }
            }
        }
    }

    // KeePassXC keeps one passkey per entry: a second one gets an entry of its own.
    let mut extra = Vec::new();
    for (n, passkey) in passkeys.into_iter().enumerate() {
        let rp = passkey.iter().find(|f| f.name == PASSKEY_RELYING_PARTY).map(|f| f.value.clone()).unwrap_or_default();
        let user = passkey.iter().find(|f| f.name == PASSKEY_USERNAME).map(|f| f.value.clone()).unwrap_or_default();
        if n == 0 {
            if data.url.is_empty() && !rp.is_empty() {
                data.url = format!("https://{rp}");
            }
            if data.username.is_empty() {
                data.username = user;
            }
            fields.0.extend(passkey);
        } else {
            let mut own = EntryData::default();
            own.title = if data.title.is_empty() { rp.clone() } else { data.title.clone() };
            own.group = data.group.clone();
            own.username = user;
            own.url = format!("https://{rp}");
            own.fields = passkey;
            extra.push(own);
        }
    }
    data.fields = std::mem::take(&mut fields.0);
    if data.title.is_empty() {
        data.title = url::Url::parse(&data.url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_else(|| "Untitled".into());
    }

    let created = time(item, "creationAt");
    let modified = time(item, "modifiedAt");
    let empty = data.username.is_empty() && data.password.is_empty() && data.otp.is_empty() && data.notes.is_empty() && data.fields.is_empty();
    // An item with nothing but a title or an address is kept as a bookmark.
    let bookmark = extra.is_empty() && (!title.is_empty() || !data.url.is_empty());
    if !empty || bookmark {
        import.entries.push(Imported { data, created, modified });
    } else if extra.is_empty() {
        skip(import, "An item with nothing PswManager can keep".into());
    }
    import.entries.extend(extra.into_iter().map(|data| Imported { data, created, modified }));
}

/// The fields of a credential besides `known` ones: each value as a field
/// named after the credential's kind and the value (`Credit card number`).
/// False when it had none.
fn other_fields(credential: &Value, kind: &str, known: &[&str], fields: &mut Fields) -> bool {
    let Some(members) = credential.as_object() else { return false };
    let mut any = false;
    for (member, value) in members {
        if ["type", "id", "extensions"].contains(&member.as_str()) || known.contains(&member.as_str()) {
            continue;
        }
        let name = || format!("{} {}", words(kind), words(member).to_lowercase());
        if let Some(field) = as_editable(value) {
            fields.add(&field.label.map_or_else(name, str::to_string), field.value, field.concealed);
            any = true;
        } else if let Some(text) = value.as_str() {
            fields.add(&name(), text, false);
            any = true;
        }
    }
    any
}

/// `credit-card` or `streetAddress` as words: `Credit card`, `Street address`.
fn words(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        match c {
            '-' | '_' => out.push(' '),
            c if c.is_uppercase() => {
                out.push(' ');
                out.extend(c.to_lowercase());
            }
            c => out.push(c),
        }
    }
    let mut chars = out.trim().chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// A TOTP credential as an `otpauth://` URI, as the app keeps it.
fn totp_uri(credential: &Value, title: &str) -> Result<String, String> {
    let secret = text(credential, "secret").ok_or("there is none")?;
    let issuer = text(credential, "issuer").unwrap_or(title);
    let label = match text(credential, "username") {
        Some(user) if !issuer.is_empty() => format!("{issuer}:{user}"),
        Some(user) => user.to_string(),
        None if issuer.is_empty() => "PswManager".to_string(),
        None => issuer.to_string(),
    };
    let mut uri = url::Url::parse("otpauth://totp/").expect("constant URL");
    uri.set_path(&label);
    {
        let mut query = uri.query_pairs_mut();
        query.append_pair("secret", &secret.replace([' ', '-', '='], "").to_ascii_uppercase());
        if !issuer.is_empty() {
            query.append_pair("issuer", issuer);
        }
        let number = |key| credential.get(key).and_then(Value::as_u64);
        query.append_pair("period", &number("period").unwrap_or(30).to_string());
        query.append_pair("digits", &number("digits").unwrap_or(6).to_string());
        query.append_pair("algorithm", &text(credential, "algorithm").unwrap_or("sha1").to_ascii_uppercase());
    }
    let uri = uri.to_string();
    otp::Totp::parse(&uri)?;
    Ok(uri)
}

/// A passkey as KeePassXC keeps it: the ids as unpadded base64url (as CXF
/// has them), the PKCS#8 key as PEM.
fn passkey_fields(credential: &Value) -> Result<Vec<FieldData>, String> {
    let need = |key| text(credential, key).filter(|v| !v.is_empty()).ok_or_else(|| format!("its {key} is missing"));
    let key = pem("PRIVATE KEY", need("key")?)?;
    let field = |name: &str, value: &str, protected| FieldData { name: name.into(), value: value.trim_end_matches('=').into(), protected };
    Ok(vec![
        field(PASSKEY_RELYING_PARTY, need("rpId")?, false),
        field(PASSKEY_USERNAME, text(credential, "username").unwrap_or(""), false),
        field(PASSKEY_CREDENTIAL_ID, need("credentialId")?, true),
        field(PASSKEY_USER_HANDLE, need("userHandle")?, true),
        FieldData { name: PASSKEY_PRIVATE_KEY.into(), value: key, protected: true },
    ])
}

/// base64url DER as PEM with `label`.
fn pem(label: &str, base64url: &str) -> Result<String, String> {
    let mut der = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(base64url.trim().trim_end_matches('='))
        .map_err(|_| "the key is not valid base64url".to_string())?;
    let mut body = base64::engine::general_purpose::STANDARD.encode(&der);
    der.zeroize();
    let lines: Vec<&str> = body.as_bytes().chunks(64).map(|c| std::str::from_utf8(c).expect("base64 is ASCII")).collect();
    let pem = format!("-----BEGIN {label}-----\n{}\n-----END {label}-----", lines.join("\n"));
    body.zeroize();
    Ok(pem)
}

/// Additional fields, each name used once: a taken name, or one of the
/// standard fields', gets a number (`Password (2)`).
#[derive(Default)]
struct Fields(Vec<FieldData>);

impl Fields {
    fn add(&mut self, name: &str, value: &str, protected: bool) {
        if value.is_empty() {
            return;
        }
        let base = match name.trim() {
            "" => "Field",
            name => name,
        };
        let taken = |name: &str| edit::STANDARD.contains(&name) || self.0.iter().any(|f| f.name == name);
        let mut name = base.to_string();
        for n in 2.. {
            if !taken(&name) {
                break;
            }
            name = format!("{base} ({n})");
        }
        self.0.push(FieldData { name, value: value.to_string(), protected });
    }
}

/// A CXF editable field's value.
struct Editable<'a> {
    value: &'a str,
    label: Option<&'a str>,
    concealed: bool,
}

fn as_editable(value: &Value) -> Option<Editable<'_>> {
    Some(Editable {
        value: text(value, "value")?,
        label: text(value, "label").filter(|l| !l.trim().is_empty()),
        concealed: text(value, "fieldType") == Some("concealed-string"),
    })
}

fn editable<'a>(credential: &'a Value, key: &str) -> Option<Editable<'a>> {
    credential.get(key).and_then(as_editable)
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn list<'a>(value: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    value.get(key).and_then(Value::as_array).into_iter().flatten()
}

/// A CXF time (seconds since 1970) in the file's form.
fn time(value: &Value, key: &str) -> Option<NaiveDateTime> {
    let seconds = value.get(key).and_then(Value::as_i64)?;
    DateTime::from_timestamp(seconds, 0).map(|t| t.naive_utc())
}

/// Overwrites every string in the JSON, secrets among them.
fn wipe(value: &mut Value) {
    match value {
        Value::String(s) => s.zeroize(),
        Value::Array(items) => items.iter_mut().for_each(wipe),
        Value::Object(members) => members.values_mut().for_each(wipe),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use keepass::db::fields;

    /// A made-up export: no real credentials.
    const EXPORT: &str = r#"{
      "version": {"major": 1, "minor": 0},
      "exporterRpId": "example.com",
      "exporterDisplayName": "Example Vault",
      "timestamp": 1760000000,
      "accounts": [{
        "id": "YWNjb3VudA", "username": "", "email": "user@example.com",
        "collections": [{"id": "Y29s", "title": "Work", "items": [], "subCollections": [
          {"id": "c3Vi", "title": "Mail", "items": [{"item": "aXRlbTE"}]}
        ]}],
        "items": [
          {"id": "aXRlbTE", "creationAt": 1700000000, "modifiedAt": 1710000000, "title": "Mail",
           "favorite": true, "tags": ["work"],
           "scope": {"urls": ["https://mail.example.com", "https://webmail.example.com"],
                     "androidApps": [{"bundleId": "com.example.mail"}]},
           "credentials": [
             {"type": "basic-auth",
              "username": {"fieldType": "string", "value": "alice"},
              "password": {"fieldType": "concealed-string", "value": "pw-1"}},
             {"type": "basic-auth",
              "username": {"fieldType": "string", "value": "alice2"},
              "password": {"fieldType": "concealed-string", "value": "pw-2"}},
             {"type": "totp", "secret": "JBSWY3DPEHPK3PXP", "period": 30, "digits": 6,
              "algorithm": "sha1", "issuer": "Mail", "username": "alice"},
             {"type": "note", "content": {"fieldType": "string", "value": "first"}},
             {"type": "note", "content": {"fieldType": "string", "value": "second"}},
             {"type": "custom-fields", "label": "Security", "fields": [
               {"fieldType": "concealed-string", "value": "blue", "label": "Answer"},
               {"fieldType": "string", "value": "x"}]},
             {"type": "file", "id": "Zg", "name": "scan.pdf", "decryptedSize": 10, "integrityHash": "aA"}
           ]},
          {"id": "aXRlbTI", "title": "", "credentials": [
             {"type": "passkey", "credentialId": "Y3JlZC0x", "rpId": "site.example", "username": "bob",
              "userDisplayName": "Bob", "userHandle": "dXNlcg", "key": "AAECAwQFBgcICQoLDA0ODw"},
             {"type": "passkey", "credentialId": "Y3JlZC0y", "rpId": "other.example", "username": "bob",
              "userDisplayName": "Bob", "userHandle": "dXNlcg", "key": "AAEC"}]},
          {"id": "aXRlbTM", "title": "Card", "credentials": [
             {"type": "credit-card",
              "number": {"fieldType": "concealed-string", "value": "4111"},
              "fullName": {"fieldType": "string", "value": "A B"}},
             {"type": "totp", "secret": "not base32!", "period": 30, "digits": 6, "algorithm": "sha1"},
             {"type": "something-new", "count": 3}]},
          {"id": "aXRlbTQ", "title": "", "credentials": []}
        ]
      }]
    }"#;

    fn field<'a>(data: &'a EntryData, name: &str) -> Option<&'a FieldData> {
        data.fields.iter().find(|f| f.name == name)
    }

    #[test]
    fn reads_an_item_with_logins_urls_totp_notes_and_fields() {
        let import = parse(EXPORT).unwrap();
        assert_eq!(import.exporter, "Example Vault");
        let mail = &import.entries[0];
        let data = &mail.data;
        assert_eq!((data.title.as_str(), data.username.as_str(), data.password.as_str()), ("Mail", "alice", "pw-1"));
        assert_eq!(data.group, ["Work", "Mail"]);
        assert_eq!(data.tags, ["work", FAVORITE]);
        assert_eq!(data.url, "https://mail.example.com");
        assert_eq!(field(data, "KP2A_URL_1").unwrap().value, "https://webmail.example.com");
        assert_eq!(field(data, "KP2A_URL_2").unwrap().value, "androidapp://com.example.mail");
        assert_eq!(field(data, "Username").unwrap().value, "alice2");
        let second = field(data, "Password (2)").unwrap();
        assert!(second.protected && second.value == "pw-2");
        assert_eq!(data.otp, "otpauth://totp/Mail:alice?secret=JBSWY3DPEHPK3PXP&issuer=Mail&period=30&digits=6&algorithm=SHA1");
        assert_eq!(data.notes, "first\n\nsecond");
        assert!(field(data, "Answer").unwrap().protected);
        assert!(!field(data, "Security").unwrap().protected);
        assert_eq!(mail.created, DateTime::from_timestamp(1_700_000_000, 0).map(|t| t.naive_utc()));
        assert!(import.skipped.contains(&Skipped { title: "Mail".into(), why: "The file scan.pdf: its content is not in the export".into() }));
    }

    #[test]
    fn passkeys_are_kept_as_keepassxc_keeps_them() {
        let import = parse(EXPORT).unwrap();
        let first = &import.entries[1].data;
        assert_eq!((first.title.as_str(), first.url.as_str(), first.username.as_str()), ("site.example", "https://site.example", "bob"));
        assert_eq!(field(first, PASSKEY_CREDENTIAL_ID).unwrap().value, "Y3JlZC0x");
        assert_eq!(field(first, PASSKEY_USER_HANDLE).unwrap().value, "dXNlcg");
        assert_eq!(field(first, PASSKEY_RELYING_PARTY).unwrap().value, "site.example");
        let key = field(first, PASSKEY_PRIVATE_KEY).unwrap();
        assert!(key.protected);
        assert_eq!(key.value, "-----BEGIN PRIVATE KEY-----\nAAECAwQFBgcICQoLDA0ODw==\n-----END PRIVATE KEY-----");
        // The second passkey gets an entry of its own.
        let second = &import.entries[2].data;
        assert_eq!((second.title.as_str(), second.url.as_str()), ("other.example", "https://other.example"));
        assert_eq!(field(second, PASSKEY_CREDENTIAL_ID).unwrap().value, "Y3JlZC0y");
    }

    #[test]
    fn other_kinds_become_fields_and_what_cannot_be_kept_is_listed() {
        let import = parse(EXPORT).unwrap();
        let card = &import.entries[3].data;
        assert!(field(card, "Credit card number").unwrap().protected);
        assert_eq!(field(card, "Credit card full name").unwrap().value, "A B");
        assert!(card.otp.is_empty());
        let why: Vec<&str> = import.skipped.iter().filter(|s| s.title == "Card").map(|s| s.why.as_str()).collect();
        assert_eq!(why.len(), 2, "{why:?}");
        assert!(why[0].starts_with("A TOTP secret"));
        assert!(why[1].contains("something-new"));
        // The empty item is not an entry.
        assert_eq!(import.entries.len(), 4);
        assert!(import.skipped.iter().any(|s| s.title.is_empty() && s.why.contains("nothing")));
    }

    #[test]
    fn refuses_what_is_not_a_cxf_1_export() {
        assert!(parse("not json").is_err());
        assert!(parse(r#"{"version": {"major": 2, "minor": 0}, "accounts": []}"#).is_err());
        assert!(parse(r#"{"version": {"major": 1, "minor": 0}}"#).is_err());
    }

    #[test]
    fn several_accounts_each_get_a_group() {
        let json = r#"{"version": {"major": 1, "minor": 0}, "exporterRpId": "x", "accounts": [
          {"id": "YQ", "username": "ann", "email": "", "collections": [], "items": [{"id": "MQ", "title": "One", "credentials": []}]},
          {"id": "Yg", "username": "", "email": "ben@example.com", "collections": [], "items": [{"id": "Mg", "title": "Two", "credentials": []}]}]}"#;
        let import = parse(json).unwrap();
        assert_eq!(import.exporter, "x");
        assert_eq!(import.entries[0].data.group, ["ann"]);
        assert_eq!(import.entries[1].data.group, ["ben@example.com"]);
    }

    #[test]
    fn adds_new_entries_under_the_import_group() {
        let import = parse(EXPORT).unwrap();
        let mut db = Database::new();
        let day = NaiveDate::from_ymd_opt(2026, 10, 4).unwrap();
        let group = import.group_name(day);
        assert_eq!(group, "Imported from Example Vault 2026-10-04");
        assert_eq!(add(&mut db, &import, std::slice::from_ref(&group), &HashSet::new()).unwrap(), 4);
        let mail = db.iter_all_entries().find(|e| e.get(fields::TITLE) == Some("Mail")).unwrap();
        assert_eq!(edit::path_of(&db, mail.parent().id()), [group.as_str(), "Work", "Mail"]);
        assert_eq!(mail.times.last_modification, DateTime::from_timestamp(1_710_000_000, 0).map(|t| t.naive_utc()));
        assert!(mail.fields[fields::PASSWORD].is_protected());
        assert_eq!(db.iter_all_entries().count(), 4);
    }
}
