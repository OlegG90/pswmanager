//! What the Android app decides as a credential and autofill provider (#315),
//! kept pure so tests cover it without a phone: who an autofill request is
//! for, and the answers that cross the JNI seam to Kotlin. Kotlin gathers the
//! evidence and reads the answer; the decisions are made here.

use crate::vault::{LoginPlace, UNCONFIRMED};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

/// Who an autofill request is for (#236).
#[derive(Debug, PartialEq, Eq)]
pub enum Asker {
    /// The app itself, by its package and signing certificate.
    App,
    /// The site a privileged browser speaks for: its page's origin.
    Site(String),
}

/// Who an autofill request from `package` (signed with `certificate`, its
/// current one) is for: the page's web domain when a browser on Google's list
/// of privileged apps (`allowlist`) shows it over https (any app can put a
/// domain in its views, so only those speak for a site), else the app. Such a
/// browser speaks for a page only: one over plain http (or with no scheme
/// said), its own screens and a page without a domain are no one's (`None`),
/// so nothing is filled or saved there, rather than as the browser app's.
pub fn autofill_asker(allowlist: &str, package: &str, certificate: &[u8], web_domain: Option<&str>, web_scheme: Option<&str>) -> Option<Asker> {
    if !privileged_browser(allowlist, package, certificate) {
        return Some(Asker::App);
    }
    let https = web_scheme.is_some_and(|scheme| scheme.eq_ignore_ascii_case("https"));
    let domain = web_domain.filter(|domain| https && !domain.is_empty())?;
    Some(Asker::Site(format!("https://{domain}")))
}

/// Whether `package` is a browser on Google's list, signed with one of the
/// certificates listed for it. A list that cannot be read names no one.
fn privileged_browser(allowlist: &str, package: &str, certificate: &[u8]) -> bool {
    let Ok(list) = serde_json::from_str::<Value>(allowlist) else { return false };
    let fingerprint = Sha256::digest(certificate).iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":");
    let apps = list["apps"].as_array().map(Vec::as_slice).unwrap_or_default();
    apps.iter().map(|app| &app["info"]).filter(|info| info["package_name"] == package).any(|info| {
        let signatures = info["signatures"].as_array().map(Vec::as_slice).unwrap_or_default();
        signatures.iter().any(|s| s["cert_fingerprint_sha256"].as_str().is_some_and(|f| f.eq_ignore_ascii_case(&fingerprint)))
    })
}

/// An answer to Kotlin, as one JSON object Kotlin tells apart by its key
/// (`ok`, `unconfirmed` or `failed`), not by prefixes of a string.
#[derive(Debug, PartialEq)]
pub enum Answer {
    /// Done: what was asked for (`null` when there is nothing to give).
    Ok(Value),
    /// The entry is for the app's package but keeps no certificate: the
    /// user confirms the app, whose certificate this is, and is asked again.
    Unconfirmed(String),
    /// Not done, and why, for the user; `stale` when the sealed key no
    /// longer opens the database (Kotlin then deletes it).
    Failed { why: String, stale: bool },
}

impl Answer {
    /// Not done for `why`.
    pub fn failed(why: impl Into<String>) -> Answer {
        Answer::Failed { why: why.into(), stale: false }
    }

    /// Not done for `why`, refused for `place`: [Answer::Unconfirmed] with
    /// the app's certificate when the user has not confirmed the app for the
    /// entry yet.
    pub fn refused(why: String, place: &LoginPlace) -> Answer {
        match place {
            LoginPlace::App { certificate, .. } if why == UNCONFIRMED => Answer::Unconfirmed(certificate.clone()),
            _ => Answer::failed(why),
        }
    }

    pub fn to_json(&self) -> String {
        match self {
            Answer::Ok(value) => json!({ "ok": value }),
            Answer::Unconfirmed(certificate) => json!({ "unconfirmed": certificate }),
            Answer::Failed { why, stale } => json!({ "failed": why, "stale": stale }),
        }
        .to_string()
    }
}

impl From<String> for Answer {
    fn from(why: String) -> Answer {
        Answer::failed(why)
    }
}

impl From<&str> for Answer {
    fn from(why: &str) -> Answer {
        Answer::failed(why)
    }
}

/// [Answer::Ok] with a secret (a login, an export), as JSON wiped once
/// dropped: the secret is never held in a [Value], which is not.
pub fn ok_secret<T: Serialize + ?Sized>(secret: &T) -> Result<Zeroizing<String>, String> {
    #[derive(Serialize)]
    struct Done<'a, T: ?Sized> {
        ok: &'a T,
    }
    serde_json::to_string(&Done { ok: secret }).map(Zeroizing::new).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BROWSER: &str = "org.example.browser";
    const CERTIFICATE: &[u8] = b"the browser's certificate";

    /// A list naming [BROWSER] with [CERTIFICATE]'s fingerprint, in `case`.
    fn allowlist(lowercase: bool) -> String {
        let fingerprint = Sha256::digest(CERTIFICATE).iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":");
        let fingerprint = if lowercase { fingerprint.to_lowercase() } else { fingerprint };
        json!({ "apps": [{ "type": "android", "info": {
            "package_name": BROWSER,
            "signatures": [{ "build": "release", "cert_fingerprint_sha256": fingerprint }],
        } }] })
        .to_string()
    }

    #[test]
    fn a_privileged_browser_speaks_for_its_https_page() {
        let asker = autofill_asker(&allowlist(false), BROWSER, CERTIFICATE, Some("mail.example"), Some("HTTPS"));
        assert_eq!(asker, Some(Asker::Site("https://mail.example".into())));
        let asker = autofill_asker(&allowlist(true), BROWSER, CERTIFICATE, Some("mail.example"), Some("https"));
        assert_eq!(asker, Some(Asker::Site("https://mail.example".into())), "fingerprints match in any case");
    }

    #[test]
    fn a_privileged_browser_off_an_https_page_is_no_one() {
        let list = allowlist(false);
        assert_eq!(autofill_asker(&list, BROWSER, CERTIFICATE, Some("mail.example"), Some("http")), None, "plain http");
        assert_eq!(autofill_asker(&list, BROWSER, CERTIFICATE, Some("mail.example"), None), None, "no scheme said");
        assert_eq!(autofill_asker(&list, BROWSER, CERTIFICATE, None, Some("https")), None, "its own screens");
        assert_eq!(autofill_asker(&list, BROWSER, CERTIFICATE, Some(""), Some("https")), None, "a page without a domain");
    }

    #[test]
    fn any_other_app_is_itself_whatever_domain_it_shows() {
        let list = allowlist(false);
        let other = autofill_asker(&list, "org.example.app", CERTIFICATE, Some("mail.example"), Some("https"));
        assert_eq!(other, Some(Asker::App), "another package");
        let resigned = autofill_asker(&list, BROWSER, b"another certificate", Some("mail.example"), Some("https"));
        assert_eq!(resigned, Some(Asker::App), "the browser's package signed otherwise");
        let unread = autofill_asker("not json", BROWSER, CERTIFICATE, Some("mail.example"), Some("https"));
        assert_eq!(unread, Some(Asker::App), "a list that cannot be read names no browser");
    }

    #[test]
    fn answers_are_told_apart_by_their_key() {
        assert_eq!(Answer::Ok(json!({ "username": "user" })).to_json(), r#"{"ok":{"username":"user"}}"#);
        assert_eq!(Answer::Ok(Value::Null).to_json(), r#"{"ok":null}"#);
        assert_eq!(Answer::failed("The database is locked").to_json(), r#"{"failed":"The database is locked","stale":false}"#);
        let stale = Answer::Failed { why: "The key has changed".into(), stale: true };
        assert_eq!(stale.to_json(), r#"{"failed":"The key has changed","stale":true}"#);
        let secret = ok_secret("{\"a\":1}").unwrap();
        assert_eq!(secret.as_str(), r#"{"ok":"{\"a\":1}"}"#, "a secret is answered as Ok is");
    }

    #[test]
    fn an_unconfirmed_app_is_asked_with_its_certificate() {
        let app = LoginPlace::App { package: "org.example.app".into(), certificate: "android:apk-key-hash:abc".into() };
        assert_eq!(Answer::refused(UNCONFIRMED.into(), &app), Answer::Unconfirmed("android:apk-key-hash:abc".into()));
        assert_eq!(Answer::refused("Not for this app".into(), &app), Answer::failed("Not for this app"));
        let site = LoginPlace::Site("mail.example".into());
        assert_eq!(Answer::refused(UNCONFIRMED.into(), &site), Answer::failed(UNCONFIRMED), "a site has no app to confirm");
    }
}
