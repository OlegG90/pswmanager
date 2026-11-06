//! The syncs of the phone's database, as the app runs them (`docs/spec-android.md`,
//! *Synchronisation*): one at a time, a sync asked for while one runs runs
//! once more after it, a change to where the database syncs waits for none to
//! run, and what the last one did is kept for the status line. The sync itself
//! is [crate::sync]'s; the app hands each run the sync to do.

use crate::store::Store;
use crate::sync::{self, Outcome, SyncError};
use serde::Serialize;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Mutex;
use std::time::Duration;

/// What a sync did, for the status line (the page's event `synced`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub text: String,
    pub problem: bool,
    /// The entries changed: the list is read again.
    pub changed: bool,
    /// The store wants the user to sign in again.
    pub sign_in: bool,
    /// The visible copy could not be written, and why.
    pub copy_problem: Option<String>,
    /// The remote file opens with a key this phone does not know: another
    /// device changed it, and the page offers to enter it.
    pub other_key: bool,
}

impl Report {
    /// What `result` did.
    pub fn of(store: &Store, result: &Result<Outcome, SyncError>) -> Report {
        let (text, problem) = sync::describe(store, result);
        Report {
            text,
            problem,
            changed: matches!(result, Ok(Outcome::Downloaded(c) | Outcome::Merged(c)) if !c.is_empty()),
            sign_in: matches!(result, Err(SyncError::SignIn(_))),
            copy_problem: None,
            other_key: matches!(result, Err(SyncError::OtherKey)),
        }
    }

    /// With why the visible copy was not written after the sync, if it was not: a problem too.
    pub fn with_copy_problem(self, copy_problem: Option<String>) -> Report {
        Report { problem: self.problem || copy_problem.is_some(), copy_problem, ..self }
    }
}

/// [Syncs::first]'s answer when the remote file is on a key this phone does not know.
pub const OTHER_KEY: &str = "The remote file has another master password or key file";

/// The answer to a change asked for while a sync runs.
const SYNC_RUNNING: &str = "A sync is running; try again in a moment";

#[derive(Default)]
struct Flags {
    running: bool,
    again: bool,
}

/// The syncs of the database, one at a time, and the last one's report.
#[derive(Default)]
pub struct Syncs {
    flags: Mutex<Flags>,
    last: Mutex<Option<Report>>,
}

impl Syncs {
    /// True when a sync may start now, to be run with [Syncs::drain];
    /// otherwise the one running goes again.
    pub fn begin(&self) -> bool {
        let mut flags = self.flags.lock().unwrap();
        if flags.running {
            flags.again = true;
            return false;
        }
        flags.running = true;
        true
    }

    fn is_running(&self) -> bool {
        self.flags.lock().unwrap().running
    }

    /// Ends a run that will not go again; true when one was asked for meanwhile.
    fn finish(&self) -> bool {
        let mut flags = self.flags.lock().unwrap();
        flags.running = false;
        std::mem::take(&mut flags.again)
    }

    /// True when it was asked again meanwhile: run once more.
    fn again(&self) -> bool {
        let mut flags = self.flags.lock().unwrap();
        flags.running = std::mem::take(&mut flags.again);
        flags.running
    }

    /// Runs `once` (the sync [Syncs::begin] let start) until none was asked
    /// for meanwhile; whether the last one is settled ([sync::settled]). A
    /// sync that panics does not stop the ones after it, and is not settled.
    pub fn drain(&self, mut once: impl FnMut() -> bool) -> bool {
        loop {
            let settled = catch_unwind(AssertUnwindSafe(&mut once)).unwrap_or(false);
            if !self.again() {
                return settled;
            }
        }
    }

    /// Waits for a sync that runs (`tries` times `pause` at most), then runs
    /// `once` as [Syncs::drain] does; false when its turn never came.
    pub fn after_running(&self, tries: u32, pause: Duration, once: impl FnMut() -> bool) -> bool {
        for _ in 0..tries {
            if !self.is_running() && self.begin() {
                return self.drain(once);
            }
            std::thread::sleep(pause);
        }
        false
    }

    /// Runs `change` to where the database syncs while no sync runs (refused
    /// while one does); a sync asked for meanwhile is started after it with
    /// `sync_after`, with the change.
    pub fn while_not_syncing<T>(&self, change: impl FnOnce() -> Result<T, String>, sync_after: impl FnOnce()) -> Result<T, String> {
        if !self.begin() {
            return Err(SYNC_RUNNING.into());
        }
        // A change that panics must not leave the syncs stopped.
        let result = catch_unwind(AssertUnwindSafe(change)).unwrap_or_else(|_| Err("Something went wrong".into()));
        if self.finish() {
            sync_after();
        }
        result
    }

    /// Syncs now with `once`, waiting for it: before a change the remote file
    /// must be in step with (a new key: after it, the remote file, still on
    /// the old key, opens no more). Refused while a sync runs, and when it
    /// does not settle; `sync_after` as [Syncs::while_not_syncing].
    pub fn first(&self, once: impl FnOnce() -> bool, sync_after: impl FnOnce()) -> Result<(), String> {
        let settled = self.while_not_syncing(|| Ok(once()), sync_after)?;
        match self.last() {
            Some(last) if last.other_key => Err(OTHER_KEY.into()),
            _ if settled => Ok(()),
            last => Err(format!("The database could not sync first ({})", last.map(|s| s.text).unwrap_or_default())),
        }
    }

    /// Keeps `report` as the last sync's (for a page that missed it) and gives it back.
    pub fn keep(&self, report: Report) -> Report {
        *self.last.lock().unwrap() = Some(report.clone());
        report
    }

    /// What the last sync did, if one ran since the database synced from here.
    pub fn last(&self) -> Option<Report> {
        self.last.lock().unwrap().clone()
    }

    /// Forgets the last sync's report (syncing with the store stopped).
    pub fn forget(&self) {
        *self.last.lock().unwrap() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::sync::Arc;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::load(dir.path().join("pswm.json"));
        (dir, store)
    }

    fn report(other_key: bool) -> Report {
        Report { text: "Offline".into(), problem: true, changed: false, sign_in: false, copy_problem: None, other_key }
    }

    #[test]
    fn a_sync_asked_for_while_one_runs_runs_once_after_it() {
        let syncs = Syncs::default();
        assert!(syncs.begin());
        let runs = Cell::new(0);
        let settled = syncs.drain(|| {
            runs.set(runs.get() + 1);
            // Asked twice while the first runs: one more run, not two.
            if runs.get() == 1 {
                assert!(!syncs.begin());
                assert!(!syncs.begin());
            }
            true
        });
        assert!(settled);
        assert_eq!(runs.get(), 2);
        assert!(!syncs.is_running());
        assert!(syncs.begin());
    }

    #[test]
    fn a_sync_that_panics_does_not_stop_the_next() {
        let syncs = Syncs::default();
        assert!(syncs.begin());
        let runs = Cell::new(0);
        let settled = syncs.drain(|| {
            runs.set(runs.get() + 1);
            if runs.get() == 1 {
                syncs.begin();
                panic!("a sync went wrong");
            }
            false
        });
        assert!(!settled);
        assert_eq!(runs.get(), 2);
        assert!(!syncs.is_running());
    }

    #[test]
    fn a_change_waits_for_no_sync_and_starts_the_one_asked_for_meanwhile() {
        let syncs = Syncs::default();
        assert!(syncs.begin());
        let refused: Result<(), String> = syncs.while_not_syncing(|| Ok(()), || panic!("no sync after a refusal"));
        assert_eq!(refused, Err(SYNC_RUNNING.into()));
        syncs.drain(|| true);

        let started = Cell::new(false);
        let changed = syncs.while_not_syncing(|| Ok(syncs.begin()), || started.set(true));
        // The sync asked for during the change does not start then: after it.
        assert_eq!(changed, Ok(false));
        assert!(started.get());
        assert!(!syncs.is_running());

        let started = Cell::new(false);
        let failed: Result<(), String> = syncs.while_not_syncing(|| panic!("the change went wrong"), || started.set(true));
        assert_eq!(failed, Err("Something went wrong".into()));
        assert!(!started.get());
        assert!(syncs.begin());
    }

    #[test]
    fn syncing_first_needs_a_settled_sync_on_a_known_key() {
        let syncs = Syncs::default();
        assert_eq!(syncs.first(|| true, || {}), Ok(()));
        syncs.keep(report(false));
        assert_eq!(syncs.first(|| false, || {}), Err("The database could not sync first (Offline)".into()));
        syncs.keep(report(true));
        assert_eq!(syncs.first(|| true, || {}), Err(OTHER_KEY.into()));
        syncs.forget();
        assert_eq!(syncs.last(), None);
        assert_eq!(syncs.first(|| false, || {}), Err("The database could not sync first ()".into()));
    }

    #[test]
    fn the_background_upload_waits_its_turn() {
        let syncs = Arc::new(Syncs::default());
        assert!(syncs.begin());
        assert!(!syncs.after_running(2, Duration::from_millis(1), || true));
        let running = syncs.clone();
        let ended = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            running.drain(|| true)
        });
        assert!(syncs.after_running(500, Duration::from_millis(5), || true));
        assert!(ended.join().unwrap());
        assert!(!syncs.is_running());
    }

    #[test]
    fn the_report_says_what_the_sync_did() {
        let (_dir, store) = store();
        let merged = Report::of(&store, &Ok(Outcome::Merged(vec!["Mail".into()])));
        assert!(merged.changed && !merged.problem && !merged.sign_in && !merged.other_key);
        assert!(!Report::of(&store, &Ok(Outcome::Downloaded(Vec::new()))).changed);
        assert!(!Report::of(&store, &Ok(Outcome::Uploaded)).problem);
        let copy = Report::of(&store, &Ok(Outcome::UpToDate)).with_copy_problem(Some("No folder".into()));
        assert!(copy.problem);
        assert_eq!(copy.copy_problem.as_deref(), Some("No folder"));
        let sign_in = Report::of(&store, &Err(SyncError::SignIn("Sign in to Dropbox again".into())));
        assert!(sign_in.sign_in && sign_in.problem);
        assert_eq!(sign_in.text, "Sign in to Dropbox again");
        assert!(Report::of(&store, &Err(SyncError::OtherKey)).other_key);
    }

    #[test]
    fn the_report_reads_as_the_page_expects() {
        let json = serde_json::to_value(report(true)).unwrap();
        let keys: Vec<&str> = json.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys.len(), 6);
        for key in ["text", "problem", "changed", "signIn", "copyProblem", "otherKey"] {
            assert!(keys.contains(&key), "{key}");
        }
    }
}
