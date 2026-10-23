//! Importing an Aegis export (#282): the vault JSON of the Android
//! authenticator, plain or encrypted with a password, in its documented
//! format (`docs/vault.md` in its repository; no code of Aegis, which is
//! GPLv3). Each token becomes an entry with its TOTP, through the same
//! [Import] as the other file imports.
//!
//! An encrypted export: the vault's master key is wrapped in slots; a
//! password slot derives the key that unwraps it with scrypt, and both the
//! slot and the vault are AES-256-GCM. Keys and the decrypted vault are wiped.

use crate::cxf::{self, list, text, Fields, Import};
use crate::edit::{EntryData, IconChoice, FAVORITE};
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use serde_json::Value;
use std::collections::HashMap;
use zeroize::Zeroizing;

/// An Aegis slot holding the master key under a key derived from a password.
const PASSWORD_SLOT: u64 = 1;

/// Whether `root` is an Aegis export (`{version, header, db}`).
pub(crate) fn is_export(root: &Value) -> bool {
    root.get("header").is_some_and(Value::is_object) && root.get("db").is_some() && root.get("version").is_some()
}

/// Whether the export is encrypted: its password is needed to read it.
pub(crate) fn is_encrypted(root: &Value) -> bool {
    root["header"].get("slots").is_some_and(|s| !s.is_null())
}

/// Reads an export: a plain one as it is, an encrypted one with `password`.
pub(crate) fn read(root: &Value, password: Option<&str>) -> Result<Import, String> {
    if !is_encrypted(root) {
        return Ok(entries(root.get("db").filter(|db| db.is_object()).ok_or("The Aegis export is damaged")?));
    }
    let password = password.ok_or("This Aegis export is encrypted: its password is needed")?;
    let mut db = decrypt(root, password)?;
    let import = entries(&db);
    cxf::wipe(&mut db);
    Ok(import)
}

/// The vault of an encrypted export, decrypted with the master key a
/// password slot gives.
fn decrypt(root: &Value, password: &str) -> Result<Value, String> {
    let header = &root["header"];
    let damaged = || "The Aegis export is damaged".to_string();
    let mut slots = list(header, "slots").filter(|slot| slot.get("type").and_then(Value::as_u64) == Some(PASSWORD_SLOT)).peekable();
    if slots.peek().is_none() {
        return Err("This Aegis export has no password to open it with (only a fingerprint's): export it again with a password".into());
    }
    let master = slots.find_map(|slot| unwrap_master(slot, password)).ok_or("Wrong password for this Aegis export")?;
    let params = header.get("params").ok_or_else(damaged)?;
    let sealed = base64::engine::general_purpose::STANDARD.decode(text(root, "db").ok_or_else(damaged)?).map_err(|_| damaged())?;
    let plain = Zeroizing::new(open(&master, params, &sealed).ok_or_else(damaged)?);
    serde_json::from_slice(&plain).map_err(|_| damaged())
}

/// The master key a password slot holds, when `password` is its password.
fn unwrap_master(slot: &Value, password: &str) -> Option<Zeroizing<Vec<u8>>> {
    let salt = hex(text(slot, "salt")?)?;
    let number = |key| slot.get(key).and_then(Value::as_u64);
    let (n, r, p) = (number("n")?, u32::try_from(number("r")?).ok()?, u32::try_from(number("p")?).ok()?);
    // scrypt takes N as a power of two. The cost comes from the file: kept
    // within what a phone can do (Aegis uses 2^15, r 8, p 1), so a file made
    // to ask for more memory than there is cannot stop the app.
    let log_n = u8::try_from(n.trailing_zeros()).ok().filter(|&log_n| n.is_power_of_two() && log_n <= 20)?;
    if r > 32 || p > 16 || 128 * u64::from(r) * n > 1 << 30 {
        return None;
    }
    let params = scrypt::Params::new(log_n, r, p).ok()?;
    let mut derived = Zeroizing::new([0u8; 32]);
    scrypt::scrypt(password.as_bytes(), &salt, &params, derived.as_mut()).ok()?;
    let wrapped = hex(text(slot, "key")?)?;
    open(derived.as_ref(), slot.get("key_params")?, &wrapped).map(Zeroizing::new)
}

/// AES-256-GCM with the hex `nonce` and `tag` in `params`; `None` when the key
/// is wrong or the data damaged.
fn open(key: &[u8], params: &Value, sealed: &[u8]) -> Option<Vec<u8>> {
    let (nonce, tag) = (hex(text(params, "nonce")?)?, hex(text(params, "tag")?)?);
    let nonce = Nonce::try_from(nonce.as_slice()).ok()?;
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    let joined: Vec<u8> = sealed.iter().chain(&tag).copied().collect();
    cipher.decrypt(&nonce, Payload { msg: &joined, aad: &[] }).ok()
}

fn hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok()).collect()
}

/// The tokens of a decrypted (or plain) vault as entries: the title from the
/// issuer, the user name from the name, the TOTP as an `otpauth://` URI.
fn entries(db: &Value) -> Import {
    let groups: HashMap<&str, &str> = list(db, "groups").filter_map(|g| Some((text(g, "uuid")?, text(g, "name")?))).collect();
    let mut import = Import { exporter: String::new(), entries: Vec::new(), skipped: Vec::new() };
    for token in list(db, "entries") {
        let (name, issuer) = (text(token, "name").unwrap_or("").trim(), text(token, "issuer").unwrap_or("").trim());
        let title = if issuer.is_empty() { name } else { issuer };
        let mut data = EntryData::default();
        data.title = title.to_string();
        data.username = name.to_string();
        data.notes = text(token, "note").unwrap_or("").to_string();
        if token.get("favorite").and_then(Value::as_bool) == Some(true) {
            data.tags.push(FAVORITE.to_string());
        }
        data.tags.extend(list(token, "groups").filter_map(Value::as_str).filter_map(|id| groups.get(id)).map(|name| name.to_string()));
        data.icon = icon(token);
        let mut why = Vec::new();
        match uri(token, &data.title, name) {
            Ok(otp) => data.otp = otp,
            Err(e) => why.push(e),
        }
        if data.otp.is_empty() {
            import.skipped.extend(why.into_iter().map(|why| cxf::Skipped { title: title.to_string(), why }));
            continue;
        }
        cxf::finish(&mut import, data, Fields::default(), Vec::new(), why, (None, None));
    }
    import
}

/// The token as an `otpauth://` URI: TOTP and Steam as `totp` (Steam marked
/// as KeePassXC marks it), HOTP as `hotp`, kept for other clients though
/// PswManager shows no code for it. mOTP and Yandex have no URI to go in.
fn uri(token: &Value, title: &str, name: &str) -> Result<String, String> {
    let kind = text(token, "type").unwrap_or("");
    let info = token.get("info").ok_or("A token without its secret")?;
    let secret = text(info, "secret").filter(|s| !s.is_empty()).ok_or("A token without its secret")?;
    let mut url = url::Url::parse(&format!("otpauth://{}/", if kind == "hotp" { "hotp" } else { "totp" })).expect("constant URL");
    url.set_path(&if title.is_empty() || title == name { name.to_string() } else { format!("{title}:{name}") });
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("secret", secret);
        if !title.is_empty() {
            query.append_pair("issuer", title);
        }
        let number = |key| info.get(key).and_then(Value::as_u64).map(|n| n.to_string());
        match kind {
            "totp" | "hotp" => {
                query.append_pair("algorithm", text(info, "algo").unwrap_or("SHA1"));
                query.append_pair("digits", &number("digits").unwrap_or_else(|| "6".into()));
                match kind {
                    "totp" => query.append_pair("period", &number("period").unwrap_or_else(|| "30".into())),
                    _ => query.append_pair("counter", &number("counter").unwrap_or_else(|| "0".into())),
                };
            }
            "steam" => {
                query.append_pair("encoder", "steam");
            }
            "motp" | "yandex" => return Err(format!("A {} token: PswManager has no place for it", if kind == "motp" { "mOTP" } else { "Yandex" })),
            _ => return Err(format!("A token of a kind PswManager does not know ({kind})")),
        }
    }
    Ok(url.to_string())
}

/// The token's icon, when it is one an entry can keep (PNG or JPEG, small);
/// otherwise the entry's usual one.
fn icon(token: &Value) -> IconChoice {
    let Some(data) = text(token, "icon") else { return IconChoice::Auto };
    match base64::engine::general_purpose::STANDARD.decode(data) {
        Ok(bytes) if crate::edit::check_icon_image(&bytes).is_ok() => IconChoice::Custom { data: data.to_string() },
        _ => IconChoice::Auto,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::otp::Totp;
    use serde_json::json;

    /// A made-up vault: a TOTP token in a group, a Steam one, an HOTP one
    /// and an mOTP one.
    fn vault() -> Value {
        json!({
            "version": 3,
            "entries": [
                {"type": "totp", "uuid": "1", "name": "alice@example.com", "issuer": "Example", "note": "work", "favorite": true,
                 "icon": null, "groups": ["g1"], "info": {"secret": "JBSWY3DPEHPK3PXP", "algo": "SHA256", "digits": 8, "period": 60}},
                {"type": "steam", "uuid": "2", "name": "gamer", "issuer": "Steam", "note": "", "favorite": false,
                 "icon": null, "groups": [], "info": {"secret": "GEZDGNBV", "algo": "SHA1", "digits": 5, "period": 30}},
                {"type": "hotp", "uuid": "3", "name": "bob", "issuer": "", "note": "", "favorite": false,
                 "icon": null, "groups": [], "info": {"secret": "GEZDGNBV", "algo": "SHA1", "digits": 6, "counter": 4}},
                {"type": "motp", "uuid": "4", "name": "carol", "issuer": "Old", "note": "", "favorite": false,
                 "icon": null, "groups": [], "info": {"secret": "GEZDGNBV", "algo": "MD5", "digits": 6, "period": 10, "pin": "1234"}}
            ],
            "groups": [{"uuid": "g1", "name": "Work"}]
        })
    }

    fn plain() -> Value {
        json!({"version": 1, "header": {"slots": null, "params": null}, "db": vault()})
    }

    fn to_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// `content` sealed as an Aegis export with `password` (scrypt kept small
    /// so the test is quick): the tag apart from the ciphertext, as Aegis keeps it.
    fn encrypted(content: &Value, password: &str) -> Value {
        let seal = |key: &[u8], nonce: [u8; 12], plain: &[u8]| {
            let mut sealed = Aes256Gcm::new_from_slice(key).unwrap().encrypt(&Nonce::try_from(&nonce[..]).unwrap(), plain).unwrap();
            let tag = sealed.split_off(sealed.len() - 16);
            (sealed, json!({"nonce": to_hex(&nonce), "tag": to_hex(&tag)}))
        };
        let (master, salt) = ([7u8; 32], [3u8; 32]);
        let mut derived = [0u8; 32];
        scrypt::scrypt(password.as_bytes(), &salt, &scrypt::Params::new(4, 8, 1).unwrap(), &mut derived).unwrap();
        let (wrapped, key_params) = seal(&derived, [1; 12], &master);
        let (db, params) = seal(&master, [2; 12], content.to_string().as_bytes());
        json!({
            "version": 1,
            "header": {
                "slots": [
                    {"type": 2, "uuid": "x", "key": "00", "key_params": {"nonce": "00", "tag": "00"}},
                    {"type": 1, "uuid": "y", "key": to_hex(&wrapped), "key_params": key_params,
                     "n": 16, "r": 8, "p": 1, "salt": to_hex(&salt), "repaired": true}
                ],
                "params": params
            },
            "db": base64::engine::general_purpose::STANDARD.encode(db)
        })
    }

    fn check(import: &Import) {
        let titles: Vec<&str> = import.entries.iter().map(|e| e.data.title.as_str()).collect();
        assert_eq!(titles, ["Example", "Steam", "bob"]);
        let example = &import.entries[0].data;
        assert_eq!((example.username.as_str(), example.notes.as_str()), ("alice@example.com", "work"));
        assert_eq!(example.tags, [FAVORITE, "Work"]);
        let totp = Totp::parse(&example.otp).unwrap();
        assert_eq!((totp.algorithm(), totp.digits(), totp.period()), ("SHA256", 8, 60));
        assert!(Totp::parse(&import.entries[1].data.otp).unwrap().is_steam());
        assert!(import.entries[2].data.otp.starts_with("otpauth://hotp/bob?secret=GEZDGNBV"));
        assert_eq!(import.skipped.len(), 1);
        assert_eq!(import.skipped[0].title, "Old");
        assert!(import.skipped[0].why.contains("mOTP"));
    }

    #[test]
    fn reads_a_plain_export() {
        let root = plain();
        assert!(is_export(&root) && !is_encrypted(&root));
        check(&read(&root, None).unwrap());
    }

    #[test]
    fn reads_an_encrypted_export_with_its_password() {
        let root = encrypted(&vault(), "correct horse");
        assert!(is_export(&root) && is_encrypted(&root));
        check(&read(&root, Some("correct horse")).unwrap());
        assert_eq!(read(&root, Some("wrong")).err().unwrap(), "Wrong password for this Aegis export");
        assert!(read(&root, None).err().unwrap().contains("password is needed"));
        // Only a fingerprint's slot; a cost a phone cannot pay.
        let mut fingerprint = root.clone();
        fingerprint["header"]["slots"].as_array_mut().unwrap().remove(1);
        assert!(read(&fingerprint, Some("correct horse")).err().unwrap().contains("no password"));
        let mut costly = root.clone();
        costly["header"]["slots"][1]["n"] = json!(1u64 << 40);
        assert_eq!(read(&costly, Some("correct horse")).err().unwrap(), "Wrong password for this Aegis export");
    }

    #[test]
    fn keeps_only_icons_an_entry_can_show() {
        let png = base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR");
        assert_eq!(icon(&json!({"icon": png})), IconChoice::Custom { data: png });
        let svg = base64::engine::general_purpose::STANDARD.encode(b"<svg xmlns='http://www.w3.org/2000/svg'/>");
        assert_eq!(icon(&json!({"icon": svg})), IconChoice::Auto);
        assert_eq!(icon(&json!({"icon": null})), IconChoice::Auto);
    }
}
