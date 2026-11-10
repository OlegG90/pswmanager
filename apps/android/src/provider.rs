//! PswManager as a credential provider (stage A5, `docs/credential-provider-research.md`):
//! what `PswmCredentialService.kt` and `UnlockActivity.kt` ask of Rust through
//! JNI (`ProviderBridge.kt`). Android often starts the process for the
//! service alone, without the app's window, so this works with or without
//! the app; both share one [Session], so unlocking in either unlocks both.
//!
//! What is offered, given and saved, and when a database unlocked here locks
//! again, is the core's [Autofill]; these functions only read Kotlin's
//! arguments, hand it what the app has (the settings, the key file, the
//! sync) and answer with its [Answer] (`Answer.kt` reads it), unless null
//! alone says it (locked).

use crate::background::{native, text, Kotlin, APP};
use jni::objects::{JByteArray, JObject, JString};
use jni::sys::{jboolean, jstring, JNI_FALSE, JNI_TRUE};
use jni::{Env, EnvUnowned};
use pswm_core::autofill::{self, Answer, Asking, Autofill, Give, Offer, Save};
use pswm_core::session::Session;
use pswm_core::settings::Settings;
use pswm_core::store::Store;
use std::path::PathBuf;
use std::sync::LazyLock;
use tauri::Manager;
use zeroize::Zeroizing;

/// The one session of this process, the app's and the provider's.
pub fn session() -> Session {
    static SESSION: LazyLock<Session> = LazyLock::new(Session::default);
    SESSION.clone()
}

/// The provider on that session: a change saved here goes up with the next
/// sync, once the app has started; a lock due tells the app's window too.
fn autofill() -> &'static Autofill {
    static AUTOFILL: LazyLock<Autofill> = LazyLock::new(|| {
        let changed = || {
            if let Some(app) = APP.get() {
                crate::syncing::upload_soon(app);
            }
        };
        let lock = || match APP.get() {
            Some(app) => crate::app::lock_and_tell(app),
            None => session().set(None),
        };
        Autofill::new(session(), changed, lock)
    });
    &AUTOFILL
}

/// The app's window shows or hides (the page says so).
pub fn app_in_front(shown: bool) {
    autofill().app_in_front(shown);
}

/// The app unlocked, or came to the front: a lock waiting here no longer counts.
pub fn unlocked_in_app() {
    autofill().unlocked_in_app();
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

/// What `work` answers, for Java; a panic is taken for `panicked`.
fn reply(env: &mut Env, panicked: &str, work: impl FnOnce(&mut Env) -> Result<Answer, Answer>) -> jstring {
    let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(env))).unwrap_or_else(|_| Err(Answer::failed(panicked)));
    java_text(env, answer.unwrap_or_else(|failed| failed).to_json().as_str())
}

/// What an offer lists, as JSON; null for anything else (locked), or a panic.
fn listed(env: &mut Env, offer: impl FnOnce(&mut Env) -> Result<Answer, String>) -> jstring {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| offer(env))) {
        Ok(Ok(Answer::Ok(list))) => java_text(env, list.to_string()),
        _ => std::ptr::null_mut(),
    }
}

/// Who asks, from Kotlin's arguments ([Asking]).
fn asking(env: &mut Env, origin: &JString, client_data_hash: Option<&JByteArray>, package: &JString, certificate: &JByteArray) -> Result<Asking, String> {
    Ok(Asking {
        origin: optional_text(env, origin)?,
        client_data_hash: match client_data_hash {
            Some(hash) => optional_bytes(env, hash)?,
            None => None,
        },
        package: optional_text(env, package)?,
        certificate: optional_bytes(env, certificate)?,
    })
}

/// Whether the database syncs with a cloud store (an upload then waits for a network).
fn syncs_with_cloud(env: &mut Env, state: &JString) -> Result<bool, String> {
    with_store(env, state, |store| store.read(|s| s.syncs_with_cloud()))
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
        reply(env, "PswManager could not unlock: try in the app", |env| {
            unlock(env, &context, &state, &password, &sealed).map(|()| Answer::Ok(serde_json::Value::Null))
        })
    })
}

fn unlock(env: &mut Env, context: &JObject, state: &JString, password: &JString, sealed: &JString) -> Result<(), Answer> {
    if session().is_unlocked() {
        return Ok(()); // the app was unlocked meanwhile
    }
    let (vault, listing) = open_with(env, context, state, password, sealed)?;
    with_store(env, state, |store| {
        if let Some(app) = APP.get() {
            crate::icons::fetch(app, &listing);
        }
        autofill().unlocked(vault, Settings::of(store).lock_in_background());
    })?;
    if let Some(app) = APP.get() {
        crate::syncing::start(app.clone());
    }
    Ok(())
}

/// The database opened with the master password (`sealed` null) or the key
/// sealed for biometric unlock, as [unlock] takes them: the key is checked
/// by opening the file with it. Failed as `stale` when the sealed key no
/// longer opens it.
fn open_with(env: &mut Env, context: &JObject, state: &JString, password: &JString, sealed: &JString) -> Result<(pswm_core::vault::Vault, pswm_core::vault::Listing), Answer> {
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
        let opened = crate::app::open_vault(store, password, key_file.as_deref().map(Vec::as_slice)).map_err(|e| Answer::not_opened(e, with_sealed_key))?;
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
        reply(env, "PswManager could not export: try again", |env| {
            let (vault, _) = open_with(env, &context, &state, &password, &sealed)?;
            Ok(Answer::secret(vault.export().as_str())?)
        })
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
        listed(env, |env| {
            let request = text(env, &request)?;
            Ok(autofill().offer(Offer::Passkeys(&request)))
        })
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
        reply(env, "PswManager could not sign in", |env| {
            let id = optional_text(env, &id)?.ok_or("No passkey chosen")?;
            let request = optional_text(env, &request)?.ok_or("No request")?;
            let asking = asking(env, &origin, Some(&client_data_hash), &package, &certificate)?;
            Ok(autofill().answer(&id, Give::Passkey { request: &request, asking: &asking }))
        })
    })
}

/// Where a new passkey for a creation request (WebAuthn's `requestJson`) can
/// go ([Offer::PasskeyChoices]): `{site, excluded, entries: [{id, title,
/// username}], cloud}`; `failed` and why while locked or for a request
/// without a site.
#[no_mangle]
pub extern "system" fn Java_io_github_olegg90_pswmanager_ProviderBridge_newPasskeyChoices<'local>(
    mut env: EnvUnowned<'local>,
    _this: JObject<'local>,
    state: JString<'local>,
    request: JString<'local>,
) -> jstring {
    native(&mut env, std::ptr::null_mut(), |env| {
        reply(env, "PswManager could not read the request", |env| {
            let request = optional_text(env, &request)?.ok_or("No request")?;
            let cloud = syncs_with_cloud(env, &state)?;
            Ok(autofill().offer(Offer::PasskeyChoices { request: &request, cloud }))
        })
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
        reply(env, "PswManager could not make the passkey", |env| {
            let id = optional_text(env, &id)?;
            let request = optional_text(env, &request)?.ok_or("No request")?;
            let asking = asking(env, &origin, Some(&client_data_hash), &package, &certificate)?;
            Ok(autofill().save(Save::Passkey { id: id.as_deref(), request: &request, asking: &asking }))
        })
    })
}

/// Who an autofill request from `package` (signed with `certificate`) is for
/// ([autofill::who_asks], with `allowlist`, Google's list of privileged
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
        reply(env, "PswManager could not tell who asks", |env| {
            let allowlist = optional_text(env, &allowlist)?.unwrap_or_default();
            let package = optional_text(env, &package)?.ok_or("The app asking is unknown")?;
            let certificate = optional_bytes(env, &certificate)?.ok_or("The app asking is unknown")?;
            let (domain, scheme) = (optional_text(env, &web_domain)?, optional_text(env, &web_scheme)?);
            Ok(autofill::who_asks(&allowlist, &package, &certificate, domain.as_deref(), scheme.as_deref()))
        })
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
        listed(env, |env| {
            let asking = asking(env, &origin, None, &package, &certificate)?;
            Ok(autofill().offer(Offer::Logins(&asking)))
        })
    })
}

/// Entry `id`'s login (`{username, password}`), when the entry is for the
/// app or site asking ([Give::Login]); `unconfirmed` and the app's
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
        reply(env, "PswManager could not give the login", |env| {
            let id = optional_text(env, &id)?.ok_or("No login chosen")?;
            let asking = asking(env, &origin, None, &package, &certificate)?;
            Ok(autofill().answer(&id, Give::Login { asking: &asking, confirmed: confirmed == JNI_TRUE }))
        })
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
        reply(env, "PswManager could not read the request", |env| {
            let asking = asking(env, &origin, None, &package, &certificate)?;
            let cloud = syncs_with_cloud(env, &state)?;
            Ok(autofill().offer(Offer::LoginChoices { asking: &asking, cloud }))
        })
    })
}

/// Saves a login an app offers ([Save::Login]): into entry `id` (`confirmed`
/// once the user confirmed the app), or a new entry titled `title`; the
/// database goes up with the next sync. `ok` when saved, `unconfirmed` and
/// the app's certificate while the user has not confirmed the app for the
/// entry, else `failed` and why.
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
        reply(env, "PswManager could not save the password", |env| {
            let id = optional_text(env, &id)?;
            let title = optional_text(env, &title)?.unwrap_or_default();
            let username = Zeroizing::new(optional_text(env, &username)?.unwrap_or_default());
            let password = Zeroizing::new(optional_text(env, &password)?.ok_or("No password")?);
            let asking = asking(env, &origin, None, &package, &certificate)?;
            let confirmed = confirmed == JNI_TRUE;
            Ok(autofill().save(Save::Login { id: id.as_deref(), asking: &asking, confirmed, title: &title, username: &username, password: &password }))
        })
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
        listed(env, |env| {
            let query = optional_text(env, &query)?.unwrap_or_default();
            Ok(autofill().offer(Offer::Search(&query)))
        })
    })
}

/// Links entry `id`, which the user picked, to the app or site asking (#270,
/// [Save::Link]; `confirmed` once the user confirmed the app): `ok` when
/// linked (or it was already), `unconfirmed` and the app's certificate till
/// the app is confirmed, else `failed` and why. The database goes up with the
/// next sync.
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
        reply(env, "PswManager could not link the entry", |env| {
            let id = optional_text(env, &id)?.ok_or("No entry chosen")?;
            let asking = asking(env, &origin, None, &package, &certificate)?;
            Ok(autofill().save(Save::Link { id: &id, asking: &asking, confirmed: confirmed == JNI_TRUE }))
        })
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
