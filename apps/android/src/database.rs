//! The settings kept in the database file itself (#193), as on Windows
//! (`docs/spec.md`, *Database settings*): its name, description and default
//! user name, the history limits and the encryption. Each change is saved to
//! the working copy and goes up like an edit.

use crate::editing::upload_soon;
use pswm_core::edit;
use pswm_core::encryption;
use pswm_core::session::Session;
use pswm_core::store::Store;
use pswm_core::vault::{DatabaseSettings, Vault};
use tauri::{AppHandle, Manager, State};

/// The settings kept in the open database's file.
#[tauri::command(async)]
pub fn database_settings(session: State<Session>) -> Result<DatabaseSettings, String> {
    session.read(Vault::settings)
}

/// Changes a setting kept in the open database's file; the unlock screen
/// shows the new name and description from now on.
#[tauri::command(async)]
pub fn set_database_setting(app: AppHandle, session: State<Session>, setting: edit::Setting, value: String) -> Result<DatabaseSettings, String> {
    let settings = session.with_mut(|v| {
        v.set_setting(setting, &value)?;
        Ok(v.settings())
    })?;
    let _ = app.state::<Store>().update_if(|s| s.current_mut().is_some_and(|k| k.remember(&settings.name, &settings.description)));
    upload_soon(&app);
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
    let settings = session.with_mut(|v| {
        v.set_history_limits(max_items, max_size)?;
        Ok(v.settings())
    })?;
    upload_soon(&app);
    Ok(settings)
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
    let settings = session.with_mut(|v| {
        v.set_encryption(&encryption)?;
        Ok(v.settings())
    })?;
    upload_soon(&app);
    Ok(settings)
}
