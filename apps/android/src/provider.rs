//! PswManager as a credential provider (stage A5, `docs/credential-provider-research.md`):
//! what `PswmCredentialService.kt` and `UnlockActivity.kt` ask of Rust through
//! JNI (`ProviderBridge.kt`). Android often starts the process for the
//! service alone, without the app's window, so this works with or without
//! the app; both share one [Session], so unlocking in either unlocks both.
//!
//! Unlocked here, the database locks again as the app's *In the background*
//! setting says, but never sooner than [GRACE] (a passkey is used right after
//! the unlock); not while the app is in front, whose own rules apply then.

use crate::background::{Kotlin, APP};
use jni::objects::{JByteArray, JObject, JString};
use jni::sys::{jboolean, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;
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
fn optional_text(env: &mut JNIEnv, value: &JString) -> Result<Option<String>, String> {
    if value.is_null() {
        return Ok(None);
    }
    Ok(Some(env.get_string(value).map_err(|e| e.to_string())?.into()))
}

/// A Java byte array argument; `None` when null.
fn optional_bytes(env: &mut JNIEnv, value: &JByteArray) -> Result<Option<Vec<u8>>, String> {
    if value.is_null() {
        return Ok(None);
    }
    env.convert_byte_array(value).map(Some).map_err(|e| e.to_string())
}

/// A Rust string for Java; null if it cannot be made.
fn java_text(env: &mut JNIEnv, text: impl AsRef<str>) -> jstring {
    env.new_string(text.as_ref()).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

/// Whether the database syncs with a cloud store (an upload then waits for a network).
fn syncs_with_cloud(env: &mut JNIEnv, state: &JString) -> Result<bool, String> {
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
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_isUnlocked(_env: JNIEnv, _this: JObject) -> jboolean {
    yes(|| session().is_unlocked())
}

/// Whether the unlock offers the fingerprint: as on the app's unlock screen
/// (the setting on, the master password not due); Kotlin checks the phone
/// and the sealed key.
#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_fingerprintAllowed<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    state: JString<'local>,
) -> jboolean {
    yes(|| with_store(&mut env, &state, crate::app::biometric_allowed) == Ok(true))
}

/// Unlocks with the master password (`sealed` null), or with the key sealed
/// for biometric unlock (`sealed`, after the fingerprint). Null when
/// unlocked, else why not, for the user; [STALE] first when the sealed key no
/// longer opens the database (Kotlin then deletes it, as the app does).
/// The key is sealed only by the app (after its own unlock with the password).
#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_unlock<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    context: JObject<'local>,
    state: JString<'local>,
    password: JString<'local>,
    sealed: JString<'local>,
) -> jstring {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unlock(&mut env, &context, &state, &password, &sealed)))
        .unwrap_or_else(|_| Err("PswManager could not unlock: try in the app".into()));
    match result {
        Ok(()) => std::ptr::null_mut(),
        Err(message) => java_text(&mut env, message),
    }
}

/// What the answer starts with when the sealed key is stale.
const STALE: &str = "stale:";

fn unlock(env: &mut JNIEnv, context: &JObject, state: &JString, password: &JString, sealed: &JString) -> Result<(), String> {
    if session().is_unlocked() {
        return Ok(()); // the app was unlocked meanwhile
    }
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
        let (vault, listing) = crate::app::open_vault(store, password, key_file.as_deref().map(Vec::as_slice)).map_err(|e| {
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
        if let Some(app) = APP.get() {
            crate::icons::fetch(app, &listing);
        }
        session().set(Some(vault));
        lock_later(Settings::of(store).lock_in_background());
        Ok::<(), String>(())
    })??;
    if let Some(app) = APP.get() {
        crate::app::start_sync(app.clone());
    }
    Ok(())
}

/// The passkeys for a sign-in request (WebAuthn's `requestJson`), as JSON
/// (`[{id, title, username}]`); null while locked or for a request without a site.
#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_passkeys<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    request: JString<'local>,
) -> jstring {
    let found = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Option<String> {
        let request: String = env.get_string(&request).ok()?.into();
        let scope = webauthn::sign_in_scope(&request)?;
        let offered = session().read(|v| v.passkeys_for(&scope.rp_id, &scope.allowed)).ok()?;
        serde_json::to_string(&offered).ok()
    }));
    match found {
        Ok(Some(json)) => java_text(&mut env, json),
        _ => std::ptr::null_mut(),
    }
}

/// Signs in with the passkey of entry `id`, after the user was verified: the
/// answer (WebAuthn's `AuthenticationResponseJSON`), or [FAILED] and why.
/// A browser trusted to speak for the site gives its `origin` and
/// `clientDataHash`; an app gives its package and signing certificate.
#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_sign<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    id: JString<'local>,
    request: JString<'local>,
    origin: JString<'local>,
    client_data_hash: JByteArray<'local>,
    package: JString<'local>,
    certificate: JByteArray<'local>,
) -> jstring {
    let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<String, String> {
        let id = optional_text(&mut env, &id)?.ok_or("No passkey chosen")?;
        let request = optional_text(&mut env, &request)?.ok_or("No request")?;
        let caller = caller(&mut env, &origin, &client_data_hash, &package, &certificate)?;
        session().read(|v| v.sign_with_passkey(&id, &request, &caller))?
    }))
    .unwrap_or_else(|_| Err("PswManager could not sign in".into()));
    let text = answer.unwrap_or_else(|why| format!("{FAILED}{why}"));
    java_text(&mut env, text)
}

/// What `sign`'s answer starts with when it failed.
const FAILED: &str = "failed:";

/// Where a new passkey for a creation request (WebAuthn's `requestJson`) can
/// go, as JSON: `{site, excluded, entries: [{id, title, username}], cloud}`;
/// `excluded` when the database has one of the passkeys the site names
/// (`excludeCredentials`: no second one then); `cloud` when the database
/// syncs with a cloud store (the upload waits for a network). `FAILED` and why
/// while locked or for a request without a site.
#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_newPasskeyChoices<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    state: JString<'local>,
    request: JString<'local>,
) -> jstring {
    let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<String, String> {
        let request = optional_text(&mut env, &request)?.ok_or("No request")?;
        let (rp_id, site) = webauthn::creation_site(&request).ok_or("The request names no site")?;
        let excluded = webauthn::excluded_ids(&request);
        let (excluded, entries) = session().read(|v| {
            let has_one = !excluded.is_empty() && !v.passkeys_for(&rp_id, &excluded).is_empty();
            (has_one, v.entries_for_new_passkey(&rp_id))
        })?;
        let cloud = syncs_with_cloud(&mut env, &state)?;
        Ok(serde_json::json!({ "site": site, "excluded": excluded, "entries": entries, "cloud": cloud }).to_string())
    }))
    .unwrap_or_else(|_| Err("PswManager could not read the request".into()));
    let text = answer.unwrap_or_else(|why| format!("{FAILED}{why}"));
    java_text(&mut env, text)
}

/// Makes a passkey for a creation request, after the user was verified, and
/// keeps it in entry `id` (or a new entry for the site); the database is
/// saved, and goes up with the next sync. The answer (WebAuthn's
/// `RegistrationResponseJSON`), or `FAILED` and why. Who asks as for `sign`.
#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_makePasskey<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    id: JString<'local>,
    request: JString<'local>,
    origin: JString<'local>,
    client_data_hash: JByteArray<'local>,
    package: JString<'local>,
    certificate: JByteArray<'local>,
) -> jstring {
    let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<String, String> {
        let id = optional_text(&mut env, &id)?;
        let request = optional_text(&mut env, &request)?.ok_or("No request")?;
        let caller = caller(&mut env, &origin, &client_data_hash, &package, &certificate)?;
        let made = webauthn::make(&request, &caller)?;
        session().with_mut(|v| v.add_passkey(id.as_deref(), &made))?;
        if let Some(app) = APP.get() {
            crate::editing::upload_soon(app);
        }
        Ok(made.response)
    }))
    .unwrap_or_else(|_| Err("PswManager could not make the passkey".into()));
    let text = answer.unwrap_or_else(|why| format!("{FAILED}{why}"));
    java_text(&mut env, text)
}

/// Who asks: a browser trusted to speak for the site (its `origin` and
/// `client_data_hash`), or an app (its package and signing certificate).
fn caller(env: &mut JNIEnv, origin: &JString, client_data_hash: &JByteArray, package: &JString, certificate: &JByteArray) -> Result<webauthn::Caller, String> {
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

/// The logins for the app or site asking, as JSON (`[{id, title, username}]`);
/// null while locked.
#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_logins<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    origin: JString<'local>,
    package: JString<'local>,
) -> jstring {
    let found = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Option<String> {
        let origin = optional_text(&mut env, &origin).ok()?;
        let package = optional_text(&mut env, &package).ok()??;
        let place = LoginPlace::of(origin.as_deref(), &package).ok()?;
        let offered = session().read(|v| v.logins_for(&place)).ok()?;
        serde_json::to_string(&offered).ok()
    }));
    match found {
        Ok(Some(json)) => java_text(&mut env, json),
        _ => std::ptr::null_mut(),
    }
}

/// Entry `id`'s login, as JSON (`{username, password}`), after the user was
/// verified; null while locked or when the entry is gone.
#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_login<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    id: JString<'local>,
) -> jstring {
    #[derive(serde::Serialize)]
    struct Login<'a> {
        username: &'a str,
        password: &'a str,
    }
    let found = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Option<Zeroizing<String>> {
        let id = optional_text(&mut env, &id).ok()??;
        let (username, password) = session().with(|v| v.login(&id)).ok()?;
        // One string, wiped once Java has its own copy.
        serde_json::to_string(&Login { username: &username, password: &password }).ok().map(Zeroizing::new)
    }));
    match found {
        Ok(Some(json)) => java_text(&mut env, json.as_str()),
        _ => std::ptr::null_mut(),
    }
}

/// The entries a login an app offers to save can update, as JSON
/// (`{entries: [{id, title, username}], cloud}`), or `FAILED` and why.
#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_loginChoices<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    state: JString<'local>,
    origin: JString<'local>,
    package: JString<'local>,
) -> jstring {
    let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<String, String> {
        let origin = optional_text(&mut env, &origin)?;
        let package = optional_text(&mut env, &package)?.ok_or("The app asking is unknown")?;
        let place = LoginPlace::of(origin.as_deref(), &package)?;
        let entries = session().read(|v| v.entries_for_login(&place))?;
        let cloud = syncs_with_cloud(&mut env, &state)?;
        Ok(serde_json::json!({ "entries": entries, "cloud": cloud }).to_string())
    }))
    .unwrap_or_else(|_| Err("PswManager could not read the request".into()));
    let text = answer.unwrap_or_else(|why| format!("{FAILED}{why}"));
    java_text(&mut env, text)
}

/// Saves a login an app offers: into entry `id`, or a new entry titled
/// `title`; the database goes up with the next sync. Empty when saved, else
/// `FAILED` and why.
#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_saveLogin<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    id: JString<'local>,
    origin: JString<'local>,
    package: JString<'local>,
    title: JString<'local>,
    username: JString<'local>,
    password: JString<'local>,
) -> jstring {
    let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<(), String> {
        let id = optional_text(&mut env, &id)?;
        let origin = optional_text(&mut env, &origin)?;
        let package = optional_text(&mut env, &package)?.ok_or("The app asking is unknown")?;
        let title = optional_text(&mut env, &title)?.unwrap_or_default();
        let username = Zeroizing::new(optional_text(&mut env, &username)?.unwrap_or_default());
        let password = Zeroizing::new(optional_text(&mut env, &password)?.ok_or("No password")?);
        let place = LoginPlace::of(origin.as_deref(), &package)?;
        session().with_mut(|v| v.save_login(id.as_deref(), &place, &title, &username, &password))?;
        if let Some(app) = APP.get() {
            crate::editing::upload_soon(app);
        }
        Ok(())
    }))
    .unwrap_or_else(|_| Err("PswManager could not save the password".into()));
    let text = match answer {
        Ok(()) => String::new(),
        Err(why) => format!("{FAILED}{why}"),
    };
    java_text(&mut env, text)
}

/// Runs `work` with the app's `Store` when the app runs, else with one on the
/// state file (as the background upload does, with the same caveat: should
/// the app start meanwhile, both write the file whole).
fn with_store<T>(env: &mut JNIEnv, state: &JString, work: impl FnOnce(&Store) -> T) -> Result<T, String> {
    if let Some(app) = APP.get() {
        return Ok(work(&app.state::<Store>()));
    }
    let state: String = env.get_string(state).map_err(|e| e.to_string())?.into();
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
