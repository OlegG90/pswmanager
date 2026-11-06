//! The database's syncs (`docs/spec-android.md`, *Synchronisation*): Sync now,
//! the sync after an unlock, the upload soon after a change, the upload on
//! going to the background or locking (and WorkManager's later, when Android
//! stops the app first), the visible copy written after each, and moving
//! where the database syncs. The core's [Syncs] runs them one at a time and
//! keeps the last one's report; this is where the app hands it the sync to do.

use crate::app::{off_main, status_of, Latest, Status};
use crate::documents;
use pswm_core::session::Session;
use pswm_core::store::Store;
use pswm_core::sync;
use pswm_core::syncs::{Report, Syncs};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State, Wry};

pub use pswm_core::syncs::OTHER_KEY;

/// The app's state for its syncs.
pub fn manage(app: &AppHandle) {
    app.manage(Syncs::default());
    app.manage(UploadSoon::default());
}

#[tauri::command]
pub fn sync_now(app: AppHandle) {
    start(app);
}

/// What the last sync did, kept for a page that missed the event.
#[tauri::command]
pub fn last_sync(syncs: State<Syncs>) -> Option<Report> {
    syncs.last()
}

/// Runs `change` to where the database syncs while no sync runs (refused
/// while one does); a sync asked for meanwhile runs after it, with the change.
pub fn while_not_syncing<T>(app: &AppHandle, change: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    app.state::<Syncs>().while_not_syncing(change, || start(app.clone()))
}

/// Syncs now, waiting for it: before a change the remote file must be in step
/// with (see [Syncs::first]). Nothing to do for a database that syncs with nothing.
pub(crate) fn sync_first(app: &AppHandle) -> Result<(), String> {
    if app.state::<Store>().read(|s| s.remote().is_none()) {
        return Ok(());
    }
    app.state::<Syncs>().first(|| sync_once(app, &app.state::<Session>()), || start(app.clone()))
}

/// Syncs in the background and tells the page (`synced`) what happened.
pub fn start(app: AppHandle) {
    run(app, true);
}

/// Runs the syncs in the background unless one runs (it goes again then):
/// with the session, or (`unlocked` false) as uploads only.
fn run(app: AppHandle, unlocked: bool) {
    if !app.state::<Syncs>().begin() {
        return;
    }
    tauri::async_runtime::spawn_blocking(move || {
        let locked = Session::default();
        app.state::<Syncs>().drain(|| sync_once(&app, if unlocked { app.state::<Session>().inner() } else { &locked }))
    });
}

/// One sync with `session`: the unlocked database (merges if need be), or a
/// locked one (uploads only; a merge waits for the next unlock). Whether it
/// is settled ([sync::settled]).
fn sync_once(app: &AppHandle, session: &Session) -> bool {
    let store = app.state::<Store>();
    let Some(location) = store.read(|s| s.remote().map(|r| r.location.clone())) else { return true };
    let result = sync::sync(location.open().as_ref(), &store, session);
    let report = Report::of(&store, &result).with_copy_problem(crate::visible::refresh(app));
    // A page that is not listening reads it with `last_sync`.
    let _ = app.emit("synced", app.state::<Syncs>().keep(report));
    sync::settled(&result)
}

/// Going to the background: what has not gone up yet goes now, as an upload
/// only (as when locking; a merge needs the key and waits for the front).
#[tauri::command]
pub fn sync_if_pending(app: AppHandle) {
    upload_pending(app);
}

pub(crate) fn upload_pending(app: AppHandle) {
    let store = app.state::<Store>();
    if !sync::has_pending(&store) {
        return;
    }
    // In case Android stops the app before this upload is through.
    schedule_background_upload(&app, &store);
    run(app, false);
}

/// Asks WorkManager to upload the waiting changes later (`background.rs`):
/// when a change is saved, as the app may go at any moment (Back ends it at
/// once), and again on going to the background or locking.
pub fn schedule_background_upload(app: &AppHandle, store: &Store) {
    let cloud = store.read(|s| s.remote().is_some_and(|r| r.location.cloud().is_some()));
    let Ok(data) = app.path().app_data_dir() else { return };
    // Only a fallback: the upload that follows may well send the changes.
    let _ = app.state::<crate::system::System<Wry>>().schedule_upload(&data.join(crate::app::STATE_FILE).to_string_lossy(), cloud);
}

/// The background upload with the app running in this process: what has not
/// gone up yet goes now, as an upload only, after a sync that runs (waiting
/// two minutes at most). Whether it is settled ([sync::settled]).
pub fn upload_pending_now(app: &AppHandle) -> bool {
    app.state::<Syncs>().after_running(120, Duration::from_secs(1), || sync_once(app, &Session::default()))
}

/// How long after the last change it goes up.
const UPLOAD_DELAY: Duration = Duration::from_secs(10);

/// The upload waiting for the last change.
#[derive(Default)]
struct UploadSoon(Latest);

/// A change was saved: should the app go before it is sent, WorkManager
/// sends it later.
pub(crate) fn upload_later(app: &AppHandle) {
    schedule_background_upload(app, &app.state::<Store>());
}

/// Syncs `UPLOAD_DELAY` after the last change. Going to the background first
/// sends it at once (`sync_if_pending`).
pub(crate) fn upload_soon(app: &AppHandle) {
    upload_later(app);
    let wait = app.state::<UploadSoon>().0.next();
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(UPLOAD_DELAY);
        if app.state::<UploadSoon>().0.is_latest(wait) {
            start(app);
        }
    });
}

/// Stops syncing with the cloud store (it cannot be reached, the account has
/// a problem, or another store is wanted). The visible copy becomes the
/// database's file, synced like a local file; without one the database stays
/// in the app only. The remote file is left alone, and the store's account
/// stays signed in (#225).
#[tauri::command]
pub async fn stop_syncing(app: AppHandle) -> Result<Status, String> {
    off_main(move || {
        while_not_syncing(&app, || {
            let store = app.state::<Store>();
            if !store.read(|s| s.remote().is_some_and(|r| r.location.cloud().is_some())) {
                return Err("This database does not sync with a cloud store".into());
            }
            crate::visible::stop_syncing(&store)?;
            app.state::<Syncs>().forget();
            Ok(status_of(&store, &app.state::<Session>()))
        })
    })
    .await
}

/// Puts the synced database's visible copy into `folder` (in place of where it was).
#[tauri::command]
pub async fn set_copy_folder(app: AppHandle, folder: documents::Picked) -> Result<Status, String> {
    off_main(move || {
        let store = app.state::<Store>();
        let location = store.read(|s| s.remote().map(|r| r.location.clone())).filter(|l| l.cloud().is_some());
        let name = location.ok_or("Only a database synced with a cloud store has a copy on this phone")?.file_name();
        crate::visible::place(&app, &folder.uri, &folder.name, &name)?;
        Ok(status_of(&store, &app.state::<Session>()))
    })
    .await
}
