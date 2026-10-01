//! Running syncs in the app: when they happen, one at a time, and what the
//! window and the tray show about them. The sync itself is `pswm_core::sync`.

use crate::settings::Settings;
use crate::Session;
use pswm_core::dbfile::OpenError;
use pswm_core::store::Store;
use pswm_core::sync::{sync, Outcome, SyncError};
// The sync itself, for the rest of the app beside the driver here.
pub use pswm_core::sync::{attach, ensure_working_copy, has_pending, is_pending, keep_as_local, link, start, LinkChoice};
use serde::Serialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// Edits go up this long after the last one, so a few edits in a row make one upload.
const UPLOAD_DELAY: Duration = Duration::from_secs(10);
/// Quitting waits this long at most for the last upload.
const QUIT_WAIT: Duration = Duration::from_secs(10);

/// What the window and the tray show about syncing.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// False for a local file: nothing to show.
    pub remote: bool,
    pub busy: bool,
    pub text: String,
    /// The last sync did not finish: offline or an error.
    pub problem: bool,
}

#[derive(Default)]
struct Flags {
    running: bool,
    /// Asked again while running: run once more.
    again: bool,
    /// When edits go up.
    upload_at: Option<Instant>,
    last_run: Option<Instant>,
    status: Status,
}

/// The app's sync state: one sync at a time.
#[derive(Default)]
pub struct Syncer(Mutex<Flags>);

fn flags<R>(app: &AppHandle, f: impl FnOnce(&mut Flags) -> R) -> R {
    f(&mut app.state::<Syncer>().0.lock().unwrap())
}

/// Starts a sync in the background, or one more after the one running.
pub fn request(app: &AppHandle) {
    let synced = app.state::<Store>().read(|s| s.remote().is_some());
    let started = flags(app, |f| {
        f.upload_at = None;
        if !synced {
            return false;
        }
        if f.running {
            f.again = true;
            return false;
        }
        f.running = true;
        true
    });
    if !started {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || while pass(&app, true).1 {});
}

/// One sync, its status shown. Returns its result and whether another was
/// asked for meantime; with `go_on` the caller runs that one next (it keeps
/// the sync marked running), otherwise the sync is over.
fn pass(app: &AppHandle, go_on: bool) -> (Result<Outcome, SyncError>, bool) {
    set_status(app, |s| s.busy = true);
    let result = run_once(app);
    let status = describe(app, &result);
    let again = flags(app, |f| {
        f.last_run = Some(Instant::now());
        f.running = go_on && f.again;
        std::mem::take(&mut f.again)
    });
    let busy = go_on && again;
    set_status(app, |s| *s = Status { busy, ..status });
    // After the last pass of a run: the remote file on a key this device does not know.
    if !busy {
        crate::need_key(app, |k| k.remote = matches!(result, Err(SyncError::OtherKey)));
    }
    (result, again)
}

/// Syncs now, in the calling thread, for a change that needs the remote file
/// in step first (a new key: after it, a remote file changed on another device
/// no longer opens). Nothing to do for a local file; refused while a sync runs.
/// A remote file on a key this device does not know is [OpenError::OtherKey].
pub fn sync_first(app: &AppHandle) -> Result<(), OpenError> {
    if app.state::<Store>().read(|s| s.remote().is_none()) {
        return Ok(());
    }
    if !flags(app, |f| !std::mem::replace(&mut f.running, true)) {
        return Err(String::from("A sync is running; try again in a moment").into());
    }
    let (result, again) = pass(app, false);
    if again {
        request(app);
    }
    match result {
        Ok(_) => Ok(()),
        Err(SyncError::OtherKey) => Err(OpenError::other_key()),
        Err(SyncError::Offline(message) | SyncError::SignIn(message) | SyncError::Failed(message)) => Err(message.into()),
    }
}

fn run_once(app: &AppHandle) -> Result<Outcome, SyncError> {
    let store = app.state::<Store>();
    let Some(location) = store.read(|s| s.remote().map(|r| r.location.clone())) else {
        return Ok(Outcome::UpToDate);
    };
    let outcome = sync(location.open().as_ref(), &store, &app.state::<Session>())?;
    if let Outcome::Downloaded(changed) | Outcome::Merged(changed) = &outcome {
        crate::show_changes(app, changed.clone());
    }
    Ok(outcome)
}

fn describe(app: &AppHandle, result: &Result<Outcome, SyncError>) -> Status {
    let store = app.state::<Store>();
    let name = store.read(|s| s.remote().map(|r| r.location.name())).unwrap_or("the remote store");
    let time = chrono::Local::now().format("%H:%M");
    let (text, problem) = match result {
        Ok(Outcome::Merged(changed)) if !changed.is_empty() => {
            let entries = if changed.len() == 1 { "1 entry".to_string() } else { format!("{} entries", changed.len()) };
            (format!("Merged {entries} from {name} at {time}"), false)
        }
        Ok(Outcome::Merged(_)) => (format!("Merged with {name} at {time}"), false),
        Ok(Outcome::WaitingForUnlock) => (format!("Changes in {name} are merged at the next unlock"), false),
        Ok(_) => (format!("Synced at {time}"), false),
        Err(SyncError::Offline(message)) if has_pending(&store) => (format!("Offline — changes waiting ({message})"), true),
        Err(SyncError::Offline(message)) => (format!("Offline ({message})"), true),
        Err(SyncError::SignIn(message)) => (message.clone(), true),
        Err(SyncError::OtherKey) => (format!("The file in {name} has another master password or key file"), true),
        Err(SyncError::Failed(message)) => (format!("Sync failed: {message}"), true),
    };
    Status { text, problem, ..Status::default() } // `remote` is filled in when it is shown
}

fn set_status(app: &AppHandle, change: impl FnOnce(&mut Status)) {
    let remote = app.state::<Store>().read(|s| s.remote().is_some());
    let status = flags(app, |f| {
        change(&mut f.status);
        f.status.remote = remote;
        f.status.clone()
    });
    if let Some(tray) = app.tray_by_id("main") {
        let tooltip = if status.remote && !status.text.is_empty() { format!("PswManager — {}", status.text) } else { "PswManager".into() };
        let _ = tray.set_tooltip(Some(tooltip));
    }
    let _ = app.emit("sync-status", status);
}

pub fn status(app: &AppHandle) -> Status {
    let remote = app.state::<Store>().read(|s| s.remote().is_some());
    Status { remote, ..flags(app, |f| f.status.clone()) }
}

/// Forgets the last status (after choosing another database, or stopping sync).
pub fn reset(app: &AppHandle) {
    set_status(app, |s| *s = Status::default());
    crate::need_key(app, |k| k.remote = false);
}

/// An entry was created, changed or deleted: it goes up shortly.
pub fn upload_soon(app: &AppHandle) {
    flags(app, |f| f.upload_at = Some(Instant::now() + UPLOAD_DELAY));
}

/// Runs the delayed uploads and the regular check while unlocked.
pub fn start_clock(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        let now = Instant::now();
        let every = Settings::of(&app.state()).sync_every();
        let (upload, check) = flags(&app, |f| {
            let upload = f.upload_at.is_some_and(|at| now >= at);
            let check = !f.running && every.is_some_and(|every| f.last_run.is_none_or(|last| now - last >= every));
            (upload, check)
        });
        // The regular check runs only while unlocked: a locked app cannot merge.
        if upload || (check && app.state::<Session>().is_unlocked()) {
            request(&app);
        }
    });
}

/// Quits once the last changes went up, waiting a little at most. The wait
/// is off the main thread: the sync reports to the tray, which runs there.
pub fn quit_after_upload(app: &AppHandle) {
    request(app);
    let app = app.clone();
    std::thread::spawn(move || {
        let until = Instant::now() + QUIT_WAIT;
        while Instant::now() < until && is_running(&app) {
            std::thread::sleep(Duration::from_millis(100));
        }
        app.exit(0);
    });
}

pub fn is_running(app: &AppHandle) -> bool {
    flags(app, |f| f.running)
}

