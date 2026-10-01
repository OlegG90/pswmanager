//! The app's state and commands: the database on this phone (one at a time,
//! `docs/spec-android.md`), unlocking it, and reading and copying its entries.
//! The core does the work, on a working copy in the app's private storage.

use crate::clipboard::Clipboard;
use crate::documents::{self, Documents};
use pswm_core::remote::Location;
use pswm_core::session::Session;
use pswm_core::store::Store;
use pswm_core::sync;
use pswm_core::vault::{EntryDetail, Listing, Vault};
use pswm_core::otp;
use serde::Serialize;
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Builder, Emitter, Manager, State, Wry};
use zeroize::Zeroizing;

/// How long a copied value stays on the clipboard (a setting later).
const CLEAR_AFTER: Duration = Duration::from_secs(20);

pub fn setup(builder: Builder<Wry>) -> Builder<Wry> {
    builder
        .plugin(documents::init())
        .plugin(crate::clipboard::init())
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            app.manage(Store::load(data.join("pswm.json")));
            app.manage(Session::default());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            status,
            open_local_file,
            forget_database,
            unlock,
            lock,
            listing,
            entry,
            reveal,
            copy_field,
            totp,
            copy_totp,
            sync_now
        ])
}

/// What the unlock screen shows about the database, if there is one.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    database: Option<Database>,
    unlocked: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Database {
    title: String,
    description: String,
    /// Where it syncs with, for people.
    synced_with: Option<String>,
}

/// What the last sync did, for the status line (event `synced`).
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Synced {
    text: String,
    problem: bool,
    /// The entries changed: the list is read again.
    changed: bool,
}

/// The plugins wait for Android's main thread, so they are never called from it.
async fn off_main<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work).await.map_err(|e| e.to_string())?
}

fn status_of(store: &Store, session: &Session) -> Status {
    let database = store.read(|s| {
        s.current().map(|k| Database {
            title: k.title(),
            description: k.description.clone().unwrap_or_default(),
            synced_with: k.remote.as_ref().map(|r| r.location.describe()),
        })
    });
    Status { database, unlocked: session.is_unlocked() }
}

#[tauri::command]
fn status(store: State<Store>, session: State<Session>) -> Status {
    status_of(&store, &session)
}

/// Picks a `.kdbx` on the phone and makes it the database: its working copy is
/// downloaded into the app's storage, and it syncs with the file from then on.
#[tauri::command]
async fn open_local_file(app: AppHandle) -> Result<Option<Status>, String> {
    off_main(move || {
        let Some(picked) = app.state::<Documents<Wry>>().pick_file()? else { return Ok(None) };
        let store = app.state::<Store>();
        let taken: Vec<PathBuf> = store.read(|s| s.databases.iter().map(|k| k.file.clone()).collect());
        let local = sync::free_path(&store.dir().join("databases"), &picked.name, &taken);
        sync::start(&store, Location::Document { uri: picked.uri, name: picked.name }, local)?;
        Ok(Some(status_of(&store, &app.state::<Session>())))
    })
    .await
}

/// Forgets the database (locking it first); its working copy goes too, the
/// file it synced with stays where it is.
#[tauri::command]
fn forget_database(store: State<Store>, session: State<Session>) -> Result<Status, String> {
    session.set(None);
    if let Some(file) = store.read(|s| s.current.clone()) {
        store.update(|s| s.remove(&file)).map_err(|e| format!("Cannot save the change: {e}"))?;
        for suffix in ["", pswm_core::dbfile::BAK, pswm_core::dbfile::REMOTE_BAK] {
            let _ = std::fs::remove_file(pswm_core::dbfile::sibling(&file, suffix));
        }
    }
    Ok(status_of(&store, &session))
}

#[tauri::command]
async fn unlock(app: AppHandle, password: String) -> Result<Listing, String> {
    let password = Zeroizing::new(password);
    off_main(move || {
        let store = app.state::<Store>();
        sync::ensure_working_copy(&store)?;
        let file = store.read(|s| s.current.clone()).ok_or("Choose a database first")?;
        let vault = Vault::open(&file, Some(password.as_str()).filter(|p| !p.is_empty()), None)?;
        let listing = vault.listing();
        let _ = store.update_if(|s| s.current_mut().is_some_and(|k| k.remember(&listing.database.name, &listing.database.description)));
        app.state::<Session>().set(Some(vault));
        start_sync(app.clone());
        Ok(listing)
    })
    .await
}

#[tauri::command]
fn lock(session: State<Session>) {
    session.set(None);
}

#[tauri::command]
fn sync_now(app: AppHandle) {
    start_sync(app);
}

/// Syncs in the background and tells the page (`synced`) what happened.
fn start_sync(app: AppHandle) {
    std::thread::spawn(move || {
        let store = app.state::<Store>();
        let Some(location) = store.read(|s| s.remote().map(|r| r.location.clone())) else { return };
        let result = sync::sync(location.open().as_ref(), &store, &app.state::<Session>());
        let (text, problem) = sync::describe(&store, &result);
        let changed = matches!(&result, Ok(sync::Outcome::Downloaded(c) | sync::Outcome::Merged(c)) if !c.is_empty());
        let _ = app.emit("synced", Synced { text, problem, changed });
    });
}

#[tauri::command]
fn listing(session: State<Session>) -> Result<Listing, String> {
    session.read(Vault::listing)
}

#[tauri::command]
fn entry(session: State<Session>, id: String) -> Result<EntryDetail, String> {
    session.with(|v| v.detail(&id))
}

#[tauri::command]
fn reveal(session: State<Session>, id: String, field: String) -> Result<String, String> {
    session.with(|v| v.field_in(&id, None, &field)).map(|value| value.to_string())
}

/// Copies a field's value; how many seconds it stays on the clipboard.
#[tauri::command]
async fn copy_field(app: AppHandle, id: String, field: String) -> Result<u64, String> {
    let value = app.state::<Session>().with(|v| v.field_in(&id, None, &field))?;
    if value.is_empty() {
        return Err(format!("{field} is empty"));
    }
    copy(app, value).await
}

#[tauri::command]
fn totp(session: State<Session>, id: String) -> Result<Option<otp::Code>, String> {
    session.read(|v| v.totp(&id))?
}

#[tauri::command]
async fn copy_totp(app: AppHandle, id: String) -> Result<u64, String> {
    let code = app.state::<Session>().read(|v| v.totp(&id))??.ok_or("The entry has no TOTP")?;
    copy(app, Zeroizing::new(code.code)).await
}

async fn copy(app: AppHandle, value: Zeroizing<String>) -> Result<u64, String> {
    off_main(move || app.state::<Clipboard<Wry>>().copy(&value, CLEAR_AFTER)).await?;
    Ok(CLEAR_AFTER.as_secs())
}
