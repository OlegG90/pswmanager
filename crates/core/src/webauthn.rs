//! The authenticator's part of WebAuthn (#160): making a passkey when a site
//! or app asks (`navigator.credentials.create`) and signing in with one
//! (`navigator.credentials.get`), for PswManager as a credential provider.
//! Passkeys are kept as KeePassXC keeps them ([crate::edit::passkey]), so the
//! ones made here work in KeePassXC and KeePassDX, and the other way round.
//!
//! ES256 only (P-256 with SHA-256), attestation "none", the signature counter
//! always 0 (as KeePassXC and other synced passkeys), and the backup flags on.
//! The user is always verified (UV): the caller asks for the fingerprint or
//! the master password before making or signing, whatever the site prefers.

use crate::edit::{passkey, FieldData};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use base64::Engine;
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};
use p256::elliptic_curve::Generate;
use p256::pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey, LineEnding};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

/// COSE's number for ES256.
const ES256: i64 = -7;
/// No attestation, so no maker to name: an all-zero AAGUID.
const AAGUID: [u8; 16] = [0; 16];

/// Authenticator data flags.
const USER_PRESENT: u8 = 0x01;
const USER_VERIFIED: u8 = 0x04;
const BACKUP_ELIGIBLE: u8 = 0x08;
const BACKED_UP: u8 = 0x10;
const ATTESTED_DATA: u8 = 0x40;

/// Who asks, and how the client data is had.
pub enum Caller {
    /// An app: the origin is its signing certificate (`android:apk-key-hash:…`),
    /// and the client data is made here.
    App { origin: String, package: String },
    /// A browser trusted to speak for a site (`origin`): it made the client
    /// data itself and gives its hash, which is what is signed.
    Browser { origin: String, client_data_hash: Vec<u8> },
}

impl Caller {
    /// The client data for a request of `kind` (`webauthn.create` / `.get`).
    /// A browser's own is what counts: this one only fills the field.
    fn client_data(&self, kind: &str, challenge: &str) -> String {
        let (Caller::App { origin, .. } | Caller::Browser { origin, .. }) = self;
        let mut data = json!({ "type": kind, "challenge": challenge, "origin": origin, "crossOrigin": false });
        if let Caller::App { package, .. } = self {
            data["androidPackageName"] = json!(package);
        }
        data.to_string()
    }

    /// The hash of the client data that is signed.
    fn client_data_hash(&self, client_data: &str) -> Vec<u8> {
        match self {
            Caller::App { .. } => Sha256::digest(client_data.as_bytes()).to_vec(),
            Caller::Browser { client_data_hash, .. } => client_data_hash.clone(),
        }
    }
}

/// A passkey just made: its attributes for the entry (which also gets the tag
/// [passkey::TAG]), and the answer for the site or app (WebAuthn's
/// `RegistrationResponseJSON`).
pub struct Made {
    pub fields: Vec<FieldData>,
    /// The relying party and the user name, for choosing the entry.
    pub rp_id: String,
    pub user_name: String,
    /// The site's name as it gives it (`rp.name`), for a new entry's title.
    pub rp_name: String,
    pub response: String,
}

/// The `origin` of an Android app: the SHA-256 of its signing certificate.
pub fn app_origin(signing_certificate: &[u8]) -> String {
    format!("android:apk-key-hash:{}", B64URL.encode(Sha256::digest(signing_certificate)))
}

/// Makes a passkey from WebAuthn's creation options (`requestJson`), after the
/// user was verified. The options must name the site (`rp.id`), as Android's
/// always do.
pub fn make(options_json: &str, caller: &Caller) -> Result<Made, String> {
    let options = parse(options_json)?;
    let rp_id = string_at(&options["rp"], "id").ok_or("The request names no site (rp.id)")?.to_string();
    let user = &options["user"];
    let user_handle = unpadded(string_at(user, "id").ok_or("The request has no user id")?).to_string();
    let user_name = string_at(user, "name").unwrap_or("").to_string();
    let rp_name = string_at(&options["rp"], "name").unwrap_or(&rp_id).to_string();
    let algorithms = options["pubKeyCredParams"].as_array().map(Vec::as_slice).unwrap_or_default();
    if !algorithms.is_empty() && !algorithms.iter().any(|p| p["alg"].as_i64() == Some(ES256)) {
        return Err("The site wants a kind of key PswManager does not make (only ES256)".into());
    }
    let challenge = string_at(&options, "challenge").ok_or("The request has no challenge")?;
    // A browser speaks for its page: the site must be the page's or above it.
    if let Caller::Browser { origin, .. } = caller {
        let host = url::Url::parse(origin).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
        if !on_site(&host, &rp_id) {
            return Err(format!("{origin} cannot make a passkey for {rp_id}"));
        }
    }

    let key = SigningKey::try_generate().map_err(|e| format!("No randomness: {e}"))?;
    let mut credential_id = [0u8; 32];
    getrandom::fill(&mut credential_id).map_err(|e| format!("No randomness: {e}"))?;
    let public = key.verifying_key();
    let point = public.to_sec1_point(false);
    let cose = cose_key(point.x().expect("uncompressed"), point.y().expect("uncompressed"));

    let mut auth_data = header(&rp_id, USER_PRESENT | USER_VERIFIED | BACKUP_ELIGIBLE | BACKED_UP | ATTESTED_DATA);
    auth_data.extend_from_slice(&AAGUID);
    auth_data.extend_from_slice(&(credential_id.len() as u16).to_be_bytes());
    auth_data.extend_from_slice(&credential_id);
    auth_data.extend_from_slice(&cose);
    let attestation = attestation_object(&auth_data);
    let client_data = caller.client_data("webauthn.create", challenge);

    let pem = key.to_pkcs8_pem(LineEnding::LF).map_err(|e| format!("Cannot keep the key: {e}"))?;
    let spki = public.to_public_key_der().map_err(|e| format!("Cannot give the public key: {e}"))?;
    let id = B64URL.encode(credential_id);
    let response = credential_json(
        &id,
        json!({
            "clientDataJSON": B64URL.encode(client_data.as_bytes()),
            "attestationObject": B64URL.encode(&attestation),
            "authenticatorData": B64URL.encode(&auth_data),
            "transports": ["internal", "hybrid"],
            "publicKeyAlgorithm": ES256,
            "publicKey": B64URL.encode(spki.as_bytes()),
        }),
    );
    let mut fields = passkey::fields(&rp_id, &user_name, &id, &user_handle, &pem);
    for flag in [passkey::FLAG_BE, passkey::FLAG_BS] {
        fields.push(FieldData { name: flag.into(), value: "1".into(), protected: false });
    }
    Ok(Made {
        fields,
        rp_id,
        user_name,
        rp_name,
        response,
    })
}

/// A passkey as an entry keeps it, read for signing.
pub struct Stored<'a> {
    pub rp_id: &'a str,
    pub credential_id: &'a str,
    pub user_handle: &'a str,
    pub private_key_pem: &'a str,
    /// KeePassXC's `FLAG_BE` / `FLAG_BS` (the caller takes them as on when
    /// missing, as KeePassXC does); backed up only counts when eligible.
    pub backup_eligible: bool,
    pub backed_up: bool,
}

/// Signs in with `stored` from WebAuthn's request options (`requestJson`),
/// after the user was verified: WebAuthn's `AuthenticationResponseJSON`.
pub fn sign(options_json: &str, stored: &Stored<'_>, caller: &Caller) -> Result<String, String> {
    let options = parse(options_json)?;
    if let Some(rp_id) = string_at(&options, "rpId") {
        // Domains, so case-blind (as the passkeys are offered).
        if !rp_id.eq_ignore_ascii_case(stored.rp_id) {
            return Err(format!("This passkey is for {}, not {rp_id}", stored.rp_id));
        }
    }
    let challenge = string_at(&options, "challenge").ok_or("The request has no challenge")?;
    let key = private_key(stored.private_key_pem)?;

    let mut flags = USER_PRESENT | USER_VERIFIED;
    if stored.backup_eligible {
        flags |= BACKUP_ELIGIBLE;
        if stored.backed_up {
            flags |= BACKED_UP;
        }
    }
    let auth_data = header(stored.rp_id, flags);
    let client_data = caller.client_data("webauthn.get", challenge);
    let signed = [auth_data.as_slice(), &caller.client_data_hash(&client_data)].concat();
    let signature: Signature = key.sign(&signed);
    Ok(credential_json(
        unpadded(stored.credential_id),
        json!({
            "clientDataJSON": B64URL.encode(client_data.as_bytes()),
            "authenticatorData": B64URL.encode(&auth_data),
            "signature": B64URL.encode(signature.to_der().as_bytes()),
            "userHandle": unpadded(stored.user_handle),
        }),
    ))
}

/// The credential ids a creation request says the user has already
/// (`excludeCredentials`): with one of them in the database, no new passkey.
pub fn excluded_ids(options_json: &str) -> Vec<String> {
    parse(options_json).map(|options| credential_ids(&options["excludeCredentials"])).unwrap_or_default()
}

/// The site a creation request is for: its id (`rp.id`) and name (`rp.name`,
/// else the id).
pub fn creation_site(options_json: &str) -> Option<(String, String)> {
    let options = parse(options_json).ok()?;
    let id = string_at(&options["rp"], "id")?.to_string();
    let name = string_at(&options["rp"], "name").unwrap_or(&id).to_string();
    Some((id, name))
}

/// Whether `host` is the site `rp_id` or under it (WebAuthn's rule for an RP id).
pub fn on_site(host: &str, rp_id: &str) -> bool {
    host.len() >= rp_id.len()
        && host[host.len() - rp_id.len()..].eq_ignore_ascii_case(rp_id)
        && (host.len() == rp_id.len() || host.as_bytes()[host.len() - rp_id.len() - 1] == b'.')
}

/// What a sign-in request is for.
#[derive(Debug, PartialEq)]
pub struct SignInScope {
    pub rp_id: String,
    /// The credential ids it accepts (`allowCredentials`); empty: any of the site's.
    pub allowed: Vec<String>,
}

/// What a sign-in request (`requestJson`) is for; `None` without a site.
pub fn sign_in_scope(options_json: &str) -> Option<SignInScope> {
    let options = parse(options_json).ok()?;
    Some(SignInScope { rp_id: string_at(&options, "rpId")?.to_string(), allowed: credential_ids(&options["allowCredentials"]) })
}

/// A credential as WebAuthn's JSON gives it: `id`, its kind and `response`.
fn credential_json(id: &str, response: Value) -> String {
    json!({
        "id": id,
        "rawId": id,
        "type": "public-key",
        "authenticatorAttachment": "platform",
        "response": response,
        "clientExtensionResults": {},
    })
    .to_string()
}

/// A PKCS#8 key in PEM, wrapped in lines or not (KeePassXC writes either).
fn private_key(pem: &str) -> Result<SigningKey, String> {
    const UNREADABLE: &str = "The passkey's key cannot be read";
    let inner = pem.trim().trim_start_matches("-----BEGIN PRIVATE KEY-----").trim_end_matches("-----END PRIVATE KEY-----");
    let body: Zeroizing<String> = Zeroizing::new(inner.chars().filter(|c| !c.is_whitespace()).collect());
    let der = Zeroizing::new(base64::engine::general_purpose::STANDARD.decode(body.as_bytes()).map_err(|_| UNREADABLE.to_string())?);
    SigningKey::from_pkcs8_der(&der).map_err(|_| UNREADABLE.to_string())
}

fn parse(options_json: &str) -> Result<Value, String> {
    serde_json::from_str(options_json).map_err(|e| format!("Not a passkey request: {e}"))
}

/// The ids in a list of credential descriptors.
fn credential_ids(list: &Value) -> Vec<String> {
    list.as_array().into_iter().flatten().filter_map(|c| string_at(c, "id")).map(|id| unpadded(id).to_string()).collect()
}

/// The non-empty string member `key`.
fn string_at<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str).filter(|v| !v.is_empty())
}

/// base64url without padding, as WebAuthn writes it.
fn unpadded(base64url: &str) -> &str {
    base64url.trim_end_matches('=')
}

/// A credential id's bytes, from base64url or plain base64 (as some
/// exporters write it), padded or not: ids compared this way match however
/// they were written.
pub fn credential_id_bytes(id: &str) -> Option<Vec<u8>> {
    B64URL.decode(unpadded(id.trim()).replace('+', "-").replace('/', "_")).ok()
}

/// The start of authenticator data: the RP id's hash, the flags, the counter (0).
fn header(rp_id: &str, flags: u8) -> Vec<u8> {
    let mut data = Sha256::digest(rp_id.as_bytes()).to_vec();
    data.push(flags);
    data.extend_from_slice(&[0, 0, 0, 0]);
    data
}

/// The public key as COSE (EC2, ES256, P-256), in CTAP2's canonical CBOR.
fn cose_key(x: &[u8], y: &[u8]) -> Vec<u8> {
    let mut out = vec![0xa5]; // a map of 5
    out.extend([0x01, 0x02]); // kty: EC2
    out.extend([0x03, 0x26]); // alg: -7
    out.extend([0x20, 0x01]); // crv: P-256
    out.push(0x21); // x
    cbor_bytes(&mut out, x);
    out.push(0x22); // y
    cbor_bytes(&mut out, y);
    out
}

/// `{ "fmt": "none", "attStmt": {}, "authData": … }` in canonical CBOR.
fn attestation_object(auth_data: &[u8]) -> Vec<u8> {
    let mut out = vec![0xa3];
    cbor_text(&mut out, "fmt");
    cbor_text(&mut out, "none");
    cbor_text(&mut out, "attStmt");
    out.push(0xa0);
    cbor_text(&mut out, "authData");
    cbor_bytes(&mut out, auth_data);
    out
}

/// A CBOR head: major type and length (up to 64 KiB, all this needs).
fn cbor_head(out: &mut Vec<u8>, major: u8, len: usize) {
    debug_assert!(len <= 0xffff);
    let major = major << 5;
    match len {
        0..=23 => out.push(major | len as u8),
        24..=0xff => out.extend([major | 24, len as u8]),
        _ => {
            out.push(major | 25);
            out.extend((len as u16).to_be_bytes());
        }
    }
}

fn cbor_bytes(out: &mut Vec<u8>, value: &[u8]) {
    cbor_head(out, 2, value.len());
    out.extend_from_slice(value);
}

fn cbor_text(out: &mut Vec<u8>, value: &str) {
    cbor_head(out, 3, value.len());
    out.extend_from_slice(value.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::signature::Verifier;
    use p256::ecdsa::VerifyingKey;
    use p256::pkcs8::DecodePublicKey;

    const CREATE: &str = r#"{"rp": {"id": "example.com", "name": "Example"},
        "user": {"id": "dXNlci0x", "name": "alice@example.com", "displayName": "Alice"},
        "challenge": "Y2hhbGxlbmdl", "pubKeyCredParams": [{"type": "public-key", "alg": -8}, {"type": "public-key", "alg": -7}],
        "excludeCredentials": [{"type": "public-key", "id": "b2xkLWlk"}]}"#;

    fn app() -> Caller {
        Caller::App { origin: app_origin(b"certificate"), package: "com.example.app".into() }
    }

    fn field<'a>(made: &'a Made, name: &str) -> &'a str {
        &made.fields.iter().find(|f| f.name == name).unwrap().value
    }

    fn decode(response: &str, path: &[&str]) -> Vec<u8> {
        let json: Value = serde_json::from_str(response).unwrap();
        let value = path.iter().fold(&json, |v, k| &v[*k]);
        B64URL.decode(value.as_str().unwrap()).unwrap()
    }

    #[test]
    fn makes_a_passkey_as_keepassxc_keeps_it() {
        let made = make(CREATE, &app()).unwrap();
        assert_eq!((made.rp_id.as_str(), made.user_name.as_str()), ("example.com", "alice@example.com"));
        assert_eq!(field(&made, passkey::RELYING_PARTY), "example.com");
        assert_eq!(field(&made, passkey::USER_HANDLE), "dXNlci0x");
        assert_eq!(field(&made, passkey::FLAG_BE), "1");
        assert!(field(&made, passkey::PRIVATE_KEY).starts_with("-----BEGIN PRIVATE KEY-----\n"));
        assert_eq!(B64URL.decode(field(&made, passkey::CREDENTIAL_ID)).unwrap().len(), 32);
        assert_eq!(made.fields.iter().filter(|f| f.protected).count(), 3);
    }

    #[test]
    fn the_registration_holds_the_key_and_the_flags() {
        let made = make(CREATE, &app()).unwrap();
        let auth = decode(&made.response, &["response", "authenticatorData"]);
        assert_eq!(&auth[..32], Sha256::digest(b"example.com").as_slice());
        assert_eq!(auth[32], 0x5d); // UP, UV, BE, BS, AT
        assert_eq!(&auth[33..37], &[0, 0, 0, 0]);
        assert_eq!(&auth[37..53], &AAGUID);
        assert_eq!(u16::from_be_bytes([auth[53], auth[54]]), 32);
        let cose = &auth[55 + 32..];
        assert_eq!(&cose[..7], &[0xa5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01]);
        // The attestation object carries the same authenticator data.
        let attestation = decode(&made.response, &["response", "attestationObject"]);
        assert!(attestation.windows(auth.len()).any(|w| w == auth.as_slice()));
        assert!(attestation.starts_with(&[0xa3, 0x63, b'f', b'm', b't', 0x64, b'n', b'o', b'n', b'e']));
        // The client data names the app.
        let client: Value = serde_json::from_slice(&decode(&made.response, &["response", "clientDataJSON"])).unwrap();
        assert_eq!(client["type"], "webauthn.create");
        assert_eq!(client["challenge"], "Y2hhbGxlbmdl");
        assert_eq!(client["androidPackageName"], "com.example.app");
        assert!(client["origin"].as_str().unwrap().starts_with("android:apk-key-hash:"));
    }

    #[test]
    fn a_sign_in_verifies_with_the_public_key() {
        let made = make(CREATE, &app()).unwrap();
        let public = VerifyingKey::from_public_key_der(&decode(&made.response, &["response", "publicKey"])).unwrap();
        let stored = Stored {
            rp_id: "example.com",
            credential_id: field(&made, passkey::CREDENTIAL_ID),
            user_handle: field(&made, passkey::USER_HANDLE),
            private_key_pem: field(&made, passkey::PRIVATE_KEY),
            backup_eligible: true,
            backed_up: true,
        };
        let request = r#"{"rpId": "example.com", "challenge": "c2lnbi1pbg", "allowCredentials": []}"#;
        let response = sign(request, &stored, &app()).unwrap();
        let auth = decode(&response, &["response", "authenticatorData"]);
        assert_eq!(auth.len(), 37);
        assert_eq!(auth[32], 0x1d); // UP, UV, BE, BS
        let client = decode(&response, &["response", "clientDataJSON"]);
        let signature = Signature::from_der(&decode(&response, &["response", "signature"])).unwrap();
        let signed = [auth.as_slice(), &Sha256::digest(&client)].concat();
        assert!(public.verify(&signed, &signature).is_ok());
        let json: Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["response"]["userHandle"], "dXNlci0x");

        // A browser's own client data hash is what is signed.
        let hash = Sha256::digest(b"the browser's client data").to_vec();
        let response = sign(request, &stored, &Caller::Browser { origin: "https://example.com".into(), client_data_hash: hash.clone() }).unwrap();
        let auth = decode(&response, &["response", "authenticatorData"]);
        let signature = Signature::from_der(&decode(&response, &["response", "signature"])).unwrap();
        assert!(public.verify(&[auth.as_slice(), &hash].concat(), &signature).is_ok());
    }

    #[test]
    fn signs_with_a_key_keepassxc_made() {
        // A key in the PEM form KeePassXC writes (one line of base64).
        let key = SigningKey::generate();
        let der = key.to_pkcs8_der().unwrap();
        let pem = format!("-----BEGIN PRIVATE KEY-----{}-----END PRIVATE KEY-----", base64::engine::general_purpose::STANDARD.encode(der.as_bytes()));
        // Backed up without being eligible is no state: neither flag.
        let stored = Stored { rp_id: "example.com", credential_id: "aWQ=", user_handle: "dQ", private_key_pem: &pem, backup_eligible: false, backed_up: true };
        let response = sign(r#"{"rpId": "example.com", "challenge": "Yw"}"#, &stored, &app()).unwrap();
        assert_eq!(decode(&response, &["response", "authenticatorData"])[32], 0x05);
        let json: Value = serde_json::from_str(&response).unwrap();
        assert_eq!(json["id"], "aWQ");
    }

    #[test]
    fn tells_a_site_and_its_hosts() {
        assert!(on_site("example.com", "example.com"));
        assert!(on_site("login.Example.com", "example.com"));
        assert!(!on_site("badexample.com", "example.com"));
        assert!(!on_site("com", "example.com"));
        let browser = |origin: &str| Caller::Browser { origin: origin.into(), client_data_hash: vec![0; 32] };
        assert!(make(CREATE, &browser("https://login.example.com")).is_ok());
        assert!(make(CREATE, &browser("https://evil.test")).is_err());
        assert_eq!(creation_site(CREATE), Some(("example.com".into(), "Example".into())));
        assert_eq!(creation_site(r#"{"rp": {"id": "x.test"}}"#), Some(("x.test".into(), "x.test".into())));
        assert_eq!(creation_site("{}"), None);
    }

    #[test]
    fn refuses_what_it_cannot_do() {
        let only_rsa = CREATE.replace(r#"{"type": "public-key", "alg": -7}"#, r#"{"type": "public-key", "alg": -257}"#);
        assert!(make(&only_rsa, &app()).is_err());
        assert!(make(r#"{"user": {"id": "dQ"}, "challenge": "Yw"}"#, &app()).is_err());
        let made = make(CREATE, &app()).unwrap();
        let stored = Stored {
            rp_id: "example.com",
            credential_id: "aWQ",
            user_handle: "dQ",
            private_key_pem: field(&made, passkey::PRIVATE_KEY),
            backup_eligible: true,
            backed_up: true,
        };
        assert!(sign(r#"{"rpId": "other.com", "challenge": "Yw"}"#, &stored, &app()).is_err());
    }

    #[test]
    fn reads_what_a_request_allows() {
        assert_eq!(excluded_ids(CREATE), ["b2xkLWlk"]);
        let scope = sign_in_scope(r#"{"rpId": "example.com", "challenge": "Yw", "allowCredentials": [{"type": "public-key", "id": "aWQ="}]}"#);
        assert_eq!(scope, Some(SignInScope { rp_id: "example.com".into(), allowed: vec!["aWQ".into()] }));
        assert_eq!(app_origin(b"x"), format!("android:apk-key-hash:{}", B64URL.encode(Sha256::digest(b"x"))));
    }
}
