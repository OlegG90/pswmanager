//! A copy of each database, in a folder the user chose, every few days: the
//! database file as it is on this PC (encrypted), so it runs while locked too.
//! One copy is kept, replaced each time.

use crate::dbfile::{replace_file, sibling};
use crate::store::{Backup, Known, State, Store};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How often the list is checked for a database whose backup is due.
const CHECK_EVERY: Duration = Duration::from_secs(60 * 60);
const DAY: u64 = 24 * 60 * 60;

/// Seconds since 1970, as a backup's time is kept.
pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// When the next backup is due, or `None` for none (never, or no folder).
pub fn next(backup: &Backup) -> Option<u64> {
    backup.folder.as_ref()?;
    (backup.every_days > 0).then(|| backup.last.map_or(0, |last| last + u64::from(backup.every_days) * DAY))
}

/// The copy of `database` in `folder`: `base.kdbx` → `base.backup.kdbx`.
pub fn copy_path(database: &Path, folder: &Path) -> PathBuf {
    let name = database.file_name().unwrap_or_default().to_string_lossy();
    folder.join(format!("{}.backup.kdbx", name.strip_suffix(".kdbx").unwrap_or(&name)))
}

/// Copies `database` byte for byte into `folder`, replacing the copy there;
/// written beside it first, so a copy is never half-written.
pub fn copy(database: &Path, folder: &Path) -> Result<(), String> {
    if !folder.is_dir() {
        return Err(format!("The backup folder is not there: {}", folder.display()));
    }
    let bytes = fs::read(database).map_err(|e| format!("Cannot read the database file: {e}"))?;
    let to = copy_path(database, folder);
    replace_file(&to, &sibling(&to, ".tmp"), &bytes).map_err(|e| format!("Cannot write the backup: {e}"))
}

/// One backup at a time: the hourly check, a changed setting and *Backup now*
/// would otherwise write the same `.tmp` file together.
static RUNNING: Mutex<()> = Mutex::new(());

/// Another database in the list whose copy would be the same file as
/// `database`'s in `folder` (the same file name): its name, if any.
pub fn copy_taken(state: &State, database: &Path, folder: &Path) -> Option<String> {
    let ours = copy_path(database, folder);
    state
        .databases
        .iter()
        .filter(|d| d.file != database)
        .find(|d| d.backup.as_ref().and_then(|b| b.folder.as_deref()).is_some_and(|f| crate::dbfile::same_file(&copy_path(&d.file, f), &ours)))
        .map(Known::title)
}

/// Backs up `database` now, whatever its interval, and keeps when, or why it
/// failed. Refused when it has no backup folder.
pub fn run(store: &Store, database: &Path) -> Result<(), String> {
    let _one = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    let folder = store.read(|s| backup_of(s, database)?.folder.clone()).ok_or("Choose a backup folder first")?;
    let result = copy(database, &folder);
    store
        .update(|s| {
            if let Some(backup) = s.databases.iter_mut().find(|d| d.file == database).and_then(|d| d.backup.as_mut()) {
                match &result {
                    Ok(()) => (backup.last, backup.error) = (Some(now()), None),
                    Err(message) => backup.error = Some(message.clone()),
                }
            }
        })
        .map_err(|e| format!("Cannot save the backup state: {e}"))?;
    result
}

fn backup_of<'a>(state: &'a State, database: &Path) -> Option<&'a Backup> {
    state.databases.iter().find(|d| d.file == database)?.backup.as_ref()
}

/// Backs up each database in the list (or only `only`) whose backup is due at
/// `now`. A failure is kept on its settings and tried again at the next check.
pub fn run_due(store: &Store, now: u64, only: Option<&Path>) {
    let due: Vec<PathBuf> = store.read(|s| {
        s.databases
            .iter()
            .filter(|d| only.is_none_or(|only| d.file == only))
            .filter(|d| d.backup.as_ref().and_then(next).is_some_and(|at| at <= now))
            .map(|d| d.file.clone())
            .collect()
    });
    for database in due {
        let _ = run(store, &database);
    }
}

/// Checks at start and every hour while the app runs.
pub fn start_clock(app: tauri::AppHandle) {
    use tauri::Manager;
    std::thread::spawn(move || loop {
        run_due(&app.state::<Store>(), now(), None);
        std::thread::sleep(CHECK_EVERY);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backup(folder: &Path, every_days: u32, last: Option<u64>) -> Backup {
        Backup { folder: Some(folder.to_path_buf()), every_days, last, error: None }
    }

    /// A store with one database, `base.kdbx`, backed up as `b`.
    fn store_with(dir: &Path, b: Option<Backup>) -> (Store, PathBuf) {
        let database = dir.join("base.kdbx");
        fs::write(&database, b"the database").unwrap();
        let store = Store::load(dir.join("pswm.json"));
        store.update(|s| s.select(database.clone()).backup = b).unwrap();
        (store, database)
    }

    #[test]
    fn the_next_backup_follows_the_interval() {
        let folder = Path::new("D:/Backups");
        assert_eq!(next(&backup(folder, 7, None)), Some(0), "never backed up: due now");
        assert_eq!(next(&backup(folder, 7, Some(100))), Some(100 + 7 * DAY));
        assert_eq!(next(&backup(folder, 0, Some(100))), None, "never");
        assert_eq!(next(&Backup { folder: None, ..backup(folder, 7, None) }), None, "no folder");
    }

    #[test]
    fn a_due_database_is_copied_byte_for_byte_and_replaces_the_copy() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("backups");
        fs::create_dir(&folder).unwrap();
        let (store, database) = store_with(dir.path(), Some(backup(&folder, 2, None)));
        run_due(&store, now(), None);
        let copy = copy_path(&database, &folder);
        assert_eq!(copy.file_name().unwrap(), "base.backup.kdbx");
        assert_eq!(fs::read(&copy).unwrap(), b"the database");
        let last = store.read(|s| s.current().unwrap().backup.clone().unwrap().last).unwrap();

        // Not due again yet: nothing written.
        fs::write(&database, b"changed").unwrap();
        run_due(&store, last + DAY, None);
        assert_eq!(fs::read(&copy).unwrap(), b"the database");
        // Due: the one copy is replaced.
        run_due(&store, last + 2 * DAY, None);
        assert_eq!(fs::read(&copy).unwrap(), b"changed");
        // One file: no half-written `.tmp` left beside it.
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 1);
    }

    #[test]
    fn two_databases_of_one_file_name_cannot_share_a_folder() {
        let folder = Path::new(r"D:\Backups");
        let mut state = State::default();
        state.select(r"C:\Home\x.kdbx".into()).backup = Some(backup(folder, 7, None));
        state.select(r"C:\Work\x.kdbx".into());
        state.select(r"C:\Work\y.kdbx".into());
        assert_eq!(copy_taken(&state, Path::new(r"C:\Work\x.kdbx"), folder).as_deref(), Some("x.kdbx"));
        assert_eq!(copy_taken(&state, Path::new(r"C:\Work\y.kdbx"), folder), None);
        assert_eq!(copy_taken(&state, Path::new(r"C:\Home\x.kdbx"), folder), None, "its own folder");
    }

    #[test]
    fn never_or_no_folder_copies_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("backups");
        fs::create_dir(&folder).unwrap();
        let (never, _) = store_with(dir.path(), Some(backup(&folder, 0, None)));
        run_due(&never, now(), None);
        let (none, _) = store_with(dir.path(), None);
        run_due(&none, now(), None);
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 0);
    }

    #[test]
    fn a_missing_folder_is_reported_and_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("gone");
        let (store, database) = store_with(dir.path(), Some(backup(&folder, 2, None)));
        assert!(run(&store, &database).unwrap_err().contains("not there"));
        let kept = store.read(|s| s.current().unwrap().backup.clone().unwrap());
        assert!(kept.last.is_none());
        assert!(kept.error.unwrap().contains("not there"));
        assert!(!folder.exists());

        // Still due: the next check tries again, and the error goes once it works.
        fs::create_dir(&folder).unwrap();
        run_due(&store, now(), None);
        let kept = store.read(|s| s.current().unwrap().backup.clone().unwrap());
        assert!(kept.last.is_some() && kept.error.is_none());
        assert!(copy_path(&database, &folder).exists());
    }
}
