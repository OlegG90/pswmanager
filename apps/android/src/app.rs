//! The app's state and commands: the database on this phone (one at a time,
//! `docs/spec-android.md`), unlocking it, and reading and copying its entries.
//! The core does the work, on a working copy in the app's private storage.

use crate::clipboard::Clipboard;
use crate::documents::{self, Documents};
use crate::dropbox;
use pswm_core::otp;
use pswm_core::remote::Location;
use pswm_core::session::Session;
use pswm_core::store::Store;
use pswm_core::sync;
use pswm_core::vault::{EntryDetail, Listing, Vault};
use serde::Serialize;
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Builder, Emitter, Manager, State, Wry};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_opener::OpenerExt;
use zeroize::Zeroizing;

/// How long a copied value stays on the clipboard (a setting later).
const CLEAR_AFTER: Duration = Duration::from_secs(20);

pub fn setup(builder: Builder<Wry>) -> Builder<Wry> {
    builder
        .plugin(crate::back::init())
        .plugin(documents::init())
        .plugin(crate::clipboard::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(crate::secrets::init())
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            app.manage(Store::load(data.join("pswm.json")));
            app.manage(Session::default());
            app.manage(LastSync::default());
            app.manage(Syncing::default());
            app.manage(dropbox::SignIn::default());
            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    dropbox::on_open_url(&handle, url.as_str());
                }
            });
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
            open_url,
            sync_now,
            last_sync,
            pick_folder,
            set_copy_folder,
            sync_if_pending,
            dropbox::sign_in_to_dropbox,
            dropbox::dropbox_files,
            dropbox::open_dropbox_file,
            crate::visible::copy_name_taken
        ])
}

/// What the unlock screen shows about the database, if there is one.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
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
    /// It syncs with a cloud store (and so may have a visible copy).
    cloud: bool,
    /// The folder its visible copy is in, if one was chosen.
    copy_folder: Option<String>,
}

/// What the last sync did, for the status line (event `synced`).
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Synced {
    text: String,
    problem: bool,
    /// The entries changed: the list is read again.
    changed: bool,
    /// The store wants the user to sign in again.
    sign_in: bool,
    /// The visible copy could not be written, and why.
    copy_problem: Option<String>,
}

/// Work that waits: the plugins wait for Android's main thread (so they are
/// never called from it), and syncs and unlocks take a while.
pub async fn off_main<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work).await.map_err(|e| e.to_string())?
}

pub fn status_of(store: &Store, session: &Session) -> Status {
    let database = store.read(|s| {
        s.current().map(|k| Database {
            title: k.title(),
            description: k.description.clone().unwrap_or_default(),
            synced_with: k.remote.as_ref().map(|r| r.location.describe()),
            cloud: k.remote.as_ref().is_some_and(|r| r.location.cloud().is_some()),
            // Read from the same state: not inside this read, which holds it.
            copy_folder: None,
        })
    });
    let database = database.map(|d| Database { copy_folder: crate::visible::folder_name(store), ..d });
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
        adopt(&app, Location::Document { uri: picked.uri, name: picked.name }).map(Some)
    })
    .await
}

/// Makes the remote file at `location` the database on this phone: its working
/// copy is downloaded into the app's storage, under the file's name.
pub fn adopt(app: &AppHandle, location: Location) -> Result<Status, String> {
    let store = app.state::<Store>();
    let taken: Vec<PathBuf> = store.read(|s| s.databases.iter().map(|k| k.file.clone()).collect());
    let local = sync::free_path(&store.dir().join("databases"), &location.file_name(), &taken);
    sync::start(&store, location, local)?;
    Ok(status_of(&store, &app.state::<Session>()))
}

/// Forgets the database (locking it first); its working copy goes too, the
/// file it synced with stays where it is, and the store's account is signed
/// out (the phone has one database). Refused while the working copy has
/// changes the file lacks: they would be lost.
#[tauri::command]
async fn forget_database(app: AppHandle) -> Result<Status, String> {
    off_main(move || {
        let (store, session) = (app.state::<Store>(), app.state::<Session>());
        if sync::has_pending(&store) {
            return Err("This database has changes its file does not have yet: unlock it to sync them first".into());
        }
        session.set(None);
        if let Some(current) = store.read(|s| s.current().cloned()) {
            store.update(|s| s.remove(&current.file)).map_err(|e| format!("Cannot save the change: {e}"))?;
            for suffix in ["", pswm_core::dbfile::BAK, pswm_core::dbfile::REMOTE_BAK] {
                let _ = std::fs::remove_file(pswm_core::dbfile::sibling(&current.file, suffix));
            }
            if let Some(cloud) = current.remote.and_then(|r| r.location.cloud()) {
                cloud.provider().sign_out();
            }
            crate::visible::forget(&store)?;
        }
        Ok(status_of(&store, &session))
    })
    .await
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
        // Only for the unlock screen next time: not worth failing the unlock over.
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

/// What the last sync did, kept for a page that missed the event.
#[derive(Default)]
struct LastSync(std::sync::Mutex<Option<Synced>>);

#[tauri::command]
fn last_sync(last: State<LastSync>) -> Option<Synced> {
    last.0.lock().unwrap().clone()
}

/// One sync at a time: asked again while one runs, it runs once more after it.
#[derive(Default)]
struct Syncing(std::sync::Mutex<Flags>);

#[derive(Default)]
struct Flags {
    running: bool,
    again: bool,
}

impl Syncing {
    /// True when a sync may start now; otherwise the one running goes again.
    fn begin(&self) -> bool {
        let mut flags = self.0.lock().unwrap();
        if flags.running {
            flags.again = true;
            return false;
        }
        flags.running = true;
        true
    }

    /// True when it was asked again meanwhile: run once more.
    fn again(&self) -> bool {
        let mut flags = self.0.lock().unwrap();
        flags.running = std::mem::take(&mut flags.again);
        flags.running
    }
}

/// Syncs in the background and tells the page (`synced`) what happened.
fn start_sync(app: AppHandle) {
    if !app.state::<Syncing>().begin() {
        return;
    }
    tauri::async_runtime::spawn_blocking(move || loop {
        // A sync that panics must not stop the ones after it.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sync_once(&app, &app.state::<Session>())));
        if !app.state::<Syncing>().again() {
            return;
        }
    });
}

/// One sync with `session`: the unlocked database (merges if need be), or a
/// locked one (uploads only; a merge waits for the next unlock).
fn sync_once(app: &AppHandle, session: &Session) {
    let store = app.state::<Store>();
    let Some(location) = store.read(|s| s.remote().map(|r| r.location.clone())) else { return };
    let result = sync::sync(location.open().as_ref(), &store, session);
    let (text, problem) = sync::describe(&store, &result);
    let changed = matches!(&result, Ok(sync::Outcome::Downloaded(c) | sync::Outcome::Merged(c)) if !c.is_empty());
    let sign_in = matches!(result, Err(sync::SyncError::SignIn(_)));
    let copy_problem = crate::visible::refresh(app);
    let problem = problem || copy_problem.is_some();
    let synced = Synced { text, problem, changed, sign_in, copy_problem };
    *app.state::<LastSync>().0.lock().unwrap() = Some(synced.clone());
    // A page that is not listening reads it with `last_sync`.
    let _ = app.emit("synced", synced);
}

/// Going to the background: what has not gone up yet goes now, as an upload
/// only (as when locking; a merge needs the key and waits for the front).
#[tauri::command]
fn sync_if_pending(app: AppHandle) {
    if !sync::has_pending(&app.state::<Store>()) || !app.state::<Syncing>().begin() {
        return;
    }
    tauri::async_runtime::spawn_blocking(move || loop {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sync_once(&app, &Session::default())));
        if !app.state::<Syncing>().again() {
            return;
        }
    });
}

/// Puts the synced database's visible copy into `folder` (in place of where it was).
#[tauri::command]
async fn set_copy_folder(app: AppHandle, folder: documents::Picked) -> Result<Status, String> {
    off_main(move || {
        let store = app.state::<Store>();
        let location = store.read(|s| s.remote().map(|r| r.location.clone())).filter(|l| l.cloud().is_some());
        let name = location.ok_or("Only a database synced with a cloud store has a copy on this phone")?.file_name();
        crate::visible::place(&app, &folder.uri, &folder.name, &name)?;
        Ok(status_of(&store, &app.state::<Session>()))
    })
    .await
}

/// A folder the user picks (for the visible copy); `None` when they cancelled.
#[tauri::command]
async fn pick_folder(app: AppHandle) -> Result<Option<documents::Picked>, String> {
    off_main(move || app.state::<Documents<Wry>>().pick_folder()).await
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
    copy_to_clipboard(app, value).await
}

#[tauri::command]
fn totp(session: State<Session>, id: String) -> Result<Option<otp::Code>, String> {
    session.read(|v| v.totp(&id))?
}

#[tauri::command]
async fn copy_totp(app: AppHandle, id: String) -> Result<u64, String> {
    let code = app.state::<Session>().read(|v| v.totp(&id))??.ok_or("The entry has no TOTP")?;
    copy_to_clipboard(app, Zeroizing::new(code.code)).await
}

/// Opens the entry's URL in the default browser.
#[tauri::command]
fn open_url(app: AppHandle, session: State<Session>, id: String) -> Result<(), String> {
    let url = session.with(|v| Some(v.web_url(&id)))?.ok_or("The entry has no web address")?;
    app.opener().open_url(url.as_str(), None::<&str>).map_err(|e| e.to_string())
}

async fn copy_to_clipboard(app: AppHandle, value: Zeroizing<String>) -> Result<u64, String> {
    off_main(move || app.state::<Clipboard<Wry>>().copy(&value, CLEAR_AFTER)).await?;
    Ok(CLEAR_AFTER.as_secs())
}
