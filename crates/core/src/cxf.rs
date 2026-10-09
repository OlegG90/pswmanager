//! Exchanging entries with another password manager in the FIDO Credential
//! Exchange Format (CXF 1.0), as Android's Credential Transfer hands it over
//! (see `docs/cxp-research.md` for the mapping). Importing reads an export
//! into new entries: nothing in the database is changed or merged. Exporting
//! writes the entries in use as one account, the reverse of the same mapping.

use crate::edit::{self, passkey, EntryData, FieldData, FAVORITE};
use crate::otp;
use base64::Engine;
use chrono::{DateTime, NaiveDate, NaiveDateTime};
use keepass::db::{fields as standard, EntryRef, GroupId};
use keepass::Database;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use zeroize::{Zeroize, Zeroizing};

/// What an export holds, read and ready to add.
pub struct Import {
    /// The app it came from, as it names itself.
    pub exporter: String,
    /// The entries to add, in the export's order.
    pub entries: Vec<Imported>,
    /// What could not be brought over, and why.
    pub skipped: Vec<Skipped>,
}

/// An entry to add, with the times the other app kept for it.
pub struct Imported {
    /// The entry, its group the collection it was in (below the import's group).
    pub data: EntryData,
    pub created: Option<NaiveDateTime>,
    pub modified: Option<NaiveDateTime>,
}

/// Something in the export that no entry holds.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Skipped {
    /// The item's title.
    pub title: String,
    /// Why it was left out.
    pub why: String,
}

impl Import {
    /// The group the entries go in: `Imported from <app> <date>`.
    pub fn group_name(&self, day: NaiveDate) -> String {
        format!("Imported from {} {}", self.exporter, day.format("%Y-%m-%d"))
    }

    /// [Import::group_name] for today, as this device's clock has it.
    pub fn group_name_today(&self) -> String {
        self.group_name(chrono::Local::now().date_naive())
    }
}


/// Reads a CXF 1.x export. The strings of the parsed JSON are wiped once
/// read (not the parser's own buffers, which cannot be reached); the caller
/// wipes `json` itself.
pub fn parse(json: &str) -> Result<Import, String> {
    let mut root: Value = serde_json::from_str(json).map_err(|e| format!("Not a Credential Exchange export: {e}"))?;
    let import = read_header(&root);
    wipe(&mut root);
    import
}

/// Adds the entries under the group at `under` (made where missing), with the
/// collections they were in as groups below it. Returns how many were added.
/// All or nothing: on an error the caller drops the changed database
/// ([crate::vault::Vault::import] works on a copy).
pub fn add(db: &mut Database, import: &Import, under: &[String], hidden: &HashSet<GroupId>) -> Result<usize, String> {
    for imported in &import.entries {
        let mut data = imported.data.clone();
        data.group = under.iter().chain(&imported.data.group).cloned().collect();
        let id = edit::apply(db, None, &data, hidden)?;
        let mut entry = db.entry_mut(id).ok_or("The new entry is gone")?;
        if let Some(created) = imported.created {
            entry.times.creation = Some(created);
        }
        if let Some(modified) = imported.modified {
            entry.times.last_modification = Some(modified);
        }
    }
    Ok(import.entries.len())
}

/// Who the exports are from, as the importing app shows it.
pub const EXPORTER_NAME: &str = "PswManager";
/// The exporter's id in an export (CXF asks for a domain).
pub const EXPORTER_ID: &str = "olegg90.github.io";

/// `entries` (the entries in use) as a CXF 1.0 export: one account named
/// `account` (the database), its groups as collections. What CXF cannot
/// carry stays behind: history, icons, the expiry and files. The JSON holds
/// every secret in clear: it is wiped when dropped, and must never be logged
/// or written where the user can see it.
pub fn export<'a>(entries: impl Iterator<Item = EntryRef<'a>>, account: &str, now: i64) -> Zeroizing<String> {
    let mut items = Vec::new();
    let mut collections = Collections::default();
    for entry in entries {
        let id = b64(entry.id().uuid().as_bytes());
        collections.add(&edit::path_of(entry.database(), entry.parent().id()), &id);
        items.push(write_item(&entry, &id));
    }
    let mut root = json!({
        "version": {"major": 1, "minor": 0},
        "exporterRpId": EXPORTER_ID,
        "exporterDisplayName": EXPORTER_NAME,
        "timestamp": now,
        "accounts": [{
            "id": b64(account.as_bytes()),
            "username": "",
            "email": "",
            "fullName": account,
            "collections": collections.into_json(),
            "items": items,
        }],
    });
    let json = Zeroizing::new(root.to_string());
    wipe(&mut root);
    json
}

/// An entry as a CXF item, its id `id`.
fn write_item(entry: &EntryRef<'_>, id: &str) -> Value {
    let text = |name: &str| entry.get(name).unwrap_or("").to_string();
    let title = text(standard::TITLE);
    let mut item = Map::new();
    item.insert("id".into(), id.into());
    let seconds = |t: Option<NaiveDateTime>| t.map(|t| t.and_utc().timestamp());
    if let Some(at) = seconds(entry.times.creation) {
        item.insert("creationAt".into(), at.into());
    }
    if let Some(at) = seconds(entry.times.last_modification) {
        item.insert("modifiedAt".into(), at.into());
    }
    item.insert("title".into(), title.clone().into());
    item.insert("favorite".into(), entry.tags.iter().any(|t| t == FAVORITE).into());
    let tags: Vec<&String> = entry.tags.iter().filter(|t| *t != FAVORITE).collect();
    if !tags.is_empty() {
        item.insert("tags".into(), json!(tags));
    }

    // The entry's URL and its further ones (`KP2A_URL_n`); apps apart.
    let mut urls = Vec::new();
    let mut apps = Vec::new();
    let further = entry.fields.iter().filter(|(name, _)| name.starts_with(MORE_URLS)).map(|(_, v)| v.get().trim().to_string());
    for url in std::iter::once(text(standard::URL).trim().to_string()).chain(further).filter(|u| !u.is_empty()) {
        match url.strip_prefix("androidapp://") {
            Some(app) => apps.push(json!({"bundleId": app})),
            None => urls.push(Value::from(url)),
        }
    }
    if !urls.is_empty() || !apps.is_empty() {
        item.insert("scope".into(), json!({"urls": urls, "androidApps": apps}));
    }

    let mut credentials = Vec::new();
    // What has no CXF credential of its own (a TOTP secret or a passkey this
    // app cannot read) goes along as fields rather than be left behind.
    let mut unread: Vec<Value> = Vec::new();
    let concealed = |label: &str, value: &str| json!({"fieldType": "concealed-string", "value": value, "label": label});
    let (username, password) = (text(standard::USERNAME), text(standard::PASSWORD));
    if !username.is_empty() || !password.is_empty() {
        credentials.push(json!({
            "type": "basic-auth",
            "username": {"fieldType": "string", "value": username},
            "password": {"fieldType": "concealed-string", "value": password},
        }));
    }
    if let Some(otp) = entry.get(standard::OTP).filter(|o| !o.trim().is_empty()) {
        match write_totp(otp, &title) {
            Some(totp) => credentials.push(totp),
            None => unread.push(concealed("TOTP", otp)),
        }
    }
    let passkey = write_passkey(entry);
    let passkey_written = passkey.is_some();
    if let Some(passkey) = passkey {
        credentials.push(passkey);
    }
    let notes = text(standard::NOTES);
    if !notes.is_empty() {
        credentials.push(json!({"type": "note", "content": {"fieldType": "string", "value": notes}}));
    }
    let mut others: Vec<(&String, Value)> = entry
        .fields
        .iter()
        .filter(|(name, value)| {
            let written = passkey_written && name.starts_with(passkey::PREFIX);
            !edit::STANDARD.contains(&name.as_str()) && !written && !name.starts_with(MORE_URLS) && !value.get().is_empty()
        })
        .map(|(name, value)| {
            let kind = if value.is_protected() { "concealed-string" } else { "string" };
            (name, json!({"fieldType": kind, "value": value.get(), "label": name}))
        })
        .collect();
    others.sort_by_key(|(name, _)| name.to_lowercase());
    let fields: Vec<Value> = unread.into_iter().chain(others.into_iter().map(|(_, f)| f)).collect();
    if !fields.is_empty() {
        credentials.push(json!({"type": "custom-fields", "fields": fields}));
    }
    item.insert("credentials".into(), credentials.into());
    Value::Object(item)
}

/// Further URLs as Keepass2Android keeps them: `KP2A_URL_1`, `KP2A_URL_2`…
const MORE_URLS: &str = "KP2A_URL";

/// The entry's TOTP secret (an `otpauth://` URI or a bare secret) as a CXF
/// TOTP credential; `None` when it is not one this app can read.
fn write_totp(otp: &str, title: &str) -> Option<Value> {
    otp::Totp::parse(otp).ok()?;
    let otp = otp.trim();
    let mut totp = json!({"type": "totp", "period": 30, "digits": 6, "algorithm": "sha1"});
    if !otp.to_ascii_lowercase().starts_with("otpauth:") {
        totp["secret"] = otp.chars().filter(|c| !c.is_whitespace() && *c != '-' && *c != '=').collect::<String>().to_ascii_uppercase().into();
        totp["issuer"] = title.into();
        return Some(totp);
    }
    let uri = url::Url::parse(otp).ok()?;
    // The label is `issuer:user` or just a name.
    let label = percent_decode(uri.path().trim_start_matches('/'));
    let (label_issuer, user) = match label.split_once(':') {
        Some((issuer, user)) => (Some(issuer.trim().to_string()), user.trim().to_string()),
        None => (None, label.trim().to_string()),
    };
    let mut issuer = label_issuer;
    for (key, value) in uri.query_pairs() {
        match key.to_ascii_lowercase().as_str() {
            "secret" => totp["secret"] = value.replace([' ', '-', '='], "").to_ascii_uppercase().into(),
            "issuer" => issuer = Some(value.into_owned()),
            "period" => totp["period"] = value.parse::<u64>().ok()?.into(),
            "digits" => totp["digits"] = value.parse::<u64>().ok()?.into(),
            "algorithm" => totp["algorithm"] = value.to_ascii_lowercase().into(),
            _ => {}
        }
    }
    if let Some(issuer) = issuer.filter(|i| !i.is_empty()) {
        totp["issuer"] = issuer.into();
    }
    if !user.is_empty() {
        totp["username"] = user.into();
    }
    Some(totp)
}

/// `%xx` escapes in a URI's path, as a label has them (`AT%26T:bob`).
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The passkey KeePassXC keeps in the entry's attributes as a CXF passkey;
/// `None` without one, or with one missing a part or a key this app cannot
/// read (its attributes then go along as fields).
fn write_passkey(entry: &EntryRef<'_>) -> Option<Value> {
    let get = |name: &str| entry.get(name).filter(|v| !v.is_empty());
    let key = Zeroizing::new(b64(&crate::webauthn::pkcs8_der(get(passkey::PRIVATE_KEY)?)?));
    let username = get(passkey::USERNAME).unwrap_or("");
    Some(json!({
        "type": "passkey",
        "credentialId": get(passkey::CREDENTIAL_ID)?,
        "rpId": get(passkey::RELYING_PARTY)?,
        "username": username,
        "userDisplayName": username,
        "userHandle": get(passkey::USER_HANDLE)?,
        "key": key.as_str(),
    }))
}

/// Unpadded base64url, as CXF has ids and keys.
fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// The groups as CXF collections: each with the items right in it and its
/// subgroups as sub-collections. Entries at the top are in none.
#[derive(Default)]
struct Collections(Vec<Collection>);

struct Collection {
    title: String,
    items: Vec<String>,
    below: Collections,
}

impl Collections {
    /// Puts item `id` in the collection at `path` (made where missing).
    fn add(&mut self, path: &[String], id: &str) {
        let Some((first, rest)) = path.split_first() else { return };
        let at = match self.0.iter().position(|c| c.title == *first) {
            Some(at) => at,
            None => {
                self.0.push(Collection { title: first.clone(), items: Vec::new(), below: Collections::default() });
                self.0.len() - 1
            }
        };
        let collection = &mut self.0[at];
        if rest.is_empty() {
            collection.items.push(id.to_string());
        } else {
            collection.below.add(rest, id);
        }
    }

    fn into_json(self) -> Vec<Value> {
        self.into_json_under("")
    }

    /// Each collection's id is its path, so ids differ across the tree.
    fn into_json_under(self, above: &str) -> Vec<Value> {
        self.0
            .into_iter()
            .map(|c| {
                let path = format!("{above}/{}", c.title);
                json!({
                    "id": b64(path.as_bytes()),
                    "title": c.title,
                    "items": c.items.iter().map(|item| json!({"item": item})).collect::<Vec<_>>(),
                    "subCollections": c.below.into_json_under(&path),
                })
            })
            .collect()
    }
}

/// The export's header: its version, who made it, and every account's items.
fn read_header(root: &Value) -> Result<Import, String> {
    let major = root.get("version").and_then(|v| v.get("major")).and_then(Value::as_u64);
    if major != Some(1) {
        return Err("This export is in a Credential Exchange version PswManager cannot read".into());
    }
    let exporter = first_text(root, &["exporterDisplayName", "exporterRpId"]).unwrap_or("another app").to_string();
    let accounts = root.get("accounts").and_then(Value::as_array).ok_or("The export holds no accounts")?;
    let mut import = Import { exporter, entries: Vec::new(), skipped: Vec::new() };
    // Several accounts (a family's, say) each get a group of their own.
    for account in accounts {
        let top: Vec<String> = if accounts.len() > 1 {
            vec![first_text(account, &["fullName", "username", "email"]).unwrap_or("Account").to_string()]
        } else {
            Vec::new()
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

/// An item as an entry (and one more per further passkey), with what it
/// held that no entry can noted in `import.skipped`.
fn read_item(item: &Value, group: Vec<String>, import: &mut Import) {
    let title = text(item, "title").unwrap_or("").trim().to_string();
    let mut data = EntryData::default();
    data.title = title.clone();
    data.group = group;
    data.tags = list(item, "tags").filter_map(Value::as_str).map(str::to_string).collect();
    if item.get("favorite").and_then(Value::as_bool) == Some(true) && !data.tags.iter().any(|t| t == FAVORITE) {
        data.tags.push(FAVORITE.to_string());
    }
    let mut fields = Fields::default();
    read_scope(item, &mut data, &mut fields);
    let mut why = Vec::new();
    let passkeys = read_credentials(item, &mut data, &mut fields, &mut why);
    let extra = place_passkeys(passkeys, &mut data, &mut fields);
    data.fields = fields.0;
    if data.title.is_empty() {
        data.title = crate::icons::host_of(&data.url).unwrap_or_else(|| "Untitled".into());
    }

    let created = time(item, "creationAt");
    let modified = time(item, "modifiedAt");
    let empty = data.username.is_empty() && data.password.is_empty() && data.otp.is_empty() && data.notes.is_empty() && data.fields.is_empty();
    // An item with nothing but a title or an address is kept as a bookmark.
    if !empty || extra.is_empty() && (!title.is_empty() || !data.url.is_empty()) {
        import.entries.push(Imported { data, created, modified });
    } else if extra.is_empty() {
        why.push("An item with nothing PswManager can keep".into());
    }
    import.entries.extend(extra.into_iter().map(|data| Imported { data, created, modified }));
    import.skipped.extend(why.into_iter().map(|why| Skipped { title: title.clone(), why }));
}

/// The item's addresses: the first is the entry's URL; more, and Android
/// apps, go in fields as Keepass2Android keeps them (an app is never the
/// entry's own URL, which is opened in a browser).
fn read_scope(item: &Value, data: &mut EntryData, fields: &mut Fields) {
    let scope = item.get("scope");
    let mut urls = scope.into_iter().flat_map(|s| list(s, "urls")).filter_map(Value::as_str).map(str::to_string);
    data.url = urls.next().unwrap_or_default();
    let apps = scope.into_iter().flat_map(|s| list(s, "androidApps")).filter_map(|a| text(a, "bundleId")).map(|id| format!("androidapp://{id}"));
    for (n, url) in urls.chain(apps).enumerate() {
        fields.add(&format!("KP2A_URL_{}", n + 1), &url, false);
    }
}

/// The item's credentials into the entry; returns its passkeys, and notes in
/// `why` what could not be kept.
fn read_credentials(item: &Value, data: &mut EntryData, fields: &mut Fields, why: &mut Vec<String>) -> Vec<Passkey> {
    let (mut has_login, mut has_otp) = (false, false);
    let mut passkeys = Vec::new();
    for credential in list(item, "credentials") {
        let kind = text(credential, "type").unwrap_or("");
        match kind {
            "basic-auth" => {
                let username = editable(credential, "username").map_or("", |f| f.value);
                let password = editable(credential, "password").map_or("", |f| f.value);
                if has_login {
                    // Numbered after the standard fields: `UserName (2)`, `Password (2)`.
                    fields.add(keepass::db::fields::USERNAME, username, false);
                    fields.add(keepass::db::fields::PASSWORD, password, true);
                } else {
                    data.username = username.to_string();
                    data.password = password.to_string();
                    has_login = true;
                }
            }
            "totp" => match totp_uri(credential, &data.title) {
                Ok(uri) if has_otp => fields.add("TOTP", &uri, true),
                Ok(uri) => {
                    data.otp = uri;
                    has_otp = true;
                }
                Err(e) => why.push(format!("A TOTP secret: {e}")),
            },
            "passkey" => match Passkey::read(credential) {
                Ok(passkey) => {
                    // What KeePassXC has no attributes for (PRF and the blobs).
                    let extensions: Vec<&str> = credential
                        .get("fido2Extensions")
                        .and_then(Value::as_object)
                        .map(|e| e.keys().map(String::as_str).filter(|k| *k != "payments").collect())
                        .unwrap_or_default();
                    if !extensions.is_empty() {
                        why.push(format!("The passkey's extensions ({}): not kept", extensions.join(", ")));
                    }
                    passkeys.push(passkey);
                }
                Err(e) => why.push(format!("A passkey: {e}")),
            },
            "note" => {
                if let Some(note) = editable(credential, "content").map(|f| f.value).filter(|n| !n.is_empty()) {
                    if !data.notes.is_empty() {
                        data.notes.push_str("\n\n");
                    }
                    data.notes.push_str(note);
                }
            }
            "file" => why.push(format!("The file {}: its content is not in the export", text(credential, "name").unwrap_or("?"))),
            "item-reference" => why.push("A link to another item".into()),
            "custom-fields" => {
                let label = text(credential, "label").unwrap_or("Field");
                for field in list(credential, "fields").filter_map(as_editable) {
                    fields.add(field.label.unwrap_or(label), field.value, field.concealed);
                }
            }
            "generated-password" => fields.add("Generated password", text(credential, "password").unwrap_or(""), true),
            "ssh-key" => match text(credential, "privateKey").map(pem) {
                Some(Ok(key)) => {
                    fields.add("SSH private key", &key, true);
                    fields.add("SSH key type", text(credential, "keyType").unwrap_or(""), false);
                    fields.add("SSH key comment", text(credential, "keyComment").unwrap_or(""), false);
                    other_fields(credential, kind, &["keyType", "privateKey", "keyComment"], fields);
                }
                _ => why.push("An SSH key that is not a valid key".into()),
            },
            // Cards, addresses, Wi-Fi, documents and anything newer: their
            // values as fields, named after the kind and the value.
            _ => {
                if !other_fields(credential, kind, &[], fields) {
                    why.push(format!("Something of a kind PswManager does not know ({kind})"));
                }
            }
        }
    }
    passkeys
}

/// The first passkey goes in the entry (its site and user name filling in
/// what the item lacks); KeePassXC keeps one passkey per entry, so each
/// further one is returned as an entry of its own.
fn place_passkeys(passkeys: Vec<Passkey>, data: &mut EntryData, fields: &mut Fields) -> Vec<EntryData> {
    let mut passkeys = passkeys.into_iter();
    if let Some(first) = passkeys.next() {
        if data.url.is_empty() {
            data.url = first.site();
        }
        if data.username.is_empty() {
            data.username = first.username.clone();
        }
        // No other field takes a passkey's name (see [Fields::add]).
        fields.0.extend(first.fields);
    }
    passkeys
        .map(|passkey| {
            let mut own = EntryData::default();
            own.title = if data.title.is_empty() { passkey.rp.clone() } else { data.title.clone() };
            own.group = data.group.clone();
            own.tags = data.tags.clone();
            own.username = passkey.username.clone();
            own.url = passkey.site();
            own.fields = passkey.fields;
            own
        })
        .collect()
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
    let secret = Zeroizing::new(secret.replace([' ', '-', '='], "").to_ascii_uppercase());
    let mut uri = url::Url::parse("otpauth://totp/").expect("constant URL");
    uri.set_path(&label);
    {
        let mut query = uri.query_pairs_mut();
        query.append_pair("secret", &secret);
        if !issuer.is_empty() {
            query.append_pair("issuer", issuer);
        }
        let number = |key| credential.get(key).and_then(Value::as_u64);
        query.append_pair("period", &number("period").unwrap_or(30).to_string());
        query.append_pair("digits", &number("digits").unwrap_or(6).to_string());
        query.append_pair("algorithm", &text(credential, "algorithm").unwrap_or("sha1").to_ascii_uppercase());
    }
    let uri = Zeroizing::new(uri.to_string());
    otp::Totp::parse(&uri)?;
    Ok(uri.to_string())
}

/// A passkey: its relying party and user name, and the attributes KeePassXC
/// keeps it in.
struct Passkey {
    rp: String,
    username: String,
    fields: Vec<FieldData>,
}

impl Passkey {
    /// A CXF passkey as KeePassXC keeps it: the ids as unpadded base64url (as
    /// CXF has them), the PKCS#8 key as PEM.
    fn read(credential: &Value) -> Result<Passkey, String> {
        let need = |key| text(credential, key).filter(|v| !v.is_empty()).ok_or_else(|| format!("its {key} is missing"));
        let (rp, credential_id, user_handle) = (need("rpId")?, need("credentialId")?, need("userHandle")?);
        let username = text(credential, "username").unwrap_or("");
        let key = pem(need("key")?)?;
        Ok(Passkey { rp: rp.to_string(), username: username.to_string(), fields: passkey::fields(rp, username, credential_id, user_handle, &key) })
    }

    /// The relying party's site.
    fn site(&self) -> String {
        format!("https://{}", self.rp)
    }
}

/// A base64url PKCS#8 key (DER) as PEM.
fn pem(base64url: &str) -> Result<Zeroizing<String>, String> {
    let der = Zeroizing::new(
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(base64url.trim().trim_end_matches('='))
            .map_err(|_| "the key is not valid base64url".to_string())?,
    );
    let body = Zeroizing::new(base64::engine::general_purpose::STANDARD.encode(&*der));
    let lines: Vec<&str> = body.as_bytes().chunks(64).map(|c| std::str::from_utf8(c).expect("base64 is ASCII")).collect();
    Ok(Zeroizing::new(format!("-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----", lines.join("\n"))))
}

/// Additional fields, each name used once: a taken name, a standard field's,
/// or one a passkey's attributes use, gets a number (`Password (2)`).
#[derive(Default)]
struct Fields(Vec<FieldData>);

impl Fields {
    /// Adds a field, unless `value` is empty.
    fn add(&mut self, name: &str, value: &str, protected: bool) {
        if value.is_empty() {
            return;
        }
        let base = match name.trim() {
            "" => "Field".to_string(),
            name if name.starts_with(passkey::PREFIX) => format!("Imported {name}"),
            name => name.to_string(),
        };
        let taken = |name: &str| edit::STANDARD.contains(&name) || self.0.iter().any(|f| f.name == name);
        let mut name = base.clone();
        for n in 2.. {
            if !taken(&name) {
                break;
            }
            name = format!("{base} ({n})");
        }
        self.0.push(FieldData { name, value: value.to_string(), protected });
    }
}

/// A CXF editable field: a value with a type and perhaps a label.
struct Editable<'a> {
    value: &'a str,
    /// The label the user gave it, if any.
    label: Option<&'a str>,
    /// A hidden value (a password, a card number): stored protected.
    concealed: bool,
}

/// `value` as an editable field, if it is one.
fn as_editable(value: &Value) -> Option<Editable<'_>> {
    Some(Editable {
        value: text(value, "value")?,
        label: first_text(value, &["label"]),
        concealed: text(value, "fieldType") == Some("concealed-string"),
    })
}

/// The editable field `key` of a credential.
fn editable<'a>(credential: &'a Value, key: &str) -> Option<Editable<'a>> {
    credential.get(key).and_then(as_editable)
}

/// The string member `key`.
fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// The first of the string members `keys` that is not blank.
fn first_text<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| text(value, k).filter(|v| !v.trim().is_empty()))
}

/// The members of the array `key`; none when it is missing.
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
          {"id": "aXRlbTI", "title": "", "tags": ["keys"], "scope": {"androidApps": [{"bundleId": "com.example.site"}]}, "credentials": [
             {"type": "custom-fields", "fields": [{"fieldType": "string", "value": "mine", "label": "KPEX_PASSKEY_USERNAME"}]},
             {"type": "passkey", "credentialId": "Y3JlZC0x", "rpId": "site.example", "username": "bob",
              "userDisplayName": "Bob", "userHandle": "dXNlcg", "key": "AAECAwQFBgcICQoLDA0ODw",
              "fido2Extensions": {"payments": false, "hmacCredentials": {"algorithm": "hmac-sha256", "credWithUV": "AA", "credWithoutUV": "AA"}}},
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
        assert_eq!(field(data, "UserName (2)").unwrap().value, "alice2");
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
        assert_eq!(field(first, passkey::CREDENTIAL_ID).unwrap().value, "Y3JlZC0x");
        assert_eq!(field(first, passkey::USER_HANDLE).unwrap().value, "dXNlcg");
        assert_eq!(field(first, passkey::RELYING_PARTY).unwrap().value, "site.example");
        assert_eq!(field(first, passkey::USERNAME).unwrap().value, "bob");
        // A field of the same name as a passkey attribute does not clash with it.
        assert_eq!(field(first, "Imported KPEX_PASSKEY_USERNAME").unwrap().value, "mine");
        // An Android app is never the entry's URL.
        assert_eq!(field(first, "KP2A_URL_1").unwrap().value, "androidapp://com.example.site");
        let key = field(first, passkey::PRIVATE_KEY).unwrap();
        assert!(key.protected);
        assert_eq!(key.value, "-----BEGIN PRIVATE KEY-----\nAAECAwQFBgcICQoLDA0ODw==\n-----END PRIVATE KEY-----");
        // The second passkey gets an entry of its own.
        let second = &import.entries[2].data;
        assert_eq!((second.title.as_str(), second.url.as_str()), ("other.example", "https://other.example"));
        assert_eq!(field(second, passkey::CREDENTIAL_ID).unwrap().value, "Y3JlZC0y");
        // PRF and the blobs have nowhere to go; a payments flag says nothing.
        assert!(import.skipped.iter().any(|s| s.why == "The passkey's extensions (hmacCredentials): not kept"));
        assert_eq!(second.tags, ["keys"]);
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

    fn entry_data(title: &str) -> EntryData {
        let mut data = EntryData::default();
        data.title = title.into();
        data
    }

    #[test]
    fn an_export_reads_back_as_the_entries() {
        let mut db = Database::new();
        let hidden = edit::hidden_groups(&db);
        let mut mail = entry_data("Mail");
        mail.username = "alice".into();
        mail.password = "pw-1".into();
        mail.url = "https://mail.example.com".into();
        mail.notes = "first line\nsecond".into();
        mail.otp = "otpauth://totp/Mail:alice?secret=JBSWY3DPEHPK3PXP&issuer=Mail&period=30&digits=6&algorithm=SHA1".into();
        mail.tags = vec!["work".into(), FAVORITE.into()];
        mail.group = vec!["Work".into(), "Mail".into()];
        mail.fields = vec![
            FieldData { name: "KP2A_URL_1".into(), value: "https://webmail.example.com".into(), protected: false },
            FieldData { name: "KP2A_URL_2".into(), value: "androidapp://com.example.mail".into(), protected: false },
            FieldData { name: "Answer".into(), value: "blue".into(), protected: true },
        ];
        edit::apply(&mut db, None, &mail, &hidden).unwrap();
        let mut key = entry_data("Site");
        key.fields = passkey::fields("site.example", "bob", "Y3JlZC0x", "dXNlcg", &p256_pem(false));
        edit::apply(&mut db, None, &key, &hidden).unwrap();

        let json = export(db.iter_all_entries(), "Personal", 1_760_000_000);
        let import = parse(&json).unwrap();
        assert_eq!(import.exporter, EXPORTER_NAME);
        assert!(import.skipped.is_empty(), "{:?}", import.skipped);
        let mail = &import.entries.iter().find(|e| e.data.title == "Mail").unwrap().data;
        assert_eq!((mail.username.as_str(), mail.password.as_str(), mail.url.as_str()), ("alice", "pw-1", "https://mail.example.com"));
        assert_eq!(mail.notes, "first line\nsecond");
        assert_eq!(mail.otp, "otpauth://totp/Mail:alice?secret=JBSWY3DPEHPK3PXP&issuer=Mail&period=30&digits=6&algorithm=SHA1");
        assert_eq!(mail.tags, ["work", FAVORITE]);
        assert_eq!(mail.group, ["Work", "Mail"]);
        assert_eq!(field(mail, "KP2A_URL_1").unwrap().value, "https://webmail.example.com");
        assert_eq!(field(mail, "KP2A_URL_2").unwrap().value, "androidapp://com.example.mail");
        assert!(field(mail, "Answer").unwrap().protected);
        let site = &import.entries.iter().find(|e| e.data.title == "Site").unwrap().data;
        assert_eq!(field(site, passkey::CREDENTIAL_ID).unwrap().value, "Y3JlZC0x");
        assert_eq!(crate::webauthn::pkcs8_der(&field(site, passkey::PRIVATE_KEY).unwrap().value), crate::webauthn::pkcs8_der(&p256_pem(false)));
        assert_eq!((site.url.as_str(), site.username.as_str()), ("https://site.example", "bob"));
    }

    /// A real P-256 key, made for the test.
    fn p256_pem(one_line: bool) -> String {
        use p256::pkcs8::{EncodePrivateKey, LineEnding};
        let key = p256::ecdsa::SigningKey::from_slice(&[7u8; 32]).unwrap();
        let pem = key.to_pkcs8_pem(LineEnding::LF).unwrap().to_string();
        if one_line { pem.replace('\n', "") } else { pem }
    }

    #[test]
    fn a_passkey_kept_on_one_line_is_exported_with_its_key() {
        let mut db = Database::new();
        let hidden = edit::hidden_groups(&db);
        let mut site = entry_data("webauthn.io");
        site.fields = passkey::fields("webauthn.io", "me", "Y3JlZA", "dXNlcg", &p256_pem(true));
        edit::apply(&mut db, None, &site, &hidden).unwrap();
        let json = export(db.iter_all_entries(), "Personal", 0);
        let root: Value = serde_json::from_str(&json).unwrap();
        let credentials = &root["accounts"][0]["items"][0]["credentials"];
        let passkey = credentials.as_array().unwrap().iter().find(|c| c["type"] == "passkey").expect("the passkey");
        assert!(passkey["key"].as_str().unwrap().len() > 100, "{passkey}");
        // Read back: the same key, wrapped in lines as the import writes it.
        let import = parse(&json).unwrap();
        let key = field(&import.entries[0].data, passkey::PRIVATE_KEY).unwrap();
        assert_eq!(crate::webauthn::pkcs8_der(&key.value), crate::webauthn::pkcs8_der(&p256_pem(false)));
    }

    #[test]
    fn what_cannot_be_read_goes_along_as_fields() {
        let mut db = Database::new();
        let hidden = edit::hidden_groups(&db);
        let mut odd = entry_data("Odd");
        odd.fields = vec![FieldData { name: passkey::CREDENTIAL_ID.into(), value: "Y3JlZA".into(), protected: true }];
        let id = edit::apply(&mut db, None, &odd, &hidden).unwrap();
        // A TOTP value this app cannot read, kept as the file has it.
        db.entry_mut(id).unwrap().set_protected(standard::OTP, "not base32!");
        let import = parse(&export(db.iter_all_entries(), "Personal", 0)).unwrap();
        let data = &import.entries[0].data;
        assert_eq!(field(data, "TOTP").unwrap().value, "not base32!");
        assert!(field(data, "TOTP").unwrap().protected);
        // A passkey missing its parts: its attribute comes back, renamed apart.
        assert_eq!(field(data, &format!("Imported {}", passkey::CREDENTIAL_ID)).unwrap().value, "Y3JlZA");
    }

    #[test]
    fn a_totp_label_keeps_escaped_characters() {
        let totp = write_totp("otpauth://totp/AT%26T:bob%40mail?secret=JBSWY3DPEHPK3PXP", "x").unwrap();
        assert_eq!(totp["issuer"], "AT&T");
        assert_eq!(totp["username"], "bob@mail");
        // The issuer parameter wins over the label's.
        let totp = write_totp("otpauth://totp/Old:bob?secret=JBSWY3DPEHPK3PXP&issuer=New", "x").unwrap();
        assert_eq!(totp["issuer"], "New");
    }

    #[test]
    fn a_bare_totp_secret_is_exported_with_the_title_as_issuer() {
        let totp = write_totp("jbsw y3dp ehpk 3pxp", "Bank").unwrap();
        assert_eq!(totp["secret"], "JBSWY3DPEHPK3PXP");
        assert_eq!(totp["issuer"], "Bank");
        assert!(write_totp("not base32!", "Bank").is_none());
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
