//! Keeping the working copy and the remote file in step. At each sync the app
//! compares both with where the last sync left them: an unchanged side takes
//! the other's file, and when both changed they are merged.

use crate::dbfile::{hash_hex, sibling};
use crate::remote::{Remote, RemoteError};
use crate::settings::Settings;
use crate::store::{self, Store};
use crate::Session;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// Edits go up this long after the last one, so a few edits in a row make one upload.
const UPLOAD_DELAY: Duration = Duration::from_secs(10);
/// Quitting waits this long at most for the last upload.
const QUIT_WAIT: Duration = Duration::from_secs(10);
const ATTEMPTS: usize = 3;

#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Both were already the same.
    UpToDate,
    Uploaded,
    /// The remote file replaced the working copy; the entries that differ.
    Downloaded(Vec<String>),
    /// Both had changed: merged and uploaded; the entries that differ.
    Merged(Vec<String>),
    /// The remote file changed, and merging needs the key: at the next unlock.
    WaitingForUnlock,
}

#[derive(Debug, PartialEq)]
pub enum SyncError {
    Offline(String),
    /// The account needs signing in again.
    SignIn(String),
    Failed(String),
}

impl From<RemoteError> for SyncError {
    fn from(e: RemoteError) -> Self {
        match e {
            RemoteError::Offline(message) => SyncError::Offline(message),
            RemoteError::SignIn(message) => SyncError::SignIn(message),
            other => SyncError::Failed(other.message()),
        }
    }
}

fn failed(message: impl ToString) -> SyncError {
    SyncError::Failed(message.to_string())
}

/// Syncs the working copy with `remote`. Uploads need nothing unlocked; a
/// changed remote file is read with the unlocked database's key.
pub fn sync(remote: &dyn Remote, store: &Store, session: &Session) -> Result<Outcome, SyncError> {
    for _ in 0..ATTEMPTS {
        if let Some(outcome) = attempt(remote, store, session)? {
            return Ok(outcome);
        }
    }
    Err(failed("The remote file keeps changing; try again in a moment"))
}

/// One pass of the decision; `None` when either file changed meanwhile.
fn attempt(remote: &dyn Remote, store: &Store, session: &Session) -> Result<Option<Outcome>, SyncError> {
    let (working, state) = synced(store).ok_or_else(|| failed("The database is not synced"))?;
    let revision = remote.revision()?;
    let mut outcome = Outcome::UpToDate;
    if revision != state.revision {
        if revision.is_none() {
            // The remote file is gone: the upload below puts it back.
            update(store, &working, |r| r.revision = None)?;
        } else {
            let Ok(Some((since, unsaved))) = session.read(|v| v.snapshot().map(|s| (s, v.has_unsaved()))) else {
                return Ok(Some(Outcome::WaitingForUnlock));
            };
            let changed_here = unsaved || Some(working_hash(&working)?) != state.synced;
            let (bytes, revision) = remote.download()?;
            // Deriving the key takes a while: done without holding the database.
            let theirs = since.parse(&bytes).map_err(|e| failed(format!("The remote copy cannot be opened: {e}")))?;
            outcome = match session.with_mut(|v| v.take_remote(&since, theirs, &bytes, changed_here)) {
                Ok(Some(taken)) => taken,
                Ok(None) => return Ok(None),
                Err(_) if !session.is_unlocked() => return Ok(Some(Outcome::WaitingForUnlock)),
                Err(message) => return Err(failed(message)),
            };
            let merged = matches!(outcome, Outcome::Merged(_));
            if merged {
                // What the upload below replaces, in case the merge got it wrong.
                fs::write(sibling(&working, ".remote.bak"), &bytes).map_err(|e| failed(format!("Cannot keep the remote copy: {e}")))?;
            }
            update(store, &working, |r| {
                r.revision = Some(revision);
                if !merged {
                    r.synced = Some(hash_hex(&bytes)); // the working copy is now exactly the remote file
                }
            })?;
        }
    }

    // What the working copy has that the remote file lacks goes up.
    let state = match synced(store) {
        Some((now, state)) if now == working => state,
        _ => return Err(failed("Another database was chosen meanwhile")),
    };
    let bytes = read_working(&working)?;
    let hash = hash_hex(&bytes);
    if state.revision.is_none() || state.synced.as_ref() != Some(&hash) {
        match remote.upload(&bytes, state.revision.as_deref()) {
            Ok(revision) => update(store, &working, |r| {
                r.revision = Some(revision);
                r.synced = Some(hash);
            })?,
            Err(RemoteError::Changed) => return Ok(None),
            Err(e) => return Err(e.into()),
        }
        if outcome == Outcome::UpToDate {
            outcome = Outcome::Uploaded;
        }
    }
    Ok(Some(outcome))
}

/// The working copy and the remote it is synced with, if it is.
fn synced(store: &Store) -> Option<(PathBuf, store::Remote)> {
    store.read(|s| {
        let current = s.current()?;
        Some((current.file.clone(), current.remote.clone()?))
    })
}

/// Changes the sync state of the database whose file is `working` — the one
/// this sync read, even if another became current meanwhile.
fn update(store: &Store, working: &Path, change: impl FnOnce(&mut store::Remote)) -> Result<(), SyncError> {
    store
        .update(|s| {
            let known = s.databases.iter_mut().find(|d| d.file == working);
            if let Some(remote) = known.and_then(|d| d.remote.as_mut()) {
                change(remote);
            }
        })
        .map_err(|e| failed(format!("Cannot save the sync state: {e}")))
}

fn read_working(path: &Path) -> Result<Vec<u8>, SyncError> {
    fs::read(path).map_err(|e| failed(format!("Cannot read the working copy: {e}")))
}

fn working_hash(path: &Path) -> Result<String, SyncError> {
    read_working(path).map(|bytes| hash_hex(&bytes))
}

/// True when the current database's file has changes its remote file lacks.
pub fn has_pending(store: &Store) -> bool {
    store.read(|s| s.current().is_some_and(is_pending))
}

/// True when a database's file has changes its remote file lacks (a missing
/// file has none).
pub fn is_pending(known: &store::Known) -> bool {
    known.remote.as_ref().is_some_and(|r| working_hash(&known.file).is_ok_and(|hash| r.synced != Some(hash)))
}

/// Adds the remote file at `location` to the list as the current database:
/// it is downloaded to `local`, a file the user chose, which it then syncs
/// with. A file already at `local` is kept as `<name>.bak`.
pub fn start(store: &Store, location: crate::remote::Location, local: PathBuf) -> Result<(), String> {
    if store.read(|s| s.databases.iter().any(|d| d.file == local)) {
        return Err(format!("{} is already in the list: choose another place", local.display()));
    }
    if local.exists() {
        fs::copy(&local, sibling(&local, ".bak")).map_err(|e| format!("Cannot keep the file that was there: {e}"))?;
    }
    let remote = download_into(&local, location)?;
    store
        .update(|s| {
            s.select(local).remote = Some(remote);
        })
        .map_err(|e| format!("Cannot save the choice: {e}"))
}

/// Starts syncing the database whose file is `file` with `location`, a
/// remote file just made from it: nothing is downloaded.
pub fn attach(store: &Store, file: &Path, location: crate::remote::Location) -> Result<(), String> {
    let revision = location.open().revision().map_err(|e| e.message())?;
    let synced = Some(working_hash(file).map_err(|e| format!("{e:?}"))?);
    store
        .update(|s| {
            if let Some(known) = s.databases.iter_mut().find(|d| d.file == file) {
                known.remote = Some(store::Remote { location, revision, synced });
            }
        })
        .map_err(|e| format!("Cannot save the sync state: {e}"))
}

/// Downloads the remote file into `working`; the sync state that goes with it.
fn download_into(working: &Path, location: crate::remote::Location) -> Result<store::Remote, String> {
    let (bytes, revision) = location.open().download().map_err(|e| e.message())?;
    store::write_atomically(working, &bytes).map_err(|e| format!("Cannot write the working copy: {e}"))?;
    Ok(store::Remote { location, revision: Some(revision), synced: Some(hash_hex(&bytes)) })
}

/// `<dir>/<name>`, or `<dir>/<stem> (2).kdbx` and so on: a path no file has
/// and no database in the list uses.
fn free_path(dir: &Path, file_name: &str, taken: &[PathBuf]) -> PathBuf {
    let stem = Path::new(file_name).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "database".into());
    (1..)
        .map(|n| if n == 1 { format!("{stem}.kdbx") } else { format!("{stem} ({n}).kdbx") })
        .map(|file| dir.join(file))
        .find(|path| !path.exists() && !taken.contains(path))
        .expect("some name is free")
}

/// Moves a working copy out of `sync/` into the data folder, under a name
/// no file there has yet, and returns where it went.
pub fn keep_as_local(store: &Store, working: &Path) -> Result<PathBuf, String> {
    let taken: Vec<PathBuf> = store.read(|s| s.databases.iter().map(|d| d.file.clone()).collect());
    let name = working.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let target = free_path(store.dir(), &name, &taken);
    fs::rename(working, &target).map_err(|e| format!("Cannot keep the working copy: {e}"))?;
    Ok(target)
}

/// Downloads the working copy again when it is missing (deleted, or a new PC
/// with a copied state file), so unlocking has a file to open.
pub fn ensure_working_copy(store: &Store) -> Result<(), String> {
    let Some((working, state)) = synced(store) else { return Ok(()) };
    if working.exists() {
        return Ok(());
    }
    let remote = download_into(&working, state.location)?;
    store
        .update(|s| {
            if let Some(known) = s.databases.iter_mut().find(|d| d.file == working) {
                known.remote = Some(remote);
            }
        })
        .map_err(|e| format!("Cannot save the sync state: {e}"))
}

// ------------------------------------------------------------ in the app

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
    std::thread::spawn(move || loop {
        set_status(&app, |s| s.busy = true);
        let result = run_once(&app);
        let status = describe(&app, &result);
        let again = flags(&app, |f| {
            f.last_run = Some(Instant::now());
            f.running = f.again;
            std::mem::take(&mut f.again)
        });
        set_status(&app, |s| *s = Status { busy: again, ..status.clone() });
        if !again {
            break;
        }
    });
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

/// Forgets the last status (after choosing another database).
pub fn reset(app: &AppHandle) {
    set_status(app, |s| *s = Status::default());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::{Folder, Location};
    use crate::store::State;
    use crate::vault::Vault;
    use keepass::db::fields;
    use keepass::{Database, DatabaseKey};
    use std::fs::File;

    struct Setup {
        _dir: tempfile::TempDir,
        store: Store,
        session: Session,
        remote: Folder,
    }

    fn key() -> DatabaseKey {
        DatabaseKey::new().with_password("test")
    }

    /// A remote folder holding the fixture, synced and unlocked.
    fn setup() -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let share = dir.path().join("share");
        fs::create_dir(&share).unwrap();
        let path = share.join("base.kdbx");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sic2kdbx.kdbx");
        fs::copy(fixture, &path).unwrap();
        let store = Store::load(dir.path().join("data").join("pswm.json"));
        start(&store, Location::Folder { path: path.clone() }, dir.path().join("local").join("base.kdbx")).unwrap();
        let session = Session::default();
        let working = store.read(|s| s.current.clone()).unwrap();
        session.set(Some(Vault::open(&working, Some("test"), None).unwrap()));
        Setup { _dir: dir, store, session, remote: Folder { path } }
    }

    impl Setup {
        fn sync(&self) -> Result<Outcome, SyncError> {
            sync(&self.remote, &self.store, &self.session)
        }

        /// Another device changes the remote file.
        fn elsewhere(&self, change: impl FnOnce(&mut Database)) {
            let mut db = Database::open(&mut File::open(&self.remote.path).unwrap(), key()).unwrap();
            change(&mut db);
            db.config.version = keepass::config::DatabaseVersion::KDB4(1);
            db.save(&mut File::create(&self.remote.path).unwrap(), key()).unwrap();
        }

        fn edit_here(&self, title: &str, password: &str) {
            self.session
                .with_mut(|v| {
                    let id = v.listing().entries.into_iter().find(|e| e.title == title).unwrap().id;
                    let mut data = v.edit_data(&id).unwrap();
                    data.password = password.into();
                    v.save_entry(Some(&id), None, &data).map(|_| ())
                })
                .unwrap();
        }

        fn remote_titles(&self) -> Vec<String> {
            let vault = Vault::open(&self.remote.path, Some("test"), None).unwrap();
            vault.listing().entries.into_iter().map(|e| e.title).collect()
        }

        fn remote_password(&self, title: &str) -> String {
            let vault = Vault::open(&self.remote.path, Some("test"), None).unwrap();
            let id = vault.listing().entries.into_iter().find(|e| e.title == title).unwrap().id;
            vault.field(&id, fields::PASSWORD).unwrap().to_string()
        }

        fn password_here(&self, title: &str) -> String {
            self.session
                .read(|v| {
                    let id = v.listing().entries.into_iter().find(|e| e.title == title).unwrap().id;
                    v.field(&id, fields::PASSWORD).unwrap().to_string()
                })
                .unwrap()
        }
    }

    #[test]
    fn nothing_changed_nothing_happens() {
        let s = setup();
        assert_eq!(s.sync(), Ok(Outcome::UpToDate));
    }

    #[test]
    fn a_change_here_goes_up() {
        let s = setup();
        s.edit_here("Mail", "from the PC");
        assert!(has_pending(&s.store));
        assert_eq!(s.sync(), Ok(Outcome::Uploaded));
        assert_eq!(s.remote_password("Mail"), "from the PC");
        assert!(!has_pending(&s.store));
        assert_eq!(s.sync(), Ok(Outcome::UpToDate));
    }

    #[test]
    fn a_change_there_comes_down_as_it_is() {
        let s = setup();
        s.elsewhere(|db| db.root_mut().add_entry().set_unprotected(fields::TITLE, "Added on the phone"));
        let Ok(Outcome::Downloaded(changed)) = s.sync() else { panic!() };
        assert_eq!(changed.len(), 1);
        let working = s.store.read(|st| st.current.clone()).unwrap();
        assert_eq!(fs::read(working).unwrap(), fs::read(&s.remote.path).unwrap());
        assert_eq!(s.sync(), Ok(Outcome::UpToDate));
    }

    #[test]
    fn changes_on_both_sides_are_merged_and_go_up() {
        let s = setup();
        s.edit_here("Mail", "from the PC");
        s.elsewhere(|db| db.root_mut().add_entry().set_unprotected(fields::TITLE, "Added on the phone"));
        assert!(matches!(s.sync(), Ok(Outcome::Merged(_))));
        assert!(s.remote_titles().contains(&"Added on the phone".to_string()));
        assert_eq!(s.remote_password("Mail"), "from the PC");
        assert_eq!(s.password_here("Mail"), "from the PC");
        let working = s.store.read(|st| st.current.clone()).unwrap();
        assert!(sibling(&working, ".remote.bak").exists());
        assert_eq!(s.sync(), Ok(Outcome::UpToDate));
    }

    #[test]
    fn an_upload_over_a_file_changed_meanwhile_merges_first() {
        /// Another device uploads between this device's check and its upload.
        struct Racing<'a>(&'a Setup, Mutex<bool>);
        impl Remote for Racing<'_> {
            fn revision(&self) -> Result<Option<String>, RemoteError> {
                self.0.remote.revision()
            }
            fn download(&self) -> Result<(Vec<u8>, String), RemoteError> {
                self.0.remote.download()
            }
            fn upload(&self, bytes: &[u8], expected: Option<&str>) -> Result<String, RemoteError> {
                if std::mem::take(&mut *self.1.lock().unwrap()) {
                    self.0.elsewhere(|db| db.root_mut().add_entry().set_unprotected(fields::TITLE, "Raced in"));
                }
                self.0.remote.upload(bytes, expected)
            }
        }
        let s = setup();
        s.edit_here("Mail", "from the PC");
        let racing = Racing(&s, Mutex::new(true));
        assert!(matches!(sync(&racing, &s.store, &s.session), Ok(Outcome::Merged(_))));
        assert!(s.remote_titles().contains(&"Raced in".to_string()));
        assert_eq!(s.remote_password("Mail"), "from the PC");
    }

    #[test]
    fn while_locked_changes_go_up_but_a_merge_waits() {
        let s = setup();
        s.edit_here("Mail", "from the PC");
        s.session.set(None);
        assert_eq!(s.sync(), Ok(Outcome::Uploaded));

        s.elsewhere(|db| db.root_mut().add_entry().set_unprotected(fields::TITLE, "Added on the phone"));
        let before = fs::read(&s.remote.path).unwrap();
        assert_eq!(s.sync(), Ok(Outcome::WaitingForUnlock));
        assert_eq!(fs::read(&s.remote.path).unwrap(), before);
    }

    #[test]
    fn an_unreadable_remote_file_changes_nothing() {
        let s = setup();
        s.edit_here("Mail", "from the PC");
        fs::write(&s.remote.path, b"half-uploaded").unwrap();
        assert!(matches!(s.sync(), Err(SyncError::Failed(_))));
        assert_eq!(fs::read(&s.remote.path).unwrap(), b"half-uploaded");
        assert_eq!(s.password_here("Mail"), "from the PC");
    }

    #[test]
    fn an_unreachable_folder_is_offline_and_the_changes_wait() {
        let s = setup();
        s.edit_here("Mail", "from the PC");
        let gone = Folder { path: s.remote.path.parent().unwrap().join("gone").join("base.kdbx") };
        assert!(matches!(sync(&gone, &s.store, &s.session), Err(SyncError::Offline(_))));
        assert!(has_pending(&s.store));
        assert_eq!(s.sync(), Ok(Outcome::Uploaded));
    }

    #[test]
    fn a_working_copy_kept_as_local_leaves_sync_and_takes_a_free_name() {
        let s = setup();
        let working = s.store.read(|st| st.current.clone()).unwrap();
        fs::write(s.store.dir().join("base.kdbx"), b"another").unwrap();
        let kept = keep_as_local(&s.store, &working).unwrap();
        assert_eq!(kept, s.store.dir().join("base (2).kdbx"));
        assert!(!working.exists());
    }

    #[test]
    fn two_remote_files_of_one_name_get_their_own_working_copies() {
        let s = setup();
        let first = s.store.read(|st| st.current.clone()).unwrap();
        let other = s.remote.path.parent().unwrap().join("other");
        fs::create_dir(&other).unwrap();
        fs::copy(&s.remote.path, other.join("base.kdbx")).unwrap();
        start(&s.store, Location::Folder { path: other.join("base.kdbx") }, s.store.dir().join("other").join("base.kdbx")).unwrap();
        let second = s.store.read(|st| st.current.clone()).unwrap();
        assert_ne!(first, second);
        assert_eq!(s.store.read(|st| st.databases.len()), 2);
        assert!(first.exists() && second.exists());
    }

    #[test]
    fn a_sync_keeps_its_state_on_its_own_database() {
        let s = setup();
        s.edit_here("Mail", "from the PC");
        let synced_file = s.store.read(|st| st.current.clone()).unwrap();
        // Another database becomes current while this one syncs.
        let other = s.remote.path.parent().unwrap().join("other.kdbx");
        s.store.update(|st| st.current = Some(other.clone())).unwrap();
        s.store.update(|st| st.select(other.clone()).key_file = None).unwrap();
        s.store.update(|st| st.current = Some(synced_file.clone())).unwrap();
        let (_, before) = synced(&s.store).unwrap();
        s.store.update(|st| st.current = Some(other.clone())).unwrap();
        let bytes = fs::read(&synced_file).unwrap();
        update(&s.store, &synced_file, |r| r.synced = Some(hash_hex(&bytes))).unwrap();
        let state = s.store.read(State::clone);
        assert!(state.databases.iter().find(|d| d.file == other).unwrap().remote.is_none());
        assert_ne!(state.databases.iter().find(|d| d.file == synced_file).unwrap().remote.as_ref().unwrap().synced, before.synced);
    }

    #[test]
    fn a_missing_working_copy_is_downloaded_again() {
        let s = setup();
        let working = s.store.read(|st| st.current.clone()).unwrap();
        fs::remove_file(&working).unwrap();
        ensure_working_copy(&s.store).unwrap();
        assert_eq!(fs::read(working).unwrap(), fs::read(&s.remote.path).unwrap());
    }
}
