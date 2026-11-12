//! When the open database syncs: one sync at a time, one more after the one
//! running when it was asked for meanwhile, edits sent a little after the
//! last one, a regular check while unlocked, and the status the window and
//! the tray show. The driver owns those rules; what it needs of the app (the
//! sync itself, showing the status, the time) is the [Host]'s, so tests drive
//! it without Tauri. The app's side is `sync`.

use pswm_core::dbfile::OpenError;
use pswm_core::sync::{Outcome, SyncError};
use serde::Serialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Edits go up this long after the last one, so a few edits in a row make one upload.
const UPLOAD_DELAY: Duration = Duration::from_secs(10);

/// What the window and the tray show about syncing.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// False for a local file: nothing to show.
    pub remote: bool,
    pub busy: bool,
    pub text: String,
    /// The last sync did not finish: offline or an error.
    pub problem: bool,
}

/// What the driver needs of the app.
pub trait Host {
    /// Whether the current database syncs with a remote file.
    fn synced(&self) -> bool;
    /// Whether a database is unlocked: a locked app cannot merge.
    fn unlocked(&self) -> bool;
    /// How often the remote file is checked while unlocked, if at all.
    fn check_every(&self) -> Option<Duration>;
    fn now(&self) -> Instant;
    /// One sync of the current database; the window is told what it changed.
    fn sync(&self) -> Result<Outcome, SyncError>;
    /// What the status says of a sync's `result`, and whether it is a problem.
    fn describe(&self, result: &Result<Outcome, SyncError>) -> (String, bool);
    /// Shows `status` in the window and the tray.
    fn show(&self, status: &Status);
    /// Whether the remote file is on a key this device does not know.
    fn remote_key_needed(&self, needed: bool);
    /// Runs [Driver::run] in the background.
    fn run_in_background(&self);
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
pub struct Driver(Mutex<Flags>);

impl Driver {
    fn flags<R>(&self, f: impl FnOnce(&mut Flags) -> R) -> R {
        f(&mut self.0.lock().unwrap())
    }

    /// Starts a sync in the background, or one more after the one running.
    pub fn request(&self, host: &impl Host) {
        let synced = host.synced();
        let started = self.flags(|f| {
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
        if started {
            host.run_in_background();
        }
    }

    /// The sync [Driver::request] started, and again while one more was asked
    /// for meantime.
    pub fn run(&self, host: &impl Host) {
        while self.pass(host, true).1 {}
    }

    /// One sync, its status shown. Returns its result and whether another was
    /// asked for meantime; with `go_on` the caller runs that one next (it keeps
    /// the sync marked running), otherwise the sync is over.
    fn pass(&self, host: &impl Host, go_on: bool) -> (Result<Outcome, SyncError>, bool) {
        self.set_status(host, |s| s.busy = true);
        let result = host.sync();
        let (text, problem) = host.describe(&result);
        let now = host.now();
        let again = self.flags(|f| {
            f.last_run = Some(now);
            f.running = go_on && f.again;
            std::mem::take(&mut f.again)
        });
        let busy = go_on && again;
        self.set_status(host, |s| *s = Status { busy, text, problem, ..Status::default() }); // `remote` is filled in when it is shown
        // After the last pass of a run: the remote file on a key this device does not know.
        if !busy {
            host.remote_key_needed(matches!(result, Err(SyncError::OtherKey)));
        }
        (result, again)
    }

    /// Syncs now, in the calling thread, for a change that needs the remote file
    /// in step first (a new key: after it, a remote file changed on another device
    /// no longer opens). Nothing to do for a local file; refused while a sync runs.
    /// A remote file on a key this device does not know is [OpenError::OtherKey].
    pub fn sync_first(&self, host: &impl Host) -> Result<(), OpenError> {
        if !host.synced() {
            return Ok(());
        }
        if !self.flags(|f| !std::mem::replace(&mut f.running, true)) {
            return Err("A sync is running; try again in a moment".into());
        }
        let (result, again) = self.pass(host, false);
        if again {
            self.request(host);
        }
        match result {
            Ok(_) => Ok(()),
            Err(SyncError::OtherKey) => Err(OpenError::other_key()),
            Err(SyncError::Offline(message) | SyncError::SignIn(message) | SyncError::Failed(message)) => Err(message.into()),
        }
    }

    fn set_status(&self, host: &impl Host, change: impl FnOnce(&mut Status)) {
        let remote = host.synced();
        let status = self.flags(|f| {
            change(&mut f.status);
            f.status.remote = remote;
            f.status.clone()
        });
        host.show(&status);
    }

    pub fn status(&self, host: &impl Host) -> Status {
        Status { remote: host.synced(), ..self.flags(|f| f.status.clone()) }
    }

    /// Forgets the last status (after choosing another database, or stopping sync).
    pub fn reset(&self, host: &impl Host) {
        self.set_status(host, |s| *s = Status::default());
        host.remote_key_needed(false);
    }

    /// An entry was created, changed or deleted: it goes up shortly.
    pub fn upload_soon(&self, host: &impl Host) {
        let at = host.now() + UPLOAD_DELAY;
        self.flags(|f| f.upload_at = Some(at));
    }

    /// The clock's tick: asks for the delayed upload when it is due, and for
    /// the regular check while unlocked.
    pub fn tick(&self, host: &impl Host) {
        let now = host.now();
        let every = host.check_every();
        let (upload, check) = self.flags(|f| {
            let upload = f.upload_at.is_some_and(|at| now >= at);
            let check = !f.running && every.is_some_and(|every| f.last_run.is_none_or(|last| now - last >= every));
            (upload, check)
        });
        // The regular check runs only while unlocked: a locked app cannot merge.
        if upload || (check && host.unlocked()) {
            self.request(host);
        }
    }

    pub fn is_running(&self) -> bool {
        self.flags(|f| f.running)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;

    const MINUTE: Duration = Duration::from_secs(60);

    /// The app as a test scripts it: what each sync answers, and what the
    /// driver showed and started. A run started in the background waits for
    /// the test to run it ([Scripted::run]), as the app's thread would.
    struct Scripted {
        driver: Driver,
        synced: Cell<bool>,
        unlocked: Cell<bool>,
        check_every: Option<Duration>,
        now: Cell<Instant>,
        /// What the syncs answer, in turn; up to date once it runs out.
        results: RefCell<VecDeque<Result<Outcome, SyncError>>>,
        /// How many times the next sync asks for another while it runs.
        asks_while_syncing: Cell<usize>,
        syncs: Cell<usize>,
        shown: RefCell<Vec<Status>>,
        key_needed: RefCell<Vec<bool>>,
        /// The runs started in the background and not run yet.
        started: Cell<usize>,
    }

    impl Scripted {
        fn new() -> Scripted {
            Scripted {
                driver: Driver::default(),
                synced: Cell::new(true),
                unlocked: Cell::new(true),
                check_every: None,
                now: Cell::new(Instant::now()),
                results: RefCell::default(),
                asks_while_syncing: Cell::new(0),
                syncs: Cell::new(0),
                shown: RefCell::default(),
                key_needed: RefCell::default(),
                started: Cell::new(0),
            }
        }

        fn checking_every(every: Duration) -> Scripted {
            Scripted { check_every: Some(every), ..Scripted::new() }
        }

        fn answering(results: impl IntoIterator<Item = Result<Outcome, SyncError>>) -> Scripted {
            let host = Scripted::new();
            host.results.borrow_mut().extend(results);
            host
        }

        fn after(&self, time: Duration) {
            self.now.set(self.now.get() + time);
        }

        /// Runs the one run started in the background, as the app's thread does.
        fn run(&self) {
            assert_eq!(self.started.replace(0), 1, "one run was started");
            self.driver.run(self);
        }

        fn busy_shown(&self) -> Vec<bool> {
            self.shown.borrow().iter().map(|s| s.busy).collect()
        }
    }

    impl Host for Scripted {
        fn synced(&self) -> bool {
            self.synced.get()
        }

        fn unlocked(&self) -> bool {
            self.unlocked.get()
        }

        fn check_every(&self) -> Option<Duration> {
            self.check_every
        }

        fn now(&self) -> Instant {
            self.now.get()
        }

        fn sync(&self) -> Result<Outcome, SyncError> {
            self.syncs.set(self.syncs.get() + 1);
            for _ in 0..self.asks_while_syncing.replace(0) {
                self.driver.request(self);
            }
            self.results.borrow_mut().pop_front().unwrap_or(Ok(Outcome::UpToDate))
        }

        fn describe(&self, result: &Result<Outcome, SyncError>) -> (String, bool) {
            (format!("{result:?}"), result.is_err())
        }

        fn show(&self, status: &Status) {
            self.shown.borrow_mut().push(status.clone());
        }

        fn remote_key_needed(&self, needed: bool) {
            self.key_needed.borrow_mut().push(needed);
        }

        fn run_in_background(&self) {
            self.started.set(self.started.get() + 1);
        }
    }

    #[test]
    fn a_request_runs_one_sync_in_the_background() {
        let host = Scripted::new();
        host.driver.request(&host);
        assert!(host.driver.is_running());
        assert_eq!(host.syncs.get(), 0, "not in the calling thread");
        host.run();
        assert_eq!(host.syncs.get(), 1);
        assert!(!host.driver.is_running());
        assert_eq!(host.busy_shown(), [true, false]);
    }

    #[test]
    fn a_local_database_is_never_synced() {
        let host = Scripted::checking_every(MINUTE);
        host.synced.set(false);
        host.driver.request(&host);
        host.driver.upload_soon(&host);
        host.after(MINUTE);
        host.driver.tick(&host);
        assert_eq!(host.driver.sync_first(&host), Ok(()));
        assert_eq!((host.started.get(), host.syncs.get()), (0, 0));
        assert!(!host.driver.is_running());
        assert!(!host.driver.status(&host).remote);
    }

    #[test]
    fn a_request_while_running_runs_once_more() {
        let host = Scripted::new();
        host.driver.request(&host);
        // Asked twice while the first runs: one more sync, not two, in the same run.
        host.asks_while_syncing.set(2);
        host.run();
        assert_eq!(host.syncs.get(), 2);
        assert_eq!(host.started.get(), 0, "no second run beside the one running");
        assert!(!host.driver.is_running());
        // Busy from the first sync to the end of the second.
        assert_eq!(host.busy_shown(), [true, true, true, false]);
        // The key is the last sync's to say.
        assert_eq!(*host.key_needed.borrow(), [false]);
    }

    #[test]
    fn syncing_first_is_refused_while_a_sync_runs() {
        let host = Scripted::new();
        host.driver.request(&host);
        let refused = host.driver.sync_first(&host).unwrap_err();
        assert_eq!(refused.to_string(), "A sync is running; try again in a moment");
        assert_eq!(host.syncs.get(), 0);
        // The run it was refused for goes on as it was.
        host.run();
        assert_eq!(host.syncs.get(), 1);
        assert!(!host.driver.is_running());
    }

    #[test]
    fn syncing_first_runs_in_the_calling_thread_and_says_why_it_failed() {
        let offline = || Err(SyncError::Offline("Offline".into()));
        let host = Scripted::answering([Ok(Outcome::Uploaded), offline(), Err(SyncError::OtherKey)]);
        assert_eq!(host.driver.sync_first(&host), Ok(()));
        assert_eq!((host.syncs.get(), host.started.get()), (1, 0));
        assert!(!host.driver.is_running());

        assert_eq!(host.driver.sync_first(&host), Err(OpenError::Other("Offline".into())));
        assert!(host.driver.status(&host).problem);

        assert!(matches!(host.driver.sync_first(&host), Err(OpenError::OtherKey(_))));
        assert_eq!(*host.key_needed.borrow(), [false, false, true]);
        assert!(!host.driver.is_running());
    }

    #[test]
    fn a_request_while_syncing_first_runs_after_it() {
        let host = Scripted::new();
        host.asks_while_syncing.set(1);
        assert_eq!(host.driver.sync_first(&host), Ok(()));
        assert_eq!(host.syncs.get(), 1, "not within the sync that had to come first");
        host.run();
        assert_eq!(host.syncs.get(), 2);
        assert!(!host.driver.is_running());
    }

    #[test]
    fn edits_in_a_row_make_one_upload() {
        let host = Scripted::new();
        host.driver.upload_soon(&host);
        host.after(Duration::from_secs(6));
        host.driver.upload_soon(&host);
        host.after(Duration::from_secs(6));
        host.driver.tick(&host);
        assert_eq!(host.started.get(), 0, "ten seconds after the last edit, not the first");
        host.after(Duration::from_secs(4));
        host.driver.tick(&host);
        host.run();
        assert_eq!(host.syncs.get(), 1);
        // Sent: nothing more goes up until the next edit.
        host.after(MINUTE);
        host.driver.tick(&host);
        assert_eq!(host.started.get(), 0);
    }

    #[test]
    fn a_sync_asked_for_takes_the_waiting_edits_with_it() {
        let host = Scripted::new();
        host.driver.upload_soon(&host);
        host.driver.request(&host);
        host.run();
        host.after(MINUTE);
        host.driver.tick(&host);
        assert_eq!((host.started.get(), host.syncs.get()), (0, 1));
    }

    #[test]
    fn the_regular_check_runs_only_while_unlocked() {
        let host = Scripted::checking_every(5 * MINUTE);
        host.unlocked.set(false);
        host.driver.tick(&host);
        assert_eq!(host.started.get(), 0, "a locked app cannot merge");
        // Edits made before the lock still go up.
        host.driver.upload_soon(&host);
        host.after(UPLOAD_DELAY);
        host.driver.tick(&host);
        host.run();
        assert_eq!(host.syncs.get(), 1);
    }

    #[test]
    fn the_regular_check_comes_every_so_often_after_the_last_sync() {
        let host = Scripted::checking_every(5 * MINUTE);
        // None ran yet: the first tick checks.
        host.driver.tick(&host);
        host.driver.tick(&host);
        host.run();
        host.after(4 * MINUTE);
        host.driver.tick(&host);
        assert_eq!(host.started.get(), 0);
        host.after(MINUTE);
        host.driver.tick(&host);
        // Not again while that one runs, however long it takes.
        host.after(10 * MINUTE);
        host.driver.tick(&host);
        host.run();
        assert_eq!(host.syncs.get(), 2);
    }

    #[test]
    fn no_regular_check_when_it_is_off() {
        let host = Scripted::new();
        host.after(60 * MINUTE);
        host.driver.tick(&host);
        assert_eq!(host.started.get(), 0);
    }

    #[test]
    fn the_status_says_what_the_last_sync_did_until_it_is_reset() {
        let host = Scripted::answering([Err(SyncError::OtherKey)]);
        host.driver.request(&host);
        host.run();
        let status = host.driver.status(&host);
        assert_eq!(status, Status { remote: true, busy: false, text: "Err(OtherKey)".into(), problem: true });
        assert_eq!(host.shown.borrow().last(), Some(&status));
        assert_eq!(*host.key_needed.borrow(), [true]);

        host.driver.reset(&host);
        assert_eq!(host.driver.status(&host), Status { remote: true, ..Status::default() });
        assert_eq!(*host.key_needed.borrow(), [true, false]);
        // Sync stopped meanwhile: the status follows without a sync.
        host.synced.set(false);
        assert_eq!(host.driver.status(&host), Status::default());
    }
}
