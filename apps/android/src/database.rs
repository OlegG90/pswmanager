//! The settings kept in the database file itself (#193), as on Windows
//! (`docs/spec.md`, *Database settings*): its name, description and default
//! user name, the history limits, the encryption, and the master password and
//! key file. Each change is saved to the working copy and goes up like an edit
//! (a new key at once).

use crate::app::{self, off_main, Status};
use crate::biometric::Biometric;
use crate::documents::{Documents, Picked};
use crate::editing::upload_soon;
use pswm_core::documents::DocumentStore;
use pswm_core::edit;
use pswm_core::encryption;
use pswm_core::session::Session;
use pswm_core::store::Store;
use pswm_core::vault::{self, DatabaseSettings, Vault};
use serde::Deserialize;
use tauri::{AppHandle, Manager, State, Wry};
use zeroize::Zeroizing;

/// The settings kept in the open database's file.
#[tauri::command(async)]
pub fn database_settings(session: State<Session>) -> Result<DatabaseSettings, String> {
    session.read(Vault::settings)
}

/// Makes `change` to the open database, which saves it; it goes up like an
/// edit. The settings as saved.
fn saved(app: &AppHandle, session: &Session, change: impl FnOnce(&mut Vault) -> Result<(), String>) -> Result<DatabaseSettings, String> {
    let settings = session.with_mut(|v| {
        change(v)?;
        Ok(v.settings())
    })?;
    upload_soon(app);
    Ok(settings)
}

/// Changes a setting kept in the open database's file; the unlock screen
/// shows the new name and description from now on.
#[tauri::command(async)]
pub fn set_database_setting(app: AppHandle, session: State<Session>, setting: edit::Setting, value: String) -> Result<DatabaseSettings, String> {
    let settings = saved(&app, &session, |v| v.set_setting(setting, &value))?;
    let _ = app.state::<Store>().update_if(|s| s.current_mut().is_some_and(|k| k.remember(&settings.name, &settings.description)));
    Ok(settings)
}

/// How many old versions these history limits (-1: none) would remove, to
/// ask before lowering one.
#[tauri::command(async)]
pub fn history_limits_preview(session: State<Session>, max_items: isize, max_size: isize) -> Result<usize, String> {
    session.read(|v| v.versions_over_limits(max_items, max_size))
}

/// Sets the history limits (-1: none); every entry's history is trimmed to them.
#[tauri::command(async)]
pub fn set_history_limits(app: AppHandle, session: State<Session>, max_items: isize, max_size: isize) -> Result<DatabaseSettings, String> {
    saved(&app, &session, |v| v.set_history_limits(max_items, max_size))
}

/// How long unlocking takes on this phone with this encryption, in
/// milliseconds; as slow as an unlock.
#[tauri::command(async)]
pub fn encryption_unlock_time(encryption: encryption::Encryption) -> Result<u64, String> {
    encryption::unlock_time(&encryption).map(|time| time.as_millis() as u64)
}

/// Gives the open database another cipher and / or key derivation.
#[tauri::command(async)]
pub fn set_encryption(app: AppHandle, session: State<Session>, encryption: encryption::Encryption) -> Result<DatabaseSettings, String> {
    saved(&app, &session, |v| v.set_encryption(&encryption))
}

/// The key file the database is to have after a key change.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum NewKeyFile {
    /// The one it has now, if any.
    Keep,
    None,
    /// One picked for it ([pick_new_key_file]).
    Picked { uri: String, name: String },
}

impl NewKeyFile {
    /// The key file it means, `now` being the one the database has.
    fn chosen(self, now: Option<Picked>) -> Option<Picked> {
        match self {
            NewKeyFile::Keep => now,
            NewKeyFile::None => None,
            NewKeyFile::Picked { uri, name } => Some(Picked { uri, name }),
        }
    }
}

/// A key file for the database, picked with Android's picker (its access is
/// kept); `None` when cancelled. It is used once the key is changed to it.
#[tauri::command]
pub async fn pick_new_key_file(app: AppHandle) -> Result<Option<Picked>, String> {
    off_main(move || app.state::<Documents<Wry>>().pick_file()).await
}

/// Gives the open database a new master password and / or key file, after
/// the current master password (with the key file it has now) proved right.
/// An empty password means none. A synced database syncs first and goes up
/// at once after. The phone unlocks with the new key file from then on, and
/// the key sealed for fingerprint unlock (the old one) goes: it is sealed
/// again at the next unlock with the password.
#[tauri::command]
pub async fn change_master_key(app: AppHandle, current: String, password: String, key_file: NewKeyFile) -> Result<Status, String> {
    let (current, password) = (Zeroizing::new(current), Zeroizing::new(password));
    off_main(move || {
        let store = app.state::<Store>();
        let read = |picked: &Picked| read_key_file(&app, picked);
        let now = app::key_file(&store);
        let now_content = now.as_ref().map(read).transpose()?;
        let current = key(&current, now_content.as_ref())?;
        let next = key_file.chosen(now);
        let next_content = next.as_ref().map(read).transpose()?;
        let new = if password.is_empty() && next.is_none() { None } else { Some(key(&password, next_content.as_ref())?) };
        app::sync_first(&app).map_err(|e| format!("{e}; the key is unchanged"))?;
        let session = app.state::<Session>();
        session.with_mut(|v| v.change_key_to(&current, new))?;
        match &next {
            Some(picked) => app::remember_key_file(&store, picked)?,
            None => app::forget_key_file(&store)?,
        }
        let _ = app.state::<Biometric<Wry>>().forget();
        app::start_sync(app.clone());
        Ok(app::status_of(&store, &session))
    })
    .await
}

/// A key file's content, read once.
fn read_key_file(app: &AppHandle, picked: &Picked) -> Result<Zeroizing<Vec<u8>>, String> {
    let content = app.state::<Documents<Wry>>().read(&picked.uri)?;
    content.map(Zeroizing::new).ok_or(format!("Cannot read the key file {}: it is gone", picked.name))
}

/// The key a typed password (empty: none) and a key file's content make.
fn key(password: &str, file: Option<&Zeroizing<Vec<u8>>>) -> Result<vault::DatabaseKey, String> {
    let mut file = file.map(|f| f.as_slice());
    vault::key_reading(Some(password).filter(|p| !p.is_empty()), file.as_mut().map(|f| f as &mut dyn std::io::Read))
}

/// The key another device changed the database to, given here: the remote
/// file is synced with it (it is kept to try only while it opens something).
/// Once the database is on it, the phone unlocks with that key file, and the
/// key sealed for fingerprint unlock (the old one) goes.
#[tauri::command]
pub async fn enter_other_key(app: AppHandle, password: String, key_file: NewKeyFile) -> Result<Status, String> {
    let password = Zeroizing::new(password);
    off_main(move || {
        let store = app.state::<Store>();
        let next = key_file.chosen(app::key_file(&store));
        let content = next.as_ref().map(|p| read_key_file(&app, p)).transpose()?;
        let given = key(&password, content.as_ref())?;
        let session = app.state::<Session>();
        session.with_mut(|v| v.remember_key(given.clone()))?;
        if let Err(e) = app::sync_first(&app) {
            // A key that opened nothing is not kept.
            session.with_mut(|v| v.forget_key(&given))?;
            return Err(if e == app::OTHER_KEY { "This master password or key file does not open it either".into() } else { e });
        }
        if session.read(|v| v.uses_key(&given))?? {
            match &next {
                Some(picked) => app::remember_key_file(&store, picked)?,
                None => app::forget_key_file(&store)?,
            }
            let _ = app.state::<Biometric<Wry>>().forget();
        }
        Ok(app::status_of(&store, &session))
    })
    .await
}
