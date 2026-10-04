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
use jni::objects::{JObject, JString};
use jni::sys::{jboolean, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;
use pswm_core::session::Session;
use pswm_core::settings::Settings;
use pswm_core::store::Store;
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
        Err(message) => env.new_string(message).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut()),
    }
}

/// What the answer starts with when the sealed key is stale.
const STALE: &str = "stale:";

fn unlock(env: &mut JNIEnv, context: &JObject, state: &JString, password: &JString, sealed: &JString) -> Result<(), String> {
    if session().is_unlocked() {
        return Ok(()); // the app was unlocked meanwhile
    }
    Kotlin::install(env, context)?;
    let text = |env: &mut JNIEnv, value: &JString| -> Result<Option<Zeroizing<String>>, String> {
        if value.is_null() {
            return Ok(None);
        }
        let value: String = env.get_string(value).map_err(|e| e.to_string())?.into();
        Ok(Some(Zeroizing::new(value)))
    };
    let password = text(env, password)?;
    let sealed = text(env, sealed)?;
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
