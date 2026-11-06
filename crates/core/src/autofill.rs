//! The Android app as a credential and autofill provider (#315, #364): what
//! is offered for a request, the login or passkey given, what is saved back,
//! and when a database unlocked for it locks again. Kotlin gathers the
//! evidence (who asks, what the user picked) and reads the [Answer]; the JNI
//! functions (`apps/android/src/provider.rs`) only carry both across the
//! seam, so the decisions are made, and tested, here without a phone.

use crate::session::Session;
use crate::vault::{EntryChoice, LoginPlace, Vault, UNCONFIRMED};
use crate::webauthn::{self, Caller};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use zeroize::Zeroizing;

/// The shortest time a database unlocked for the provider stays so: a
/// passkey is used right after the unlock.
const GRACE: Duration = Duration::from_secs(60);

/// The provider's side of the one session the app shares (unlocking in
/// either unlocks both).
pub struct Autofill {
    session: Session,
    /// Run after each change saved here: the app sends it up with the next sync.
    changed: Box<dyn Fn() + Send + Sync>,
    /// Shared with the wait for a lock after an unlock here.
    relock: Arc<Relock>,
}

/// When a database unlocked here locks again.
struct Relock {
    /// The latest unlock, here or in the app: a lock waiting since an earlier one no longer counts.
    unlocks: AtomicU64,
    /// Whether the app's window is in front: its own rules apply then.
    app_in_front: AtomicBool,
    /// Locks the database (and tells the app's window, when it runs).
    lock: Box<dyn Fn() + Send + Sync>,
}

/// Who asks, as Kotlin tells it: a browser trusted to speak for a site gives
/// the page's `origin` (and, for a passkey, the hash of the client data it
/// made); any other app its package and current signing `certificate`.
#[derive(Debug, Default, Clone)]
pub struct Asking {
    pub origin: Option<String>,
    pub client_data_hash: Option<Vec<u8>>,
    pub package: Option<String>,
    pub certificate: Option<Vec<u8>>,
}

const UNKNOWN_APP: &str = "The app asking is unknown";

impl Asking {
    /// Where a login is for: the site a browser speaks for, or the app.
    fn place(&self) -> Result<LoginPlace, String> {
        let package = self.package.as_deref().ok_or(UNKNOWN_APP)?;
        LoginPlace::of(self.origin.as_deref(), package, self.certificate.as_deref())
    }

    /// Who signs with a passkey: a browser, with the client data it made, or an app.
    fn caller(&self) -> Result<Caller, String> {
        Ok(match (&self.origin, &self.client_data_hash) {
            (Some(origin), Some(hash)) => Caller::Browser { origin: origin.clone(), client_data_hash: hash.clone() },
            (Some(_), None) => return Err("The browser did not give its client data".into()),
            (None, _) => {
                let package = self.package.clone().ok_or(UNKNOWN_APP)?;
                let certificate = self.certificate.as_deref().ok_or(UNKNOWN_APP)?;
                Caller::App { origin: webauthn::app_origin(certificate), package }
            }
        })
    }
}

/// What the database can offer, from memory: Android asks every provider at
/// once and needs a quick answer.
pub enum Offer<'a> {
    /// The logins for who asks (`[{id, title, username}]`).
    Logins(&'a Asking),
    /// The passkeys for a sign-in request (WebAuthn's `requestJson`), listed as [Offer::Logins].
    Passkeys(&'a str),
    /// The entries in use holding the text in their title, user name or URLs,
    /// to pick one for a login no entry is for yet (#270).
    Search(&'a str),
    /// The entries a login an app offers to save can update: `{entries, cloud}`,
    /// `cloud` when the database syncs with a cloud store (an upload then waits for a network).
    LoginChoices { asking: &'a Asking, cloud: bool },
    /// Where a new passkey for a creation request can go: `{site, excluded,
    /// entries, cloud}`; `excluded` when the database has one of the passkeys
    /// the site names (`excludeCredentials`: no second one then).
    PasskeyChoices { request: &'a str, cloud: bool },
}

/// What is handed to who asks, from the entry the user picked.
pub enum Give<'a> {
    /// The entry's login (`{username, password}`) when it is for who asks;
    /// `confirmed` once the user confirmed the app (the entry keeps its
    /// certificate first, and is saved).
    Login { asking: &'a Asking, confirmed: bool },
    /// The answer to a sign-in request (WebAuthn's `AuthenticationResponseJSON`,
    /// as text), signed with the entry's passkey after the user was verified.
    Passkey { request: &'a str, asking: &'a Asking },
}

/// What is kept in the database, and goes up with the next sync.
pub enum Save<'a> {
    /// A login an app offers: into entry `id` (which must be for who asks, as
    /// for [Give::Login]), or a new entry titled `title`.
    Login { id: Option<&'a str>, asking: &'a Asking, confirmed: bool, title: &'a str, username: &'a str, password: &'a str },
    /// Entry `id`, which the user picked, linked to who asks (#270).
    Link { id: &'a str, asking: &'a Asking, confirmed: bool },
    /// A passkey made for a creation request, after the user was verified,
    /// kept in entry `id` (or a new entry for the site); answered with
    /// WebAuthn's `RegistrationResponseJSON`, as text.
    Passkey { id: Option<&'a str>, request: &'a str, asking: &'a Asking },
}

/// A lock waiting to happen after an unlock here: due after `after`, if
/// [Relock::due] says so then.
#[derive(Debug, PartialEq, Eq)]
struct Wait {
    unlock: u64,
    after: Duration,
}

impl Relock {
    /// The wait for a lock after an unlock here, as the app's *In the
    /// background* setting says (`lock_in_background`; none with `None`), but
    /// never sooner than [GRACE]. Either way, a wait from before no longer counts.
    fn wait(&self, lock_in_background: Option<Duration>) -> Option<Wait> {
        let unlock = self.unlocks.fetch_add(1, Ordering::SeqCst) + 1;
        lock_in_background.map(|after| Wait { unlock, after: after.max(GRACE) })
    }

    /// Whether `wait`, waited out, locks the database now: not when it was
    /// unlocked again since (here or in the app), nor while the app is in front.
    fn due(&self, wait: &Wait) -> bool {
        self.unlocks.load(Ordering::SeqCst) == wait.unlock && !self.app_in_front.load(Ordering::SeqCst)
    }
}

impl Autofill {
    /// The provider on `session`, the app's; `changed` runs after each change
    /// saved here, `lock` when a database unlocked here is due to lock again.
    pub fn new(session: Session, changed: impl Fn() + Send + Sync + 'static, lock: impl Fn() + Send + Sync + 'static) -> Autofill {
        let relock = Relock { unlocks: AtomicU64::new(0), app_in_front: AtomicBool::new(false), lock: Box::new(lock) };
        Autofill { session, changed: Box::new(changed), relock: Arc::new(relock) }
    }

    /// What the database offers for a request; `failed` and why while locked
    /// or for a request without a site.
    pub fn offer(&self, offer: Offer) -> Answer {
        answered(|| match offer {
            Offer::Logins(asking) => {
                let place = asking.place()?;
                listed(self.session.read(|v| v.logins_for(&place))?)
            }
            Offer::Passkeys(request) => {
                let scope = webauthn::sign_in_scope(request).ok_or(NO_SITE)?;
                listed(self.session.read(|v| v.passkeys_for(&scope.rp_id, &scope.allowed))?)
            }
            Offer::Search(query) => listed(self.session.read(|v| v.login_candidates(query))?),
            Offer::LoginChoices { asking, cloud } => {
                let place = asking.place()?;
                let entries = self.session.read(|v| v.entries_for_login(&place))?;
                Ok(Answer::Ok(json!({ "entries": entries, "cloud": cloud })))
            }
            Offer::PasskeyChoices { request, cloud } => {
                let (rp_id, site) = webauthn::creation_site(request).ok_or(NO_SITE)?;
                let excluded = webauthn::excluded_ids(request);
                let (excluded, entries) = self.session.read(|v| {
                    let has_one = !excluded.is_empty() && !v.passkeys_for(&rp_id, &excluded).is_empty();
                    (has_one, v.entries_for_new_passkey(&rp_id))
                })?;
                Ok(Answer::Ok(json!({ "site": site, "excluded": excluded, "entries": entries, "cloud": cloud })))
            }
        })
    }

    /// Entry `id`'s login or passkey answer for who asks; `unconfirmed` and
    /// the app's certificate while the user has not confirmed the app for the
    /// entry; `failed` and why while locked, when the entry is gone or is not
    /// for who asks. Credential Manager verifies the user first; autofill
    /// does not, the database being unlocked.
    pub fn answer(&self, id: &str, give: Give) -> Answer {
        #[derive(Serialize)]
        struct Login<'a> {
            username: &'a str,
            password: &'a str,
        }
        answered(|| match give {
            Give::Login { asking, confirmed } => {
                let place = asking.place()?;
                if confirmed {
                    self.session.with_mut(|v| v.confirm_app(id, &place))?;
                    (self.changed)();
                }
                let (username, password) = self.session.read(|v| v.login(id, &place))?.map_err(|why| Answer::refused(why, &place))?;
                // One string, wiped once Java has its own copy.
                Ok(Answer::secret(&Login { username: &username, password: &password })?)
            }
            Give::Passkey { request, asking } => {
                let caller = asking.caller()?;
                Ok(Answer::Ok(self.session.read(|v| v.sign_with_passkey(id, request, &caller))??.into()))
            }
        })
    }

    /// Keeps a login, a link or a passkey; the database is saved, and goes up
    /// with the next sync. `ok` when kept, `unconfirmed` as for
    /// [Autofill::answer], else `failed` and why.
    pub fn save(&self, save: Save) -> Answer {
        answered(|| {
            let saved = match save {
                Save::Login { id, asking, confirmed, title, username, password } => {
                    let place = asking.place()?;
                    let saved = self.session.with_mut(|v| v.save_login(id, &place, confirmed, title, username, password));
                    saved.map_err(|why| Answer::refused(why, &place))?;
                    Answer::Ok(Value::Null)
                }
                Save::Link { id, asking, confirmed } => {
                    let place = asking.place()?;
                    self.session.with_mut(|v| v.link_login(id, &place, confirmed)).map_err(|why| Answer::refused(why, &place))?;
                    Answer::Ok(Value::Null)
                }
                Save::Passkey { id, request, asking } => {
                    let caller = asking.caller()?;
                    let made = webauthn::make(request, &caller)?;
                    self.session.with_mut(|v| v.add_passkey(id, &made))?;
                    Answer::Ok(made.response.into())
                }
            };
            (self.changed)();
            Ok(saved)
        })
    }

    /// Unlocked here with `vault`: the session (the app's too) has it, and it
    /// locks again as the app's *In the background* setting says
    /// (`lock_in_background`; never with `None`), but never sooner than
    /// [GRACE]; not when unlocked again first, or while the app is in front.
    pub fn unlocked(&self, vault: Vault, lock_in_background: Option<Duration>) {
        self.session.set(Some(vault));
        let Some(wait) = self.relock.wait(lock_in_background) else { return };
        let relock = self.relock.clone();
        std::thread::spawn(move || {
            std::thread::sleep(wait.after);
            if relock.due(&wait) {
                (relock.lock)();
            }
        });
    }

    /// The app unlocked, or came to the front: a lock waiting here no longer counts.
    pub fn unlocked_in_app(&self) {
        self.relock.unlocks.fetch_add(1, Ordering::SeqCst);
    }

    /// The app's window shows or hides.
    pub fn app_in_front(&self, shown: bool) {
        self.relock.app_in_front.store(shown, Ordering::SeqCst);
    }
}

const NO_SITE: &str = "The request names no site";

/// What `work` answers, done or not.
fn answered(work: impl FnOnce() -> Result<Answer, Answer>) -> Answer {
    work().unwrap_or_else(|failed| failed)
}

/// The entries offered, as `[{id, title, username}]`.
fn listed(choices: Vec<EntryChoice>) -> Result<Answer, Answer> {
    serde_json::to_value(choices).map(Answer::Ok).map_err(|e| Answer::failed(e.to_string()))
}

/// Who an autofill request from `package` (signed with `certificate`, its
/// current one) is for, as Kotlin reads it ([autofill_asker], with
/// `allowlist`, Google's list of privileged browsers): `ok` with the origin
/// of the site a browser speaks for, or null for the app itself; `failed`
/// when it is no one's (nothing is filled or saved there).
pub fn who_asks(allowlist: &str, package: &str, certificate: &[u8], web_domain: Option<&str>, web_scheme: Option<&str>) -> Answer {
    match autofill_asker(allowlist, package, certificate, web_domain, web_scheme) {
        Some(Asker::App) => Answer::Ok(Value::Null),
        Some(Asker::Site(origin)) => Answer::Ok(origin.into()),
        None => Answer::failed("The browser shows no https page"),
    }
}

/// Who an autofill request is for (#236).
#[derive(Debug, PartialEq, Eq)]
enum Asker {
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
fn autofill_asker(allowlist: &str, package: &str, certificate: &[u8], web_domain: Option<&str>, web_scheme: Option<&str>) -> Option<Asker> {
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
    let apps = list["apps"].as_array().map(Vec::as_slice).unwrap_or_default();
    let mut listed = apps.iter().map(|app| &app["info"]).filter(|info| info["package_name"] == package).peekable();
    if listed.peek().is_none() {
        return false; // most apps asking are no browser: no fingerprint to make
    }
    let fingerprint = fingerprint(certificate);
    listed.any(|info| {
        let signatures = info["signatures"].as_array().map(Vec::as_slice).unwrap_or_default();
        signatures.iter().any(|s| s["cert_fingerprint_sha256"].as_str().is_some_and(|f| f.eq_ignore_ascii_case(&fingerprint)))
    })
}

/// A certificate's SHA-256 as Google's list writes it: upper-case hex bytes
/// joined by colons.
fn fingerprint(certificate: &[u8]) -> String {
    Sha256::digest(certificate).iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
}

/// An answer to Kotlin, as one JSON object Kotlin tells apart by its key
/// (`ok`, `unconfirmed` or `failed`), not by prefixes of a string.
#[derive(Debug, PartialEq)]
pub enum Answer {
    /// Done: what was asked for (`null` when there is nothing to give).
    Ok(Value),
    /// Done, and what was asked for is a secret (a login, an export): kept
    /// as JSON wiped once dropped, never in a [Value], which is not.
    Secret(Zeroizing<String>),
    /// The entry is for the app's package but keeps no certificate: the
    /// user confirms the app, whose certificate this is, and is asked again.
    Unconfirmed(String),
    /// Not done, and why, for the user; `stale` when the sealed key no
    /// longer opens the database (Kotlin then deletes it).
    Failed { why: String, stale: bool },
}

impl Answer {
    /// [Answer::Secret] with `secret`.
    pub fn secret<T: Serialize + ?Sized>(secret: &T) -> Result<Answer, String> {
        serde_json::to_string(secret).map(|json| Answer::Secret(Zeroizing::new(json))).map_err(|e| e.to_string())
    }

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

    /// Not opened with the key Kotlin gave (an unlock, or the check before an
    /// export) for `why`: `stale` when the key sealed for biometric unlock
    /// (`with_sealed_key`) no longer opens the database.
    pub fn not_opened(why: String, with_sealed_key: bool) -> Answer {
        if with_sealed_key && why == crate::dbfile::WRONG_KEY {
            Answer::Failed { why: "The database's key has changed: unlock with the new master password".into(), stale: true }
        } else {
            Answer::failed(why)
        }
    }

    /// The answer for Kotlin, wiped once dropped (it may hold a secret).
    pub fn to_json(&self) -> Zeroizing<String> {
        Zeroizing::new(match self {
            Answer::Ok(value) => json!({ "ok": value }).to_string(),
            Answer::Secret(secret) => format!(r#"{{"ok":{}}}"#, secret.as_str()),
            Answer::Unconfirmed(certificate) => json!({ "unconfirmed": certificate }).to_string(),
            Answer::Failed { why, stale } => json!({ "failed": why, "stale": stale }).to_string(),
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    const BROWSER: &str = "org.example.browser";
    const CERTIFICATE: &[u8] = b"the browser's certificate";

    /// A list naming [BROWSER] with [CERTIFICATE]'s fingerprint, in `case`.
    fn allowlist(lowercase: bool) -> String {
        let fingerprint = if lowercase { fingerprint(CERTIFICATE).to_lowercase() } else { fingerprint(CERTIFICATE) };
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
    fn a_browser_is_found_among_several_signed_as_its_own_entry_says() {
        let fingerprint = fingerprint(CERTIFICATE);
        let signed = |fingerprint: &str| json!([{ "build": "release", "cert_fingerprint_sha256": fingerprint }]);
        let list = json!({ "apps": [
            { "type": "android", "info": { "package_name": "org.example.other", "signatures": signed(&fingerprint) } },
            { "type": "android", "info": { "package_name": BROWSER, "signatures": signed("00:11") } },
            { "type": "android", "info": { "package_name": BROWSER, "signatures": [{ "build": "userdebug", "cert_fingerprint_sha256": "22:33" }, signed(&fingerprint)[0]] } },
        ] })
        .to_string();
        let asker = autofill_asker(&list, BROWSER, CERTIFICATE, Some("mail.example"), Some("https"));
        assert_eq!(asker, Some(Asker::Site("https://mail.example".into())), "its second entry, second signature");
        let other = autofill_asker(&list, "org.example.other", b"another certificate", Some("mail.example"), Some("https"));
        assert_eq!(other, Some(Asker::App), "another package's fingerprint does not count for it");
    }

    #[test]
    fn answers_are_told_apart_by_their_key() {
        assert_eq!(Answer::Ok(json!({ "username": "user" })).to_json().as_str(), r#"{"ok":{"username":"user"}}"#);
        assert_eq!(Answer::Ok(Value::Null).to_json().as_str(), r#"{"ok":null}"#);
        assert_eq!(Answer::failed("The database is locked").to_json().as_str(), r#"{"failed":"The database is locked","stale":false}"#);
        let stale = Answer::Failed { why: "The key has changed".into(), stale: true };
        assert_eq!(stale.to_json().as_str(), r#"{"failed":"The key has changed","stale":true}"#);
        let login = Answer::secret(&json!({ "username": "user" })).unwrap();
        assert_eq!(login.to_json().as_str(), r#"{"ok":{"username":"user"}}"#, "a secret is answered as Ok is");
        let export = Answer::secret("{\"a\":1}").unwrap();
        assert_eq!(export.to_json().as_str(), r#"{"ok":"{\"a\":1}"}"#, "text stays text");
    }

    #[test]
    fn an_unconfirmed_app_is_asked_with_its_certificate() {
        let app = LoginPlace::App { package: "org.example.app".into(), certificate: "android:apk-key-hash:abc".into() };
        assert_eq!(Answer::refused(UNCONFIRMED.into(), &app), Answer::Unconfirmed("android:apk-key-hash:abc".into()));
        assert_eq!(Answer::refused("Not for this app".into(), &app), Answer::failed("Not for this app"));
        let site = LoginPlace::Site("mail.example".into());
        assert_eq!(Answer::refused(UNCONFIRMED.into(), &site), Answer::failed(UNCONFIRMED), "a site has no app to confirm");
    }

    /// A provider on a fresh session, and how many changes it saved.
    fn provider() -> (Autofill, std::sync::Arc<AtomicU64>) {
        let changes = std::sync::Arc::new(AtomicU64::new(0));
        let counted = changes.clone();
        let changed = move || {
            counted.fetch_add(1, Ordering::SeqCst);
        };
        (Autofill::new(Session::default(), changed, || ()), changes)
    }

    /// A copy of a test database in `dir`, opened.
    fn database(dir: &std::path::Path) -> Vault {
        let path = dir.join("autofill.kdbx");
        std::fs::copy(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sic2kdbx.kdbx"), &path).unwrap();
        Vault::open(&path, Some("test"), None).unwrap()
    }

    /// An app asking, signed with `certificate`.
    fn app(package: &str, certificate: &[u8]) -> Asking {
        Asking { package: Some(package.into()), certificate: Some(certificate.to_vec()), ..Asking::default() }
    }

    /// A privileged browser asking for the page at `origin`.
    fn browser(origin: &str) -> Asking {
        Asking { origin: Some(origin.into()), package: Some(BROWSER.into()), ..Asking::default() }
    }

    /// What an answer gives, when done.
    fn given(answer: Answer) -> Value {
        match answer {
            Answer::Ok(value) => value,
            Answer::Secret(json) => serde_json::from_str(&json).unwrap(),
            other => panic!("not done: {other:?}"),
        }
    }

    /// The ids of the entries an offer lists.
    fn ids(answer: Answer) -> Vec<String> {
        given(answer).as_array().unwrap().iter().map(|e| e["id"].as_str().unwrap().to_string()).collect()
    }

    const LOCKED: &str = "The database is locked";

    #[test]
    fn nothing_is_offered_given_or_saved_while_locked() {
        let (autofill, changes) = provider();
        let mail = app("com.example.mail", b"mail's certificate");
        assert_eq!(autofill.offer(Offer::Logins(&mail)), Answer::failed(LOCKED));
        assert_eq!(autofill.offer(Offer::Search("mail")), Answer::failed(LOCKED));
        assert_eq!(autofill.offer(Offer::LoginChoices { asking: &mail, cloud: true }), Answer::failed(LOCKED));
        let create = r#"{"rp": {"id": "shop.example"}, "user": {"id": "dQ", "name": "bob"}, "challenge": "Yw"}"#;
        assert_eq!(autofill.offer(Offer::PasskeyChoices { request: create, cloud: true }), Answer::failed(LOCKED));
        assert_eq!(autofill.answer("any", Give::Login { asking: &mail, confirmed: false }), Answer::failed(LOCKED));
        let save = Save::Login { id: None, asking: &mail, confirmed: false, title: "Mail", username: "alice", password: "pw" };
        assert_eq!(autofill.save(save), Answer::failed(LOCKED));
        assert_eq!(changes.load(Ordering::SeqCst), 0, "nothing goes up");
    }

    #[test]
    fn a_login_saved_for_an_app_is_offered_and_given_to_it_alone() {
        let dir = tempfile::tempdir().unwrap();
        let (autofill, changes) = provider();
        autofill.unlocked(database(dir.path()), None);
        let mail = app("com.example.mail", b"mail's certificate");
        assert!(ids(autofill.offer(Offer::Logins(&mail))).is_empty());

        let save = Save::Login { id: None, asking: &mail, confirmed: false, title: "Mail", username: "alice", password: "pw-1" };
        assert_eq!(autofill.save(save), Answer::Ok(Value::Null));
        assert_eq!(changes.load(Ordering::SeqCst), 1, "a saved login goes up");
        let offered = ids(autofill.offer(Offer::Logins(&mail)));
        assert_eq!(offered.len(), 1);
        let login = given(autofill.answer(&offered[0], Give::Login { asking: &mail, confirmed: false }));
        assert_eq!(login, json!({ "username": "alice", "password": "pw-1" }));
        assert!(matches!(autofill.answer(&offered[0], Give::Login { asking: &mail, confirmed: false }), Answer::Secret(_)), "kept wiped");

        // Another app of that name, a site, or no one known gets nothing.
        let impostor = app("com.example.mail", b"another certificate");
        assert!(ids(autofill.offer(Offer::Logins(&impostor))).is_empty());
        assert!(matches!(autofill.answer(&offered[0], Give::Login { asking: &impostor, confirmed: true }), Answer::Failed { .. }));
        assert!(matches!(autofill.answer(&offered[0], Give::Login { asking: &browser("https://mail.example"), confirmed: false }), Answer::Failed { .. }));
        let unknown = Asking { package: Some("com.example.mail".into()), ..Asking::default() };
        assert_eq!(autofill.offer(Offer::Logins(&unknown)), Answer::failed(UNKNOWN_APP));
        assert_eq!(autofill.answer(&offered[0], Give::Login { asking: &Asking::default(), confirmed: false }), Answer::failed(UNKNOWN_APP));

        // Offered again to save, with whether the upload waits for a network.
        let choices = given(autofill.offer(Offer::LoginChoices { asking: &mail, cloud: true }));
        assert_eq!((choices["entries"][0]["id"].as_str(), &choices["cloud"]), (Some(offered[0].as_str()), &json!(true)));
        let save = Save::Login { id: Some(&offered[0]), asking: &mail, confirmed: false, title: "", username: "", password: "pw-2" };
        assert_eq!(autofill.save(save), Answer::Ok(Value::Null));
        assert_eq!(given(autofill.answer(&offered[0], Give::Login { asking: &mail, confirmed: false }))["password"], "pw-2");
        assert_eq!(changes.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn an_entry_without_the_apps_certificate_is_given_once_the_user_confirms_the_app() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = database(dir.path());
        // As Keepass2Android keeps it: the app's package among its URLs, no certificate.
        let mut data = crate::edit::EntryData::default();
        data.title = "Bank".into();
        data.username = "alice".into();
        data.password = "pw-1".into();
        data.url = "androidapp://com.example.bank".into();
        let (id, _) = vault.save_entry(None, None, &data, false, &[]).unwrap();
        let (autofill, changes) = provider();
        autofill.unlocked(vault, None);
        let bank = app("com.example.bank", b"the bank's certificate");
        let certificate = webauthn::app_origin(b"the bank's certificate");

        assert_eq!(ids(autofill.offer(Offer::Logins(&bank))), std::slice::from_ref(&id), "offered before it is confirmed");
        assert_eq!(autofill.answer(&id, Give::Login { asking: &bank, confirmed: false }), Answer::Unconfirmed(certificate.clone()));
        let save = Save::Login { id: Some(&id), asking: &bank, confirmed: false, title: "", username: "", password: "pw-x" };
        assert_eq!(autofill.save(save), Answer::Unconfirmed(certificate));
        assert_eq!(changes.load(Ordering::SeqCst), 0, "nothing changed");

        let login = given(autofill.answer(&id, Give::Login { asking: &bank, confirmed: true }));
        assert_eq!(login["password"], "pw-1");
        assert_eq!(changes.load(Ordering::SeqCst), 1, "the entry keeps the certificate, and goes up");
        assert!(ids(autofill.offer(Offer::Logins(&app("com.example.bank", b"another certificate")))).is_empty());
    }

    #[test]
    fn a_picked_entry_is_linked_to_who_asks() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = database(dir.path());
        let mut data = crate::edit::EntryData::default();
        data.title = "Photos".into();
        data.username = "alice".into();
        data.password = "pw-1".into();
        let (id, _) = vault.save_entry(None, None, &data, false, &[]).unwrap();
        let (autofill, changes) = provider();
        autofill.unlocked(vault, None);
        let photos = app("com.example.photos", b"photos' certificate");

        assert!(ids(autofill.offer(Offer::Search("PHOTOS"))).contains(&id));
        let link = |confirmed| Save::Link { id: &id, asking: &photos, confirmed };
        assert_eq!(autofill.save(link(false)), Answer::Unconfirmed(webauthn::app_origin(b"photos' certificate")));
        assert_eq!(changes.load(Ordering::SeqCst), 0);
        assert_eq!(autofill.save(link(true)), Answer::Ok(Value::Null));
        assert_eq!(changes.load(Ordering::SeqCst), 1);
        assert_eq!(ids(autofill.offer(Offer::Logins(&photos))), std::slice::from_ref(&id));
        let site = browser("https://photos.example");
        assert_eq!(autofill.save(Save::Link { id: &id, asking: &site, confirmed: false }), Answer::Ok(Value::Null));
        assert_eq!(ids(autofill.offer(Offer::Logins(&site))), [id]);
    }

    #[test]
    fn a_passkey_is_made_offered_and_signed_with() {
        let dir = tempfile::tempdir().unwrap();
        let (autofill, changes) = provider();
        autofill.unlocked(database(dir.path()), None);
        let shop = app("com.example.shop", b"the shop's certificate");
        let create = r#"{"rp": {"id": "shop.example", "name": "Shop"}, "user": {"id": "dQ", "name": "bob"}, "challenge": "Yw"}"#;

        let choices = given(autofill.offer(Offer::PasskeyChoices { request: create, cloud: false }));
        assert_eq!(choices, json!({ "site": "Shop", "excluded": false, "entries": [], "cloud": false }));
        let made = given(autofill.save(Save::Passkey { id: None, request: create, asking: &shop }));
        let made: Value = serde_json::from_str(made.as_str().unwrap()).unwrap();
        assert_eq!(changes.load(Ordering::SeqCst), 1, "the new passkey goes up");

        // Offered for the site; a second one for the same credential is not made.
        let sign_in = r#"{"rpId": "shop.example", "challenge": "c2lnbg"}"#;
        let offered = ids(autofill.offer(Offer::Passkeys(sign_in)));
        assert_eq!(offered.len(), 1);
        let again = format!(r#"{{"rp": {{"id": "shop.example"}}, "user": {{"id": "dQ", "name": "bob"}}, "challenge": "Yw", "excludeCredentials": [{{"type": "public-key", "id": "{}"}}]}}"#, made["id"].as_str().unwrap());
        assert_eq!(given(autofill.offer(Offer::PasskeyChoices { request: &again, cloud: true }))["excluded"], json!(true));

        let signed = given(autofill.answer(&offered[0], Give::Passkey { request: sign_in, asking: &shop }));
        let signed: Value = serde_json::from_str(signed.as_str().unwrap()).unwrap();
        assert_eq!((&signed["id"], &signed["response"]["userHandle"]), (&made["id"], &json!("dQ")));

        // A browser speaks for the site only with the client data it made.
        let page = browser("https://shop.example");
        assert_eq!(autofill.answer(&offered[0], Give::Passkey { request: sign_in, asking: &page }), Answer::failed("The browser did not give its client data"));
        let page = Asking { client_data_hash: Some(vec![7; 32]), ..page };
        assert!(matches!(autofill.answer(&offered[0], Give::Passkey { request: sign_in, asking: &page }), Answer::Ok(_)));
        assert_eq!(autofill.offer(Offer::Passkeys(r#"{"challenge": "Yw"}"#)), Answer::failed(NO_SITE));
        assert_eq!(autofill.save(Save::Passkey { id: None, request: create, asking: &Asking::default() }), Answer::failed(UNKNOWN_APP));
        assert_eq!(changes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn who_asks_is_answered_as_kotlin_reads_it() {
        let list = allowlist(false);
        assert_eq!(who_asks(&list, BROWSER, CERTIFICATE, Some("mail.example"), Some("https")), Answer::Ok("https://mail.example".into()));
        assert_eq!(who_asks(&list, "org.example.app", CERTIFICATE, Some("mail.example"), Some("https")), Answer::Ok(Value::Null));
        assert_eq!(who_asks(&list, BROWSER, CERTIFICATE, None, None), Answer::failed("The browser shows no https page"));
    }

    #[test]
    fn a_database_unlocked_here_locks_again_but_no_sooner_than_a_minute() {
        let dir = tempfile::tempdir().unwrap();
        let (autofill, _) = provider();
        autofill.unlocked(database(dir.path()), None);
        assert!(autofill.session.is_unlocked());
        let relock = &autofill.relock;
        let wait = relock.wait(Some(Duration::from_secs(10))).unwrap();
        assert_eq!(wait.after, GRACE, "a passkey is used right after the unlock");
        assert!(relock.due(&wait));
        autofill.app_in_front(true);
        assert!(!relock.due(&wait), "the app's own rules apply while it is in front");
        autofill.app_in_front(false);
        autofill.unlocked_in_app();
        assert!(!relock.due(&wait), "the app unlocked since");

        let five_minutes = Duration::from_secs(300);
        let earlier = relock.wait(Some(five_minutes)).unwrap();
        assert_eq!(earlier.after, five_minutes);
        assert_eq!(relock.wait(None), None, "never locked by the setting");
        assert!(!relock.due(&earlier), "an unlock since voids the wait");
    }

    #[test]
    fn only_a_sealed_key_that_no_longer_opens_the_database_is_stale() {
        let stale = Answer::not_opened(crate::dbfile::WRONG_KEY.into(), true);
        assert!(matches!(stale, Answer::Failed { stale: true, .. }));
        assert_eq!(Answer::not_opened(crate::dbfile::WRONG_KEY.into(), false), Answer::failed(crate::dbfile::WRONG_KEY));
        assert_eq!(Answer::not_opened("Choose a database first".into(), true), Answer::failed("Choose a database first"));
    }
}
