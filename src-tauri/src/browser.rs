//! Browser integration (#273): the KeePassXC-Browser protocol, so that the
//! KeePassXC-Browser extension (Chrome, Edge, Firefox…) works with PswManager.
//! A connection swaps temporary X25519 keys in the clear, then every message
//! is NaCl's `crypto_box` (XSalsa20-Poly1305) with a 24-byte nonce the answer
//! increments. What the extension asks of the database goes through [Host];
//! how the messages travel is `native_messaging`'s.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use crypto_box::aead::generic_array::GenericArray;
use crypto_box::aead::Aead;
use crypto_box::{PublicKey, SalsaBox, SecretKey};
use pswm_core::browser::{Group, Login};
use serde_json::{json, Map, Value};

/// The KeePassXC version reported: the extension turns its features on by it.
/// 2.7.10 has the password generator, the new TOTP, and passkeys with a
/// default group (#285).
pub const VERSION: &str = "2.7.10";

/// KeePassXC's error codes, as the extension knows them: a request's, and
/// those a passkey answer carries (#285).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Failure {
    DatabaseNotOpened = 1,
    ClientKeyNotReceived = 3,
    CannotDecrypt = 4,
    CancelledOrDenied = 6,
    CannotEncrypt = 7,
    AssociationFailed = 8,
    IncorrectAction = 12,
    NoUrl = 14,
    NoLogins = 15,
    NoGroups = 16,
    CannotCreateGroup = 17,
    NoValidUuid = 18,
    CredentialIsExcluded = 21,
    PasskeyCancelled = 22,
    EmptyPublicKey = 24,
    PasskeyFailed = 31,
}

impl Failure {
    fn message(self) -> &'static str {
        match self {
            Failure::DatabaseNotOpened => "Database not opened",
            Failure::ClientKeyNotReceived => "Client public key not received",
            Failure::CannotDecrypt => "Cannot decrypt message",
            Failure::CancelledOrDenied => "Action cancelled or denied",
            Failure::CannotEncrypt => "Message encryption failed.",
            Failure::AssociationFailed => "KeePassXC association failed, try again",
            Failure::IncorrectAction => "Incorrect action",
            Failure::NoUrl => "No URL provided",
            Failure::NoLogins => "No logins found",
            Failure::NoGroups => "No groups found",
            Failure::CannotCreateGroup => "Cannot create new group",
            Failure::NoValidUuid => "No valid UUID provided",
            Failure::CredentialIsExcluded => "Credential is excluded",
            Failure::PasskeyCancelled => "Passkeys request canceled",
            Failure::PasskeyFailed => "Unknown passkeys error",
            Failure::EmptyPublicKey => "Empty public key",
        }
    }
}

/// What the protocol needs of the app. Questions to the user (associating,
/// replacing a login) wait for the answer in the window.
pub trait Host {
    fn unlocked(&self) -> bool;
    /// The extension asks for the database to be unlocked: the window comes up.
    fn ask_to_unlock(&self);
    /// [pswm_core::browser::database_hash] of the open database.
    fn hash(&self) -> Option<String>;
    /// The identification key associated as `name`.
    fn key(&self, name: &str) -> Option<String>;
    /// Asks the user to associate an extension with this identification key;
    /// the name kept, or `None` when declined.
    fn associate(&self, key: &str) -> Option<String>;
    fn logins(&self, url: &str) -> Vec<Login>;
    fn totp(&self, uuid: &str) -> Option<String>;
    fn groups(&self) -> Option<Group>;
    /// The new group's name and UUID.
    fn create_group(&self, path: &str) -> Option<(String, String)>;
    /// Saves a login for a page; replacing one asks the user first. False
    /// when declined or refused.
    fn save_login(&self, uuid: Option<&str>, url: &str, login: &str, password: &str, group: Option<&str>) -> bool;
    fn generate_password(&self) -> Option<String>;
    fn lock(&self);
    /// Signs in with a passkey for a page (`passkeys-get`): WebAuthn's
    /// `PublicKeyCredential` as JSON, or `{ "errorCode": … }`.
    fn passkeys_get(&self, public_key: Value, origin: &str, related: &[String]) -> Value;
    /// Makes a passkey for a page (`passkeys-register`), kept in group `group`
    /// (else the default one): as [Host::passkeys_get] answers.
    fn passkeys_register(&self, public_key: Value, origin: &str, related: &[String], group: Option<&str>) -> Value;
}

/// One extension's connection: its temporary key and ours, and the database
/// (by its hash) it proved it is associated with — another one opened since
/// gives it nothing until it proves that one.
#[derive(Default)]
pub struct Session {
    client: Option<PublicKey>,
    secret: Option<SecretKey>,
    associated: Option<String>,
}

/// The nonce of the answer: the request's, plus one (little-endian), as
/// libsodium's `sodium_increment`.
fn increment(nonce: &[u8]) -> Vec<u8> {
    let mut next = nonce.to_vec();
    for byte in &mut next {
        *byte = byte.wrapping_add(1);
        if *byte != 0 {
            break;
        }
    }
    next
}

fn new_secret() -> SecretKey {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the OS gives random bytes");
    SecretKey::from(bytes)
}

/// A message's nonce: 24 bytes, in base64.
fn nonce(text: &str) -> Option<Vec<u8>> {
    B64.decode(text).ok().filter(|n| n.len() == 24)
}

fn public_key(text: &str) -> Option<PublicKey> {
    let bytes: [u8; 32] = B64.decode(text).ok()?.try_into().ok()?;
    Some(PublicKey::from(bytes))
}

/// An entry's UUID as KeePassXC writes it: 32 hex digits.
fn is_uuid(text: &str) -> bool {
    text.len() == 32 && text.chars().all(|c| c.is_ascii_hexdigit())
}

fn error(action: &str, failure: Failure) -> Value {
    json!({ "action": action, "errorCode": (failure as u8).to_string(), "error": failure.message() })
}

impl Session {
    fn sealed(&self) -> Option<SalsaBox> {
        Some(SalsaBox::new(self.client.as_ref()?, self.secret.as_ref()?))
    }

    /// The request's encrypted message, and the nonce of the answer.
    fn open(&self, request: &Value) -> Option<(Map<String, Value>, Vec<u8>)> {
        let nonce = nonce(request.get("nonce")?.as_str()?)?;
        let message = B64.decode(request.get("message")?.as_str()?).ok()?;
        let plain = self.sealed()?.decrypt(GenericArray::from_slice(&nonce), message.as_slice()).ok()?;
        let Value::Object(map) = serde_json::from_slice(&plain).ok()? else { return None };
        Some((map, increment(&nonce)))
    }

    /// An encrypted answer carrying `fields`.
    fn answer(&self, action: &str, nonce: &[u8], fields: Value) -> Value {
        let nonce_text = B64.encode(nonce);
        let mut message = json!({ "version": VERSION, "success": "true", "nonce": nonce_text });
        if let (Value::Object(message), Value::Object(fields)) = (&mut message, fields) {
            message.extend(fields);
        }
        let sealed = self.sealed().and_then(|b| b.encrypt(GenericArray::from_slice(nonce), message.to_string().as_bytes()).ok());
        match sealed {
            Some(sealed) => json!({ "action": action, "message": B64.encode(sealed), "nonce": nonce_text }),
            None => error(action, Failure::CannotEncrypt),
        }
    }

    /// The answer to one request from the extension.
    pub fn handle(&mut self, host: &impl Host, request: &Value) -> Value {
        let action = request.get("action").and_then(Value::as_str).unwrap_or_default();
        if action.is_empty() {
            return error(action, Failure::IncorrectAction);
        }
        if action == "change-public-keys" {
            return self.change_keys(request);
        }
        if !host.unlocked() {
            if self.client.is_none() {
                return error(action, Failure::ClientKeyNotReceived);
            }
            if request.get("triggerUnlock").and_then(Value::as_str) == Some("true") {
                host.ask_to_unlock();
            }
            return error(action, Failure::DatabaseNotOpened);
        }
        let Some((message, nonce)) = self.open(request) else { return error(action, Failure::CannotDecrypt) };
        let text = |name: &str| message.get(name).and_then(Value::as_str).unwrap_or_default();
        let hash = host.hash().unwrap_or_default();
        let associated = self.associated.as_deref() == Some(hash.as_str());
        let needs_association =
            ["get-logins", "set-login", "get-database-groups", "create-new-group", "get-totp", "lock-database", "passkeys-get", "passkeys-register"];
        if needs_association.contains(&action) && !associated {
            return error(action, Failure::AssociationFailed);
        }
        match action {
            "get-databasehash" => self.answer(action, &nonce, json!({ "hash": hash })),
            "associate" => {
                // The key must be this connection's: the identification key comes with it.
                if text("key").is_empty() || public_key(text("key")) != self.client {
                    return error(action, Failure::AssociationFailed);
                }
                let id_key = Some(text("idKey")).filter(|k| !k.is_empty()).unwrap_or(text("key"));
                match host.associate(id_key) {
                    Some(name) => {
                        self.associated = Some(hash.clone());
                        self.answer(action, &nonce, json!({ "hash": hash, "id": name }))
                    }
                    None => error(action, Failure::CancelledOrDenied),
                }
            }
            "test-associate" => {
                let (id, key) = (text("id"), text("key"));
                if id.is_empty() || key.is_empty() {
                    return error(action, Failure::DatabaseNotOpened);
                }
                if host.key(id).as_deref() != Some(key) {
                    return error(action, Failure::AssociationFailed);
                }
                self.associated = Some(hash.clone());
                self.answer(action, &nonce, json!({ "hash": hash, "id": id }))
            }
            "get-logins" => {
                let url = text("url");
                if url.is_empty() {
                    return error(action, Failure::NoUrl);
                }
                let logins = host.logins(url);
                if logins.is_empty() {
                    return error(action, Failure::NoLogins);
                }
                self.answer(action, &nonce, json!({ "count": logins.len(), "entries": logins, "hash": hash, "id": text("id") }))
            }
            "get-totp" => {
                let uuid = text("uuid");
                if !is_uuid(uuid) {
                    return error(action, Failure::NoValidUuid);
                }
                self.answer(action, &nonce, json!({ "totp": host.totp(uuid).unwrap_or_default() }))
            }
            "generate-password" => match host.generate_password() {
                Some(password) => self.answer(action, &nonce, json!({ "password": password })),
                None => error(action, Failure::CancelledOrDenied),
            },
            "set-login" => {
                let (url, uuid) = (text("url"), text("uuid"));
                if url.is_empty() {
                    return error(action, Failure::NoUrl);
                }
                if !uuid.is_empty() && !is_uuid(uuid) {
                    return error(action, Failure::NoValidUuid);
                }
                let given = |s: &'static str| Some(text(s)).filter(|t| !t.is_empty());
                let saved = host.save_login(given("uuid"), url, text("login"), text("password"), given("groupUuid"));
                let outcome = if saved { "success" } else { "error" };
                self.answer(action, &nonce, json!({ "count": null, "entries": null, "error": outcome, "hash": hash }))
            }
            "get-database-groups" => match host.groups() {
                Some(root) => self.answer(action, &nonce, json!({ "groups": { "groups": [root] } })),
                None => error(action, Failure::NoGroups),
            },
            "create-new-group" => match host.create_group(text("groupName")) {
                Some((name, uuid)) => self.answer(action, &nonce, json!({ "name": name, "uuid": uuid })),
                None => error(action, Failure::CannotCreateGroup),
            },
            "passkeys-get" | "passkeys-register" => {
                let Some(public_key) = message.get("publicKey").filter(|k| k.as_object().is_some_and(|o| !o.is_empty())).cloned() else {
                    return error(action, Failure::EmptyPublicKey);
                };
                let related: Vec<String> = message.get("relatedOrigins").and_then(Value::as_array).into_iter().flatten().filter_map(|o| o.as_str().map(String::from)).collect();
                let response = if action == "passkeys-get" {
                    host.passkeys_get(public_key, text("origin"), &related)
                } else {
                    host.passkeys_register(public_key, text("origin"), &related, Some(text("groupName")).filter(|g| !g.is_empty()))
                };
                self.answer(action, &nonce, json!({ "response": response }))
            }
            "lock-database" => {
                host.lock();
                self.associated = None;
                self.answer(action, &nonce, json!({}))
            }
            _ => error(action, Failure::IncorrectAction),
        }
    }

    /// `change-public-keys`: a new key pair of ours for the extension's new
    /// temporary key; it must prove its association again.
    fn change_keys(&mut self, request: &Value) -> Value {
        let action = "change-public-keys";
        let nonce = request.get("nonce").and_then(Value::as_str).and_then(nonce);
        let client = request.get("publicKey").and_then(Value::as_str).and_then(public_key);
        let (Some(nonce), Some(client)) = (nonce, client) else { return error(action, Failure::ClientKeyNotReceived) };
        let secret = new_secret();
        let public = B64.encode(secret.public_key().as_bytes());
        (self.client, self.secret, self.associated) = (Some(client), Some(secret), None);
        json!({ "action": action, "version": VERSION, "publicKey": public, "success": "true", "nonce": B64.encode(increment(&nonce)) })
    }
}

/// What the app tells every extension connected when the database locks or
/// unlocks, unencrypted as KeePassXC sends it.
pub fn signal(unlocked: bool) -> Value {
    json!({ "action": if unlocked { "database-unlocked" } else { "database-locked" } })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A database with one association and one login, and what was asked of it.
    struct Fake {
        unlocked: bool,
        hash: RefCell<String>,
        asked_to_unlock: RefCell<bool>,
        saved: RefCell<Vec<String>>,
    }

    impl Host for Fake {
        fn unlocked(&self) -> bool {
            self.unlocked
        }
        fn ask_to_unlock(&self) {
            *self.asked_to_unlock.borrow_mut() = true;
        }
        fn hash(&self) -> Option<String> {
            Some(self.hash.borrow().clone())
        }
        fn key(&self, name: &str) -> Option<String> {
            (name == "Chrome").then(|| "aWQta2V5".into())
        }
        fn associate(&self, _key: &str) -> Option<String> {
            Some("New".into())
        }
        fn logins(&self, url: &str) -> Vec<Login> {
            if !url.contains("mail.example") {
                return Vec::new();
            }
            vec![Login { login: "me".into(), password: "pw".into(), name: "Mail".into(), uuid: "0".repeat(32), group: "Root".into(), totp: None, expired: None, skip_auto_submit: None }]
        }
        fn totp(&self, _uuid: &str) -> Option<String> {
            Some("123456".into())
        }
        fn groups(&self) -> Option<Group> {
            None
        }
        fn create_group(&self, _path: &str) -> Option<(String, String)> {
            None
        }
        fn save_login(&self, _uuid: Option<&str>, url: &str, _login: &str, _password: &str, _group: Option<&str>) -> bool {
            self.saved.borrow_mut().push(url.into());
            true
        }
        fn generate_password(&self) -> Option<String> {
            Some("generated".into())
        }
        fn lock(&self) {}
        fn passkeys_get(&self, _public_key: Value, origin: &str, _related: &[String]) -> Value {
            json!({ "id": "credential", "origin": origin })
        }
        fn passkeys_register(&self, _public_key: Value, _origin: &str, _related: &[String], group: Option<&str>) -> Value {
            json!({ "errorCode": 22, "group": group })
        }
    }

    fn fake(unlocked: bool) -> Fake {
        Fake { unlocked, hash: RefCell::new("hash".into()), asked_to_unlock: RefCell::new(false), saved: RefCell::new(Vec::new()) }
    }

    /// The extension's side of a connection.
    struct Extension {
        secret: SecretKey,
        host: Option<PublicKey>,
        nonce: [u8; 24],
    }

    impl Extension {
        fn new() -> Extension {
            Extension { secret: new_secret(), host: None, nonce: [7; 24] }
        }

        fn change_keys(&mut self, session: &mut Session, host: &Fake) {
            let request = json!({ "action": "change-public-keys", "publicKey": B64.encode(self.secret.public_key().as_bytes()), "nonce": B64.encode(self.nonce), "clientID": "x" });
            let answer = session.handle(host, &request);
            assert_eq!(answer["nonce"], B64.encode(increment(&self.nonce)));
            self.host = public_key(answer["publicKey"].as_str().unwrap());
        }

        fn sealed(&self) -> SalsaBox {
            SalsaBox::new(self.host.as_ref().unwrap(), &self.secret)
        }

        /// Sends `message` encrypted and returns the decrypted answer, or the error.
        fn send(&mut self, session: &mut Session, host: &Fake, message: Value) -> Value {
            self.nonce[0] = self.nonce[0].wrapping_add(2);
            let action = message["action"].as_str().unwrap().to_string();
            let sealed = self.sealed().encrypt(GenericArray::from_slice(&self.nonce), message.to_string().as_bytes()).unwrap();
            let request = json!({ "action": action, "message": B64.encode(sealed), "nonce": B64.encode(self.nonce), "clientID": "x" });
            let answer = session.handle(host, &request);
            if answer.get("errorCode").is_some() {
                return answer;
            }
            assert_eq!(answer["nonce"], B64.encode(increment(&self.nonce)), "the answer's nonce is the next one");
            let nonce = B64.decode(answer["nonce"].as_str().unwrap()).unwrap();
            let plain = self.sealed().decrypt(GenericArray::from_slice(&nonce), B64.decode(answer["message"].as_str().unwrap()).unwrap().as_slice()).unwrap();
            serde_json::from_slice(&plain).unwrap()
        }
    }

    #[test]
    fn nonces_count_up_little_endian() {
        assert_eq!(increment(&[0, 0, 0]), [1, 0, 0]);
        assert_eq!(increment(&[255, 255, 0]), [0, 0, 1]);
        assert_eq!(increment(&[255, 255]), [0, 0]);
    }

    #[test]
    fn an_associated_extension_gets_the_site_s_logins() {
        let (host, mut session, mut extension) = (fake(true), Session::default(), Extension::new());
        extension.change_keys(&mut session, &host);
        let refused = extension.send(&mut session, &host, json!({ "action": "get-logins", "url": "https://mail.example/" }));
        assert_eq!(refused["errorCode"], "8", "not associated yet");
        let wrong = extension.send(&mut session, &host, json!({ "action": "test-associate", "id": "Chrome", "key": "b3RoZXI=" }));
        assert_eq!(wrong["errorCode"], "8");
        let ok = extension.send(&mut session, &host, json!({ "action": "test-associate", "id": "Chrome", "key": "aWQta2V5" }));
        assert_eq!((ok["success"].as_str(), ok["id"].as_str(), ok["version"].as_str()), (Some("true"), Some("Chrome"), Some(VERSION)));
        let logins = extension.send(&mut session, &host, json!({ "action": "get-logins", "url": "https://mail.example/" }));
        assert_eq!(logins["count"], 1);
        assert_eq!(logins["entries"][0]["password"], "pw");
        let none = extension.send(&mut session, &host, json!({ "action": "get-logins", "url": "https://evil.example/" }));
        assert_eq!(none["errorCode"], "15");
        let totp = extension.send(&mut session, &host, json!({ "action": "get-totp", "uuid": "0".repeat(32) }));
        assert_eq!(totp["totp"], "123456");
        // Another database opened since: associated with that one first.
        *host.hash.borrow_mut() = "other".into();
        let other = extension.send(&mut session, &host, json!({ "action": "get-logins", "url": "https://mail.example/" }));
        assert_eq!(other["errorCode"], "8");
    }

    #[test]
    fn associating_takes_the_identification_key_with_this_connection_s_key() {
        let (host, mut session, mut extension) = (fake(true), Session::default(), Extension::new());
        extension.change_keys(&mut session, &host);
        let own = B64.encode(extension.secret.public_key().as_bytes());
        let other = extension.send(&mut session, &host, json!({ "action": "associate", "key": B64.encode([1u8; 32]), "idKey": "aWQ=" }));
        assert_eq!(other["errorCode"], "8", "another key than the connection's");
        let answer = extension.send(&mut session, &host, json!({ "action": "associate", "key": own, "idKey": "aWQ=" }));
        assert_eq!((answer["id"].as_str(), answer["hash"].as_str()), (Some("New"), Some("hash")));
        let saved = extension.send(&mut session, &host, json!({ "action": "set-login", "url": "https://mail.example/", "login": "me", "password": "pw" }));
        assert_eq!(saved["error"], "success");
        assert_eq!(host.saved.borrow().as_slice(), ["https://mail.example/"]);
        let generated = extension.send(&mut session, &host, json!({ "action": "generate-password" }));
        assert_eq!(generated["password"], "generated");
        let empty = extension.send(&mut session, &host, json!({ "action": "passkeys-get", "publicKey": {}, "origin": "https://example.com" }));
        assert_eq!(empty["errorCode"], "24");
        let signed = extension.send(&mut session, &host, json!({ "action": "passkeys-get", "publicKey": { "challenge": "x" }, "origin": "https://example.com" }));
        assert_eq!(signed["response"], json!({ "id": "credential", "origin": "https://example.com" }));
        let refused = extension.send(&mut session, &host, json!({ "action": "passkeys-register", "publicKey": { "challenge": "x" }, "origin": "https://example.com", "groupName": "Mine" }));
        assert_eq!(refused["response"], json!({ "errorCode": 22, "group": "Mine" }), "an error goes inside the response");
    }

    #[test]
    fn a_locked_database_is_asked_to_unlock_only_when_the_extension_says() {
        let (host, mut session, mut extension) = (fake(false), Session::default(), Extension::new());
        let before = session.handle(&host, &json!({ "action": "get-databasehash", "nonce": "", "message": "" }));
        assert_eq!(before["errorCode"], "3", "no keys swapped yet");
        extension.change_keys(&mut session, &host);
        let locked = session.handle(&host, &json!({ "action": "get-databasehash", "nonce": "", "message": "" }));
        assert_eq!(locked["errorCode"], "1");
        assert!(!*host.asked_to_unlock.borrow());
        session.handle(&host, &json!({ "action": "get-databasehash", "triggerUnlock": "true" }));
        assert!(*host.asked_to_unlock.borrow());
    }

    #[test]
    fn a_message_that_does_not_open_is_refused() {
        let (host, mut session, mut extension) = (fake(true), Session::default(), Extension::new());
        extension.change_keys(&mut session, &host);
        let answer = session.handle(&host, &json!({ "action": "get-databasehash", "nonce": B64.encode([0u8; 24]), "message": B64.encode(b"not sealed") }));
        assert_eq!(answer["errorCode"], "4");
        assert_eq!(signal(false)["action"], "database-locked");
    }
}
