//! Running syncs in the app: the driver's rules (`sync_driver`) on the app's
//! state, the clock that ticks it, and what the window and the tray show.
//! The sync itself is `pswm_core::sync`.

use crate::settings::Settings;
use crate::sync_driver::{Driver, Host};
use crate::Session;
use pswm_core::dbfile::OpenError;
use pswm_core::store::Store;
use pswm_core::sync::{sync, Outcome, SyncError};
// The sync itself, for the rest of the app beside the driver here.
pub use pswm_core::sync::{attach, ensure_working_copy, has_pending, is_pending, keep_as_local, link, start, LinkChoice};
pub use crate::sync_driver::Status;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// Quitting waits this long at most for the last upload.
const QUIT_WAIT: Duration = Duration::from_secs(10);

/// The app, as the driver sees it.
struct App<'a>(&'a AppHandle);

impl Host for App<'_> {
    fn synced(&self) -> bool {
        self.0.state::<Store>().read(|s| s.remote().is_some())
    }

    fn unlocked(&self) -> bool {
        self.0.state::<Session>().is_unlocked()
    }

    fn check_every(&self) -> Option<Duration> {
        Settings::of(&self.0.state()).sync_every()
    }

    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sync(&self) -> Result<Outcome, SyncError> {
        let store = self.0.state::<Store>();
        let Some(location) = store.read(|s| s.remote().map(|r| r.location.clone())) else {
            return Ok(Outcome::UpToDate);
        };
        let outcome = sync(location.open().as_ref(), &store, &self.0.state::<Session>())?;
        if let Outcome::Downloaded(changed) | Outcome::Merged(changed) = &outcome {
            crate::databases::show_changes(self.0, changed.clone());
        }
        Ok(outcome)
    }

    fn describe(&self, result: &Result<Outcome, SyncError>) -> (String, bool) {
        pswm_core::sync::describe(&self.0.state::<Store>(), result)
    }

    fn show(&self, status: &Status) {
        if let Some(tray) = self.0.tray_by_id("main") {
            let tooltip = if status.remote && !status.text.is_empty() { format!("PswManager — {}", status.text) } else { "PswManager".into() };
            let _ = tray.set_tooltip(Some(tooltip));
        }
        let _ = self.0.emit("sync-status", status);
    }

    fn remote_key_needed(&self, needed: bool) {
        crate::need_key(self.0, |k| k.remote = needed);
    }

    fn run_in_background(&self) {
        let app = self.0.clone();
        std::thread::spawn(move || driver(&app).run(&App(&app)));
    }
}

fn driver(app: &AppHandle) -> tauri::State<'_, Driver> {
    app.state::<Driver>()
}

/// Starts a sync in the background, or one more after the one running.
pub fn request(app: &AppHandle) {
    driver(app).request(&App(app));
}

/// Syncs now, in the calling thread, for a change that needs the remote file
/// in step first; see [Driver::sync_first].
pub fn sync_first(app: &AppHandle) -> Result<(), OpenError> {
    driver(app).sync_first(&App(app))
}

pub fn status(app: &AppHandle) -> Status {
    driver(app).status(&App(app))
}

/// Forgets the last status (after choosing another database, or stopping sync).
pub fn reset(app: &AppHandle) {
    driver(app).reset(&App(app));
}

/// An entry was created, changed or deleted: it goes up shortly.
pub fn upload_soon(app: &AppHandle) {
    driver(app).upload_soon(&App(app));
}

/// Runs the delayed uploads and the regular check while unlocked.
pub fn start_clock(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        driver(&app).tick(&App(&app));
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
    driver(app).is_running()
}
