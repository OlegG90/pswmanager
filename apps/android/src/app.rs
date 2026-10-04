//! The app's state and commands: the database on this phone (one at a time,
//! `docs/spec-android.md`), unlocking it, and reading and copying its entries.
//! The core does the work, on a working copy in the app's private storage.

use crate::biometric::{Biometric, Failure};
use crate::clipboard::Clipboard;
use crate::documents::{self, Documents};
use crate::cloud;
use pswm_core::documents::DocumentStore;
use pswm_core::opened;
use pswm_core::otp;
use pswm_core::remote::{Cloud, Location};
use pswm_core::session::Session;
use pswm_core::settings::{self, Settings};
use pswm_core::store::Store;
use pswm_core::sync;
use pswm_core::vault::{self, EntryDetail, Listing, Vault, Version, VersionDetail};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Builder, Emitter, Manager, State, Wry};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_opener::OpenerExt;
use zeroize::Zeroizing;

/// The app's state file, in its data folder.
const STATE_FILE: &str = "pswm.json";

pub fn setup(builder: Builder<Wry>) -> Builder<Wry> {
    builder
        .plugin(crate::system::init())
        .plugin(documents::init())
        .plugin(crate::clipboard::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(crate::secrets::init())
        .plugin(crate::biometric::init())
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            app.manage(Store::load(data.join(STATE_FILE)));
            crate::background::remember(app.handle());
            app.manage(Session::default());
            app.manage(LastSync::default());
            app.manage(Syncing::default());
            app.manage(LockLater::default());
            app.manage(crate::editing::UploadSoon::default());
            app.manage(crate::icons::Icons::default());
            // Copies of attachments a crash left behind.
            if let Ok(folder) = open_folder(app.handle()) {
                opened::clean(&folder);
            }
            app.manage(cloud::SignIn::default());
            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    cloud::on_open_url(&handle, url.as_str());
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            status,
            open_local_file,
            forget_database,
            stop_syncing,
            unlock,
            unlock_with_biometric,
            biometric_ready,
            forget_biometric,
            lock,
            listing,
            entry,
            entry_history,
            entry_version,
            reveal,
            copy_field,
            totp,
            copy_totp,
            open_url,
            open_attachment,
            settings,
            set_setting,
            screen_off,
            pick_key_file,
            clear_key_file,
            crate::icons::icon,
            lock_later,
            stay_unlocked,
            sync_now,
            last_sync,
            pick_folder,
            set_copy_folder,
            sync_if_pending,
            cloud::sign_in,
            cloud::cloud_files,
            cloud::open_cloud_file,
            cloud::sync_with_cloud,
            crate::create::create_database,
            crate::visible::copy_name_taken,
            crate::editing::edit_entry,
            crate::editing::save_entry,
            crate::editing::delete_entry,
            crate::editing::set_favorite,
            crate::editing::generate_password,
            crate::editing::password_strength,
            crate::editing::pick_file_to_attach,
            crate::editing::release_files
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
    /// The cloud store it syncs with, if one (and so it may have a visible copy).
    cloud: Option<pswm_core::remote::Cloud>,
    /// The folder its visible copy is in, if one was chosen.
    copy_folder: Option<String>,
    /// The key file it is unlocked with, by name, if it has one.
    key_file: Option<String>,
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
            cloud: k.remote.as_ref().and_then(|r| r.location.cloud()),
            // Read from the same state: not inside this read, which holds it.
            copy_folder: None,
            key_file: None,
        })
    });
    let key_file = key_file(store).map(|k| k.name);
    let database = database.map(|d| Database { copy_folder: crate::visible::folder_name(store), key_file, ..d });
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
            // Every store: one whose syncing was stopped is signed in still.
            for cloud in [Cloud::Dropbox, Cloud::OneDrive, Cloud::Google] {
                cloud.provider().sign_out();
            }
            crate::visible::forget(&store)?;
            forget_key_file(&store)?;
            // It is this database's key.
            let _ = app.state::<Biometric<Wry>>().forget();
        }
        Ok(status_of(&store, &session))
    })
    .await
}

/// Stops syncing with the cloud store (it cannot be reached, the account has
/// a problem, or another store is wanted). The visible copy becomes the
/// database's file, synced like a local file; without one the database stays
/// in the app only. With `sign_out` the store's account is signed out too
/// (*Disconnect*). The remote file is left alone.
#[tauri::command]
async fn stop_syncing(app: AppHandle, sign_out: bool) -> Result<Status, String> {
    off_main(move || {
        while_not_syncing(&app, || {
            let store = app.state::<Store>();
            let cloud = store.read(|s| s.remote().and_then(|r| r.location.cloud())).ok_or("This database does not sync with a cloud store")?;
            crate::visible::stop_syncing(&store)?;
            *app.state::<LastSync>().0.lock().unwrap() = None;
            if sign_out {
                cloud.provider().sign_out();
            }
            Ok(status_of(&store, &app.state::<Session>()))
        })
    })
    .await
}

/// Unlocks with the master password and/or the key file. With biometric
/// unlock on and no sealed key, or the master password due, the key is then
/// sealed for it (the user confirms with a fingerprint or face; declining
/// changes nothing else).
#[tauri::command]
async fn unlock(app: AppHandle, password: String) -> Result<Listing, String> {
    let password = Zeroizing::new(password);
    off_main(move || {
        let store = app.state::<Store>();
        let password = Some(password.as_str()).filter(|p| !p.is_empty());
        let key_file = match key_file(&store) {
            Some(picked) => {
                let content = app.state::<Documents<Wry>>().read(&picked.uri)?;
                Some(Zeroizing::new(content.ok_or(format!("Cannot read the key file {}: it is gone", picked.name))?))
            }
            None => None,
        };
        let listing = open(&app, password, key_file.as_deref().map(Vec::as_slice))?;
        let biometric = app.state::<Biometric<Wry>>();
        if seal_wanted(&store, &biometric) {
            let secret = Secret { password: password.map(str::to_string), key_file: key_file.as_deref().map(|k| B64.encode(k)) };
            let sealed = Zeroizing::new(serde_json::to_string(&secret).map_err(|e| e.to_string())?);
            if biometric.store(&sealed).is_ok() {
                password_asked(&store);
            }
        }
        Ok(listing)
    })
    .await
}

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// The database's key as biometric unlock keeps it (sealed in the Keystore):
/// what the user typed and the key file's content, base64.
#[derive(Serialize, Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
struct Secret {
    password: Option<String>,
    key_file: Option<String>,
}

/// Where the time the master password was last asked for is kept (seconds
/// since 1970): biometric unlock asks for it again after the set days.
const PASSWORD_ASKED: &str = "passwordAsked";

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn password_asked(store: &Store) {
    let _ = store.update(|s| drop(s.settings.insert(PASSWORD_ASKED.into(), now().into())));
}

/// The master password is due again (biometric unlock is not offered).
fn password_due(store: &Store) -> bool {
    let asked = store.read(|s| s.settings.get(PASSWORD_ASKED).and_then(serde_json::Value::as_u64)).unwrap_or(0);
    now().saturating_sub(asked) >= Settings::of(store).password_every().as_secs()
}

/// After an unlock with the master password: the key is sealed for biometric
/// unlock when the setting is on, the phone can, and none is sealed or the
/// password was due.
fn seal_wanted(store: &Store, biometric: &Biometric<Wry>) -> bool {
    Settings::of(store).biometric_unlock() && biometric.status().is_ok_and(|s| s.available && (!s.stored || password_due(store)))
}

/// Opens the database with this key and makes it the session's; the sync starts.
fn open(app: &AppHandle, password: Option<&str>, key_file: Option<&[u8]>) -> Result<Listing, String> {
    let store = app.state::<Store>();
    sync::ensure_working_copy(&store)?;
    let file = store.read(|s| s.current.clone()).ok_or("Choose a database first")?;
    let mut key_file = key_file;
    let key = vault::key_reading(password, key_file.as_mut().map(|k| k as &mut dyn std::io::Read))?;
    let vault = Vault::open_with_key(&file, key)?;
    let listing = vault.listing();
    crate::icons::fetch(app, &listing);
    // Only for the unlock screen next time: not worth failing the unlock over.
    let _ = store.update_if(|s| s.current_mut().is_some_and(|k| k.remember(&listing.database.name, &listing.database.description)));
    app.state::<Session>().set(Some(vault));
    start_sync(app.clone());
    Ok(listing)
}

/// Whether the unlock screen shows the fingerprint button: the setting is
/// on, the phone has a strong biometric, a key is sealed, and the master
/// password is not due.
#[tauri::command]
async fn biometric_ready(app: AppHandle) -> Result<bool, String> {
    off_main(move || {
        let store = app.state::<Store>();
        let status = app.state::<Biometric<Wry>>().status()?;
        Ok(Settings::of(&store).biometric_unlock() && status.available && status.stored && !password_due(&store))
    })
    .await
}

/// The answer when the user chose the master password at the prompt.
const CANCELLED: &str = "cancelled";

/// Unlocks with the key sealed for biometric unlock. A key that no longer
/// opens the database (changed on another device) is deleted: the master
/// password is asked for, and the new key sealed.
#[tauri::command]
async fn unlock_with_biometric(app: AppHandle) -> Result<Listing, String> {
    off_main(move || {
        let store = app.state::<Store>();
        if !Settings::of(&store).biometric_unlock() || password_due(&store) {
            return Err("Unlock with the master password".into());
        }
        let biometric = app.state::<Biometric<Wry>>();
        let sealed = biometric.retrieve().map_err(|failure| match failure {
            Failure::Cancelled => CANCELLED.to_string(),
            Failure::Invalidated => "A new fingerprint or face was added: unlock with the master password once to use it again".into(),
            Failure::NotStored => "Unlock with the master password".into(),
            Failure::Other(message) => message,
        })?;
        let secret: Secret = serde_json::from_str(&sealed).map_err(|_| "The stored key cannot be read: unlock with the master password")?;
        let key_file = secret.key_file.as_deref().map(|k| B64.decode(k).map(Zeroizing::new)).transpose().map_err(|e| e.to_string())?;
        open(&app, secret.password.as_deref(), key_file.as_deref().map(Vec::as_slice)).map_err(|e| {
            if e == pswm_core::dbfile::WRONG_KEY {
                let _ = biometric.forget();
                "The database's key has changed: unlock with the new master password".to_string()
            } else {
                e
            }
        })
    })
    .await
}

/// Biometric unlock was turned off: the sealed key goes.
#[tauri::command]
async fn forget_biometric(app: AppHandle) -> Result<(), String> {
    off_main(move || app.state::<Biometric<Wry>>().forget()).await
}

/// Locks: the key and every decrypted value go, and so do the copies of
/// opened attachments.
#[tauri::command]
fn lock(app: AppHandle) {
    lock_now(&app);
}

fn lock_now(app: &AppHandle) {
    app.state::<Session>().set(None);
    // As on Windows: what has not gone up yet goes now; a merge waits for the next unlock.
    upload_pending(app.clone());
    if let Ok(folder) = open_folder(app) {
        opened::clean(&folder);
    }
}

/// Waits where only the latest counts: each new one makes the ones before void.
#[derive(Default)]
pub struct Latest(std::sync::atomic::AtomicU64);

impl Latest {
    /// Voids the waits before; the new one's number.
    pub fn next(&self) -> u64 {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
    }

    pub fn is_latest(&self, wait: u64) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst) == wait
    }
}

/// A lock waiting to happen (the app went to the background); `stay_unlocked`
/// voids it too.
#[derive(Default)]
struct LockLater(Latest);

/// Locks after the setting's time in the background unless the app comes
/// back first (`stay_unlocked`); tells the page (`locked`). The page also
/// checks the time itself when it comes back, in case Android kept this from
/// running on time.
#[tauri::command]
fn lock_later(app: AppHandle) {
    let wait = app.state::<LockLater>().0.next();
    let Some(after) = Settings::of(&app.state()).lock_in_background() else { return };
    std::thread::spawn(move || {
        std::thread::sleep(after);
        if app.state::<LockLater>().0.is_latest(wait) && app.state::<Session>().is_unlocked() {
            lock_now(&app);
            let _ = app.emit("locked", ());
        }
    });
}

#[tauri::command]
fn stay_unlocked(later: State<LockLater>) {
    later.0.next();
}

/// The screen turned off: locks when the setting says so; whether it did.
#[tauri::command]
fn screen_off(app: AppHandle) -> bool {
    let lock = Settings::of(&app.state()).lock_on_screen_off() && app.state::<Session>().is_unlocked();
    if lock {
        lock_now(&app);
    }
    lock
}

/// The settings screen's values, and what it says about the app.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsView {
    #[serde(flatten)]
    settings: settings::View,
    version: &'static str,
}

#[tauri::command]
fn settings(store: State<Store>) -> SettingsView {
    SettingsView { settings: Settings::of(&store).view(), version: env!("CARGO_PKG_VERSION") }
}

#[tauri::command]
fn set_setting(store: State<Store>, name: String, value: serde_json::Value) -> Result<SettingsView, String> {
    Settings::of(&store).set(&name, value)?;
    Ok(settings(store))
}

/// Where the key file is kept in the state file (the phone has one database):
/// the document the user picked, read at each unlock.
const KEY_FILE: &str = "keyFile";

fn key_file(store: &Store) -> Option<documents::Picked> {
    store.read(|s| s.settings.get(KEY_FILE).cloned()).and_then(|v| serde_json::from_value(v).ok())
}

/// Picks the key file the database is unlocked with (its access is kept).
#[tauri::command]
async fn pick_key_file(app: AppHandle) -> Result<Status, String> {
    off_main(move || {
        let store = app.state::<Store>();
        if let Some(picked) = app.state::<Documents<Wry>>().pick_file()? {
            let value = serde_json::to_value(&picked).map_err(|e| e.to_string())?;
            store.update(|s| drop(s.settings.insert(KEY_FILE.into(), value))).map_err(|e| format!("Cannot save the key file: {e}"))?;
        }
        Ok(status_of(&store, &app.state::<Session>()))
    })
    .await
}

/// The database is unlocked without a key file from now on.
#[tauri::command]
fn clear_key_file(store: State<Store>, session: State<Session>) -> Result<Status, String> {
    forget_key_file(&store)?;
    Ok(status_of(&store, &session))
}

fn forget_key_file(store: &Store) -> Result<(), String> {
    store.update(|s| drop(s.settings.remove(KEY_FILE))).map_err(|e| format!("Cannot save the change: {e}"))
}

/// Where copies of opened attachments go: the app's cache, which the
/// FileProvider shares (`res/xml/file_paths.xml`).
fn open_folder(app: &AppHandle) -> Result<PathBuf, String> {
    let cache = app.path().app_cache_dir().map_err(|e| format!("Cannot find the app's cache: {e}"))?;
    Ok(cache.join("open"))
}

/// Opens an attachment (of an older version with `version`) in another app,
/// through a read-only copy that locking deletes. Packages and scripts are
/// not opened: Android would offer to install or run them.
#[tauri::command]
async fn open_attachment(app: AppHandle, id: String, name: String, version: Option<usize>) -> Result<(), String> {
    if opened::is_runnable_on_android(&name) {
        return Err("Apps and scripts are not opened from the database".into());
    }
    let data = app.state::<Session>().with(|v| v.attachment(&id, version, &name))?;
    let path = opened::write(&open_folder(&app)?, &name, &data).map_err(|e| format!("Cannot open the file: {e}"))?;
    off_main(move || app.state::<Documents<Wry>>().open_file(&path)).await
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

    fn is_running(&self) -> bool {
        self.0.lock().unwrap().running
    }

    /// Ends a run that will not go again; true when one was asked for meanwhile.
    fn finish(&self) -> bool {
        let mut flags = self.0.lock().unwrap();
        flags.running = false;
        std::mem::take(&mut flags.again)
    }

    /// True when it was asked again meanwhile: run once more.
    fn again(&self) -> bool {
        let mut flags = self.0.lock().unwrap();
        flags.running = std::mem::take(&mut flags.again);
        flags.running
    }
}

/// Runs `change` to where the database syncs while no sync runs (refused
/// while one does); a sync asked for meanwhile runs after it, with the change.
pub fn while_not_syncing<T>(app: &AppHandle, change: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    if !app.state::<Syncing>().begin() {
        return Err("A sync is running; try again in a moment".into());
    }
    // A change that panics must not leave the syncs stopped.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(change)).unwrap_or_else(|_| Err("Something went wrong".into()));
    if app.state::<Syncing>().finish() {
        start_sync(app.clone());
    }
    result
}

/// Syncs in the background and tells the page (`synced`) what happened.
pub fn start_sync(app: AppHandle) {
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
/// locked one (uploads only; a merge waits for the next unlock). Whether it
/// is settled ([sync::settled]).
fn sync_once(app: &AppHandle, session: &Session) -> bool {
    let store = app.state::<Store>();
    let Some(location) = store.read(|s| s.remote().map(|r| r.location.clone())) else { return true };
    let result = sync::sync(location.open().as_ref(), &store, session);
    let settled = sync::settled(&result);
    let (text, problem) = sync::describe(&store, &result);
    let changed = matches!(&result, Ok(sync::Outcome::Downloaded(c) | sync::Outcome::Merged(c)) if !c.is_empty());
    let sign_in = matches!(result, Err(sync::SyncError::SignIn(_)));
    let copy_problem = crate::visible::refresh(app);
    let problem = problem || copy_problem.is_some();
    let synced = Synced { text, problem, changed, sign_in, copy_problem };
    *app.state::<LastSync>().0.lock().unwrap() = Some(synced.clone());
    // A page that is not listening reads it with `last_sync`.
    let _ = app.emit("synced", synced);
    settled
}

/// Going to the background: what has not gone up yet goes now, as an upload
/// only (as when locking; a merge needs the key and waits for the front).
#[tauri::command]
fn sync_if_pending(app: AppHandle) {
    upload_pending(app);
}

fn upload_pending(app: AppHandle) {
    let store = app.state::<Store>();
    if !sync::has_pending(&store) {
        return;
    }
    // In case Android stops the app before this upload is through.
    schedule_background_upload(&app, &store);
    if !app.state::<Syncing>().begin() {
        return;
    }
    tauri::async_runtime::spawn_blocking(move || loop {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sync_once(&app, &Session::default())));
        if !app.state::<Syncing>().again() {
            return;
        }
    });
}

/// Asks WorkManager to upload the waiting changes later (`background.rs`):
/// when a change is saved, as the app may go at any moment (Back ends it at
/// once), and again on going to the background or locking.
pub fn schedule_background_upload(app: &AppHandle, store: &Store) {
    let cloud = store.read(|s| s.remote().is_some_and(|r| r.location.cloud().is_some()));
    let Ok(data) = app.path().app_data_dir() else { return };
    // Only a fallback: the upload that follows may well send the changes.
    let _ = app.state::<crate::system::System<Wry>>().schedule_upload(&data.join(STATE_FILE).to_string_lossy(), cloud);
}

/// The background upload with the app running in this process: what has not
/// gone up yet goes now, as an upload only, after a sync that runs (waiting
/// two minutes at most). Whether it is settled ([sync::settled]).
pub fn upload_pending_now(app: &AppHandle) -> bool {
    let syncing = app.state::<Syncing>();
    for _ in 0..120 {
        if !syncing.is_running() && syncing.begin() {
            let mut settled;
            loop {
                settled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sync_once(app, &Session::default()))).unwrap_or(false);
                if !syncing.again() {
                    return settled;
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    false
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
fn listing(app: AppHandle, session: State<Session>) -> Result<Listing, String> {
    let listing = session.read(Vault::listing)?;
    crate::icons::fetch(&app, &listing);
    Ok(listing)
}

#[tauri::command]
fn entry(session: State<Session>, id: String) -> Result<EntryDetail, String> {
    session.with(|v| v.detail(&id))
}

/// The entry's older versions, newest first: when, and what changed; never values.
#[tauri::command]
fn entry_history(session: State<Session>, id: String) -> Result<Vec<Version>, String> {
    session.with(|v| v.history(&id))
}

/// One older version, read only, secrets masked, with how it differs from the entry now.
#[tauri::command]
fn entry_version(session: State<Session>, id: String, index: usize) -> Result<VersionDetail, String> {
    session.with(|v| v.version(&id, index))
}

/// One field's value (of an older version with `version`), for showing it.
#[tauri::command]
fn reveal(session: State<Session>, id: String, field: String, version: Option<usize>) -> Result<String, String> {
    session.with(|v| v.field_in(&id, version, &field)).map(|value| value.to_string())
}

/// Copies a field's value (of an older version with `version`); how many
/// seconds it stays on the clipboard.
#[tauri::command]
async fn copy_field(app: AppHandle, id: String, field: String, version: Option<usize>) -> Result<u64, String> {
    let value = app.state::<Session>().with(|v| v.field_in(&id, version, &field))?;
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
    let clear_after = Settings::of(&app.state()).clear_clipboard_after();
    off_main(move || app.state::<Clipboard<Wry>>().copy(&value, clear_after)).await?;
    Ok(clear_after.as_secs())
}
