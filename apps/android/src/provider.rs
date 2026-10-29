//! PswManager as a credential provider (stage A5, `docs/credential-provider-research.md`):
//! what `PswmCredentialService.kt` and `UnlockActivity.kt` ask of Rust through
//! JNI (`ProviderBridge.kt`). Android often starts the process for the
//! service alone, without the app's window, so this works with or without
//! the app; both share one [Session], so unlocking in either unlocks both.
//!
//! Unlocked here, the database locks again as the app's *In the background*
//! setting says, but never sooner than [GRACE] (a passkey is used right after
//! the unlock); not while the app is in front, whose own rules apply then.
//!
//! What Kotlin is told back is an [Answer] (`Answer.kt` reads it), unless
//! null alone says it (locked).

use crate::background::{native, text, Kotlin, APP};
use jni::objects::{JByteArray, JObject, JString};
use jni::sys::{jboolean, jstring, JNI_FALSE, JNI_TRUE};
use jni::{Env, EnvUnowned};
use pswm_core::provider::{self, Answer, Asker};
use pswm_core::session::Session;
use pswm_core::settings::Settings;
use pswm_core::store::Store;
use pswm_core::vault::LoginPlace;
use pswm_core::webauthn;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;
use std::time::Duration;
use tauri::Manager;
use zeroize::Zeroizing;

/// The one session of this process, the app's and the provider's.
pub fn session() -> Session {
    static SESSION: LazyLock<Session> = LazyLock::new(Session::default);
    SESSION.clone()
}

/// The shortest time a database unlocked for a passkey stays so.
const GRACE: Duration = Duration::from_secs(60);

/// Whether the app's window is in front (the page says so as it shows and hides).
static APP_IN_FRONT: AtomicBool = AtomicBool::new(false);

pub fn app_in_front(shown: bool) {
    APP_IN_FRONT.store(shown, Ordering::SeqCst);
}

/// The waiting lock: each unlock here voids the one before, and so does the
/// app's own (its rules apply then).
static LOCK_LATER: LazyLock<crate::app::Latest> = LazyLock::new(Default::default);

/// The app unlocked, or came to the front: a lock waiting here no longer counts.
pub fn unlocked_in_app() {
    LOCK_LATER.next();
}

/// A Java string argument; `None` when null.
fn optional_text(env: &mut Env, value: &JString) -> Result<Option<String>, String> {
    if value.is_null() {
        return Ok(None);
    }
    text(env, value).map(Some)
}

/// A Java byte array argument; `None` when null.
fn optional_bytes(env: &mut Env, value: &JByteArray) -> Result<Option<Vec<u8>>, String> {
    if value.is_null() {
        return Ok(None);
    }
    env.convert_byte_array(value).map(Some).map_err(|e| e.to_string())
}

/// A Rust string for Java; null if it cannot be made.
fn java_text(env: &mut Env, text: impl AsRef<str>) -> jstring {
    env.new_string(text.as_ref()).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

/// `answer` for Java, which takes a panic for `panicked`.
fn reply(env: &mut Env, answer: std::thread::Result<Result<Answer, Answer>>, panicked: &str) -> jstring {
    let answer = answer.unwrap_or_else(|_| Err(Answer::failed(panicked))).unwrap_or_else(|failed| failed);
    java_text(env, answer.to_json())
}

/// [Answer::Failed] for `why`: `stale` when it starts with [STALE].
fn failure(why: String) -> Answer {
    match why.strip_prefix(STALE) {
        Some(why) => Answer::Failed { why: why.into(), stale: true },
        None => Answer::failed(why),
    }
}

/// Sends a change made here up with the next sync, once the app has started.
fn upload_soon() {
    if let Some(app) = APP.get() {
        crate::editing::upload_soon(app);
    }
}

/// Whether the database syncs with a cloud store (an upload then waits for a network).
fn syncs_with_cloud(env: &mut Env, state: &JString) -> Result<bool, String> {
    with_store(env, state, |store| store.read(|s| s.remote().is_some_and(|r| r.location.cloud().is_some())))
}

/// The answer to a JNI call that should not fail: `false` on a panic.
fn yes(answer: impl FnOnce() -> bool) -> jboolean {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(answer)) {
        Ok(true) => JNI_TRUE,
        _ => JNI_FALSE,
    }
}

#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_isUnlocked(_env: EnvUnowned, _this: JObject) -> jboolean {
    yes(|| session().is_unlocked())
}

/// Whether the unlock offers the fingerprint: as on the app's unlock screen
/// (the setting on, the master password not due); Kotlin checks the phone
/// and the sealed key.
#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_fingerprintAllowed<'local>(
    mut env: EnvUnowned<'local>,
    _this: JObject<'local>,
    state: JString<'local>,
) -> jboolean {
    native(&mut env, JNI_FALSE, |env| yes(|| with_store(env, &state, crate::app::biometric_allowed) == Ok(true)))
}

/// Whether the settings allow screenshots of the app's screens
/// (`Screenshots.kt`, as each screen opens); `false` when it cannot tell.
#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_screenshotsAllowed<'local>(
    mut env: EnvUnowned<'local>,
    _this: JObject<'local>,
    state: JString<'local>,
) -> jboolean {
    native(&mut env, JNI_FALSE, |env| yes(|| with_store(env, &state, |store| Settings::of(store).allow_screenshots()) == Ok(true)))
}

/// Unlocks with the master password (`sealed` null), or with the key sealed
/// for biometric unlock (`sealed`, after the fingerprint): `ok` when
/// unlocked, else why not, for the user; `stale` when the sealed key no
/// longer opens the database (Kotlin then deletes it, as the app does).
/// The key is sealed only by the app (after its own unlock with the password).
#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_unlock<'local>(
    mut env: EnvUnowned<'local>,
    _this: JObject<'local>,
    context: JObject<'local>,
    state: JString<'local>,
    password: JString<'local>,
    sealed: JString<'local>,
) -> jstring {
    native(&mut env, std::ptr::null_mut(), |env| {
        let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            unlock(env, &context, &state, &password, &sealed).map(|()| Answer::Ok(serde_json::Value::Null)).map_err(failure)
        }));
        reply(env, answer, "PswManager could not unlock: try in the app")
    })
}

/// What a failure to open the database starts with when the sealed key is
/// stale (told to Kotlin as [Answer::Failed]'s `stale`).
const STALE: &str = "stale:";

fn unlock(env: &mut Env, context: &JObject, state: &JString, password: &JString, sealed: &JString) -> Result<(), String> {
    if session().is_unlocked() {
        return Ok(()); // the app was unlocked meanwhile
    }
    let (vault, listing) = open_with(env, context, state, password, sealed)?;
    with_store(env, state, |store| {
        if let Some(app) = APP.get() {
            crate::icons::fetch(app, &listing);
        }
        session().set(Some(vault));
        lock_later(Settings::of(store).lock_in_background());
    })?;
    if let Some(app) = APP.get() {
        crate::app::start_sync(app.clone());
    }
    Ok(())
}

/// The database opened with the master password (`sealed` null) or the key
/// sealed for biometric unlock, as [unlock] takes them: the key is checked
/// by opening the file with it. [STALE] first when the sealed key no longer
/// opens it.
fn open_with(env: &mut Env, context: &JObject, state: &JString, password: &JString, sealed: &JString) -> Result<(pswm_core::vault::Vault, pswm_core::vault::Listing), String> {
    Kotlin::install(env, context)?;
    let password = optional_text(env, password)?.map(Zeroizing::new);
    let sealed = optional_text(env, sealed)?.map(Zeroizing::new);
    let with_sealed_key = sealed.is_some();
    let (password, key_file) = match sealed {
        Some(sealed) => {
            let mut secret: crate::app::Secret = serde_json::from_str(&sealed).map_err(|_| "The stored key cannot be read: unlock with the master password")?;
            let key_file = secret.key_file().map_err(|e| e.to_string())?;
            (secret.password.take().map(Zeroizing::new), key_file)
        }
        None => (password, None),
    };
    with_store(env, state, |store| {
        let key_file = match (key_file, crate::app::key_file(store)) {
            (Some(content), _) => Some(content),
            (None, Some(picked)) => {
                let content = pswm_core::documents::read(&picked.uri)?;
                Some(Zeroizing::new(content.ok_or(format!("Cannot read the key file {}: it is gone", picked.name))?))
            }
            (None, None) => None,
        };
        let password = password.as_deref().map(String::as_str).filter(|p| !p.is_empty());
        let opened = crate::app::open_vault(store, password, key_file.as_deref().map(Vec::as_slice)).map_err(|e| {
            if with_sealed_key && e == pswm_core::dbfile::WRONG_KEY {
                format!("{STALE}The database's key has changed: unlock with the new master password")
            } else {
                e
            }
        })?;
        if !with_sealed_key {
            // The master password was typed: the fingerprint counts again for the set days.
            crate::app::password_asked(store);
        }
        Ok(opened)
    })?
}

/// Another password manager imports from PswManager (#153): the database's
/// entries in use as CXF JSON, once the user is checked again with the
/// master password or the sealed key (`ExportActivity.kt`), every time,
/// unlocked or not. The database is opened afresh with that key, so the
/// session (the app's, the provider's) is not touched. `failed` and why
/// when it cannot be (`stale` for a stale sealed key).
#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_export<'local>(
    mut env: EnvUnowned<'local>,
    _this: JObject<'local>,
    context: JObject<'local>,
    state: JString<'local>,
    password: JString<'local>,
    sealed: JString<'local>,
) -> jstring {
    native(&mut env, std::ptr::null_mut(), |env| {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let (vault, _) = open_with(env, &context, &state, &password, &sealed).map_err(failure)?;
            Ok::<_, Answer>(provider::ok_secret(vault.export().as_str())?)
        }))
        .unwrap_or_else(|_| Err("PswManager could not export: try again".into()));
        match result {
            Ok(json) => java_text(env, json.as_str()),
            Err(failed) => java_text(env, failed.to_json()),
        }
    })
}

/// The passkeys for a sign-in request (WebAuthn's `requestJson`), as JSON
/// (`[{id, title, username}]`); null while locked or for a request without a site.
#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_passkeys<'local>(
    mut env: EnvUnowned<'local>,
    _this: JObject<'local>,
    request: JString<'local>,
) -> jstring {
    native(&mut env, std::ptr::null_mut(), |env| {
        let found = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Option<String> {
            let request = text(env, &request).ok()?;
            let scope = webauthn::sign_in_scope(&request)?;
            let offered = session().read(|v| v.passkeys_for(&scope.rp_id, &scope.allowed)).ok()?;
            serde_json::to_string(&offered).ok()
        }));
        match found {
            Ok(Some(json)) => java_text(env, json),
            _ => std::ptr::null_mut(),
        }
    })
}

/// Signs in with the passkey of entry `id`, after the user was verified: the
/// answer (WebAuthn's `AuthenticationResponseJSON`, as text), or `failed` and why.
/// A browser trusted to speak for the site gives its `origin` and
/// `clientDataHash`; an app gives its package and signing certificate.
#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_sign<'local>(
    mut env: EnvUnowned<'local>,
    _this: JObject<'local>,
    id: JString<'local>,
    request: JString<'local>,
    origin: JString<'local>,
    client_data_hash: JByteArray<'local>,
    package: JString<'local>,
    certificate: JByteArray<'local>,
) -> jstring {
    native(&mut env, std::ptr::null_mut(), |env| {
        let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Answer, Answer> {
            let id = optional_text(env, &id)?.ok_or("No passkey chosen")?;
            let request = optional_text(env, &request)?.ok_or("No request")?;
            let caller = caller(env, &origin, &client_data_hash, &package, &certificate)?;
            Ok(Answer::Ok(session().read(|v| v.sign_with_passkey(&id, &request, &caller))??.into()))
        }));
        reply(env, answer, "PswManager could not sign in")
    })
}

/// Where a new passkey for a creation request (WebAuthn's `requestJson`) can
/// go: `{site, excluded, entries: [{id, title, username}], cloud}`;
/// `excluded` when the database has one of the passkeys the site names
/// (`excludeCredentials`: no second one then); `cloud` when the database
/// syncs with a cloud store (the upload waits for a network). `failed` and
/// why while locked or for a request without a site.
#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_newPasskeyChoices<'local>(
    mut env: EnvUnowned<'local>,
    _this: JObject<'local>,
    state: JString<'local>,
    request: JString<'local>,
) -> jstring {
    native(&mut env, std::ptr::null_mut(), |env| {
        let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Answer, Answer> {
            let request = optional_text(env, &request)?.ok_or("No request")?;
            let (rp_id, site) = webauthn::creation_site(&request).ok_or("The request names no site")?;
            let excluded = webauthn::excluded_ids(&request);
            let (excluded, entries) = session().read(|v| {
                let has_one = !excluded.is_empty() && !v.passkeys_for(&rp_id, &excluded).is_empty();
                (has_one, v.entries_for_new_passkey(&rp_id))
            })?;
            let cloud = syncs_with_cloud(env, &state)?;
            Ok(Answer::Ok(serde_json::json!({ "site": site, "excluded": excluded, "entries": entries, "cloud": cloud })))
        }));
        reply(env, answer, "PswManager could not read the request")
    })
}

/// Makes a passkey for a creation request, after the user was verified, and
/// keeps it in entry `id` (or a new entry for the site); the database is
/// saved, and goes up with the next sync. The answer (WebAuthn's
/// `RegistrationResponseJSON`, as text), or `failed` and why. Who asks as for `sign`.
#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_makePasskey<'local>(
    mut env: EnvUnowned<'local>,
    _this: JObject<'local>,
    id: JString<'local>,
    request: JString<'local>,
    origin: JString<'local>,
    client_data_hash: JByteArray<'local>,
    package: JString<'local>,
    certificate: JByteArray<'local>,
) -> jstring {
    native(&mut env, std::ptr::null_mut(), |env| {
        let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Answer, Answer> {
            let id = optional_text(env, &id)?;
            let request = optional_text(env, &request)?.ok_or("No request")?;
            let caller = caller(env, &origin, &client_data_hash, &package, &certificate)?;
            let made = webauthn::make(&request, &caller)?;
            session().with_mut(|v| v.add_passkey(id.as_deref(), &made))?;
            upload_soon();
            Ok(Answer::Ok(made.response.into()))
        }));
        reply(env, answer, "PswManager could not make the passkey")
    })
}

/// Who asks: a browser trusted to speak for the site (its `origin` and
/// `client_data_hash`), or an app (its package and signing certificate).
fn caller(env: &mut Env, origin: &JString, client_data_hash: &JByteArray, package: &JString, certificate: &JByteArray) -> Result<webauthn::Caller, String> {
    Ok(match (optional_text(env, origin)?, optional_bytes(env, client_data_hash)?) {
        (Some(origin), Some(client_data_hash)) => webauthn::Caller::Browser { origin, client_data_hash },
        (Some(_), None) => return Err("The browser did not give its client data".into()),
        (None, _) => {
            let package = optional_text(env, package)?.ok_or("The app asking is unknown")?;
            let certificate = optional_bytes(env, certificate)?.ok_or("The app asking is unknown")?;
            webauthn::Caller::App { origin: webauthn::app_origin(&certificate), package }
        }
    })
}

/// Where a login is for: the site a browser speaks for (its `origin`), or the
/// app asking (its package and signing certificate).
fn login_place(env: &mut Env, origin: &JString, package: &JString, certificate: &JByteArray) -> Result<LoginPlace, String> {
    let origin = optional_text(env, origin)?;
    let package = optional_text(env, package)?.ok_or("The app asking is unknown")?;
    let certificate = optional_bytes(env, certificate)?;
    LoginPlace::of(origin.as_deref(), &package, certificate.as_deref())
}

/// Who an autofill request from `package` (signed with `certificate`) is for
/// ([provider::autofill_asker], with `allowlist`, Google's list of privileged
/// browsers): `ok` with the origin of the site a browser speaks for, or null
/// for the app itself; `failed` when it is no one's (nothing is filled or
/// saved there).
#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_autofillAsker<'local>(
    mut env: EnvUnowned<'local>,
    _this: JObject<'local>,
    allowlist: JString<'local>,
    package: JString<'local>,
    certificate: JByteArray<'local>,
    web_domain: JString<'local>,
    web_scheme: JString<'local>,
) -> jstring {
    native(&mut env, std::ptr::null_mut(), |env| {
        let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Answer, Answer> {
            let allowlist = optional_text(env, &allowlist)?.unwrap_or_default();
            let package = optional_text(env, &package)?.ok_or("The app asking is unknown")?;
            let certificate = optional_bytes(env, &certificate)?.ok_or("The app asking is unknown")?;
            let (domain, scheme) = (optional_text(env, &web_domain)?, optional_text(env, &web_scheme)?);
            match provider::autofill_asker(&allowlist, &package, &certificate, domain.as_deref(), scheme.as_deref()) {
                Some(Asker::App) => Ok(Answer::Ok(serde_json::Value::Null)),
                Some(Asker::Site(origin)) => Ok(Answer::Ok(origin.into())),
                None => Err("The browser shows no https page".into()),
            }
        }));
        reply(env, answer, "PswManager could not tell who asks")
    })
}

/// The logins for the app or site asking, as JSON (`[{id, title, username}]`);
/// null while locked.
#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_logins<'local>(
    mut env: EnvUnowned<'local>,
    _this: JObject<'local>,
    origin: JString<'local>,
    package: JString<'local>,
    certificate: JByteArray<'local>,
) -> jstring {
    native(&mut env, std::ptr::null_mut(), |env| {
        let found = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Option<String> {
            let place = login_place(env, &origin, &package, &certificate).ok()?;
            let offered = session().read(|v| v.logins_for(&place)).ok()?;
            serde_json::to_string(&offered).ok()
        }));
        match found {
            Ok(Some(json)) => java_text(env, json),
            _ => std::ptr::null_mut(),
        }
    })
}

/// Entry `id`'s login (`{username, password}`), when the entry is for the
/// app or site asking (Credential Manager verifies the user first; autofill
/// does not, the database being unlocked); `unconfirmed` and the app's
/// certificate while the user has not confirmed the app for it (`confirmed`:
/// the entry keeps the certificate first, and is saved); `failed` and why
/// while locked, when the entry is gone or is not for who asks.
#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_login<'local>(
    mut env: EnvUnowned<'local>,
    _this: JObject<'local>,
    id: JString<'local>,
    origin: JString<'local>,
    package: JString<'local>,
    certificate: JByteArray<'local>,
    confirmed: jboolean,
) -> jstring {
    native(&mut env, std::ptr::null_mut(), |env| {
        #[derive(serde::Serialize)]
        struct Login<'a> {
            username: &'a str,
            password: &'a str,
        }
        let found = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Zeroizing<String>, Answer> {
            let id = optional_text(env, &id)?.ok_or("No login chosen")?;
            let place = login_place(env, &origin, &package, &certificate)?;
            if confirmed == JNI_TRUE {
                session().with_mut(|v| v.confirm_app(&id, &place))?;
                upload_soon();
            }
            let (username, password) = session().read(|v| v.login(&id, &place))?.map_err(|why| Answer::refused(why, &place))?;
            // One string, wiped once Java has its own copy.
            Ok(provider::ok_secret(&Login { username: &username, password: &password })?)
        }))
        .unwrap_or_else(|_| Err("PswManager could not give the login".into()));
        match found {
            Ok(json) => java_text(env, json.as_str()),
            Err(failed) => java_text(env, failed.to_json()),
        }
    })
}

/// The entries a login an app offers to save can update
/// (`{entries: [{id, title, username}], cloud}`), or `failed` and why.
#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_loginChoices<'local>(
    mut env: EnvUnowned<'local>,
    _this: JObject<'local>,
    state: JString<'local>,
    origin: JString<'local>,
    package: JString<'local>,
    certificate: JByteArray<'local>,
) -> jstring {
    native(&mut env, std::ptr::null_mut(), |env| {
        let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Answer, Answer> {
            let place = login_place(env, &origin, &package, &certificate)?;
            let entries = session().read(|v| v.entries_for_login(&place))?;
            let cloud = syncs_with_cloud(env, &state)?;
            Ok(Answer::Ok(serde_json::json!({ "entries": entries, "cloud": cloud })))
        }));
        reply(env, answer, "PswManager could not read the request")
    })
}

/// Saves a login an app offers: into entry `id` (which must be for who asks,
/// as for `login`; `confirmed` once the user confirmed the app), or a new
/// entry titled `title`; the database goes up with the next sync. `ok` when
/// saved, `unconfirmed` and the app's certificate while the user has not
/// confirmed the app for the entry, else `failed` and why.
#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_saveLogin<'local>(
    mut env: EnvUnowned<'local>,
    _this: JObject<'local>,
    id: JString<'local>,
    origin: JString<'local>,
    package: JString<'local>,
    certificate: JByteArray<'local>,
    title: JString<'local>,
    username: JString<'local>,
    password: JString<'local>,
    confirmed: jboolean,
) -> jstring {
    native(&mut env, std::ptr::null_mut(), |env| {
        let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Answer, Answer> {
            let id = optional_text(env, &id)?;
            let title = optional_text(env, &title)?.unwrap_or_default();
            let username = Zeroizing::new(optional_text(env, &username)?.unwrap_or_default());
            let password = Zeroizing::new(optional_text(env, &password)?.ok_or("No password")?);
            let place = login_place(env, &origin, &package, &certificate)?;
            session()
                .with_mut(|v| v.save_login(id.as_deref(), &place, confirmed == JNI_TRUE, &title, &username, &password))
                .map_err(|why| Answer::refused(why, &place))?;
            upload_soon();
            Ok(Answer::Ok(serde_json::Value::Null))
        }));
        reply(env, answer, "PswManager could not save the password")
    })
}

/// The entries in use that hold `query` in their title, user name or URLs, to
/// pick one for a login no entry is for yet (#270), as JSON
/// (`[{id, title, username}]`); null while locked.
#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_searchLogins<'local>(
    mut env: EnvUnowned<'local>,
    _this: JObject<'local>,
    query: JString<'local>,
) -> jstring {
    native(&mut env, std::ptr::null_mut(), |env| {
        let found = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Option<String> {
            let query = optional_text(env, &query).ok()?.unwrap_or_default();
            let found = session().read(|v| v.login_candidates(&query)).ok()?;
            serde_json::to_string(&found).ok()
        }));
        match found {
            Ok(Some(json)) => java_text(env, json),
            _ => std::ptr::null_mut(),
        }
    })
}

/// Links entry `id`, which the user picked, to the app or site asking (#270;
/// `confirmed` once the user confirmed the app): `ok` when linked (or it was
/// already), `unconfirmed` and the app's certificate till the app is
/// confirmed, else `failed` and why. The database goes up with the next sync.
#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_linkLogin<'local>(
    mut env: EnvUnowned<'local>,
    _this: JObject<'local>,
    id: JString<'local>,
    origin: JString<'local>,
    package: JString<'local>,
    certificate: JByteArray<'local>,
    confirmed: jboolean,
) -> jstring {
    native(&mut env, std::ptr::null_mut(), |env| {
        let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Answer, Answer> {
            let id = optional_text(env, &id)?.ok_or("No entry chosen")?;
            let place = login_place(env, &origin, &package, &certificate)?;
            session().with_mut(|v| v.link_login(&id, &place, confirmed == JNI_TRUE)).map_err(|why| Answer::refused(why, &place))?;
            upload_soon();
            Ok(Answer::Ok(serde_json::Value::Null))
        }));
        reply(env, answer, "PswManager could not link the entry")
    })
}

/// Runs `work` with the app's `Store` when the app runs, else with one on the
/// state file (as the background upload does, with the same caveat: should
/// the app start meanwhile, both write the file whole).
fn with_store<T>(env: &mut Env, state: &JString, work: impl FnOnce(&Store) -> T) -> Result<T, String> {
    if let Some(app) = APP.get() {
        return Ok(work(&app.state::<Store>()));
    }
    let state = text(env, state)?;
    Ok(work(&Store::load(PathBuf::from(state))))
}

/// Locks after `after` (at least [GRACE]; never with `None`) unless unlocked
/// here again first, or the app is in front then.
fn lock_later(after: Option<Duration>) {
    let wait = LOCK_LATER.next();
    let Some(after) = after else { return };
    std::thread::spawn(move || {
        std::thread::sleep(after.max(GRACE));
        if LOCK_LATER.is_latest(wait) && !APP_IN_FRONT.load(Ordering::SeqCst) {
            match APP.get() {
                Some(app) => crate::app::lock_and_tell(app),
                None => session().set(None),
            }
        }
    });
}
