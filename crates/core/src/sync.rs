//! Keeping the working copy and the remote file in step. At each sync the app
//! compares both with where the last sync left them: an unchanged side takes
//! the other's file, and when both changed they are merged.

use crate::dbfile::{hash_hex, same_file, sibling, OpenError, BAK, REMOTE_BAK};
use crate::remote::{Remote, RemoteError};
use crate::session::Session;
use crate::store::{self, Store};
use std::fs;
use std::path::{Path, PathBuf};

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
    /// The remote file opens with none of the keys known: another device
    /// changed the key. The user is asked for it once.
    OtherKey,
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
            // A remote file no key here opened is not downloaded again until
            // it changes or a key is added.
            if revision.as_deref().is_some_and(|r| since.known_unopened(r)) {
                return Err(SyncError::OtherKey);
            }
            let changed_here = unsaved || Some(working_hash(&working)?) != state.synced;
            let (bytes, revision) = remote.download()?;
            // Deriving the key takes a while: done without holding the database.
            let (theirs, key_change) = match since.parse(&bytes) {
                Ok(read) => read,
                Err(OpenError::OtherKey(_)) => {
                    since.note_unopened(&revision);
                    return Err(SyncError::OtherKey);
                }
                Err(OpenError::Other(message)) => return Err(failed(format!("The remote copy cannot be opened: {message}"))),
            };
            outcome = match session.with_mut(|v| v.take_remote(&since, theirs, key_change, &bytes, changed_here)) {
                Ok(Some(taken)) => taken,
                Ok(None) => return Ok(None),
                Err(_) if !session.is_unlocked() => return Ok(Some(Outcome::WaitingForUnlock)),
                Err(message) => return Err(failed(message)),
            };
            let merged = matches!(outcome, Outcome::Merged(_));
            if merged {
                // What the upload below replaces, in case the merge got it wrong.
                fs::write(sibling(&working, REMOTE_BAK), &bytes).map_err(|e| failed(format!("Cannot keep the remote copy: {e}")))?;
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
    if store.read(|s| s.lists(&local, None)) {
        return Err(format!("{} is already in the list: choose another place", local.display()));
    }
    if matches!(&location, crate::remote::Location::Folder { path } if same_file(path, &local)) {
        return Err("The copy on this PC must be another file than the one in the folder".into());
    }
    if local.exists() {
        fs::copy(&local, sibling(&local, BAK)).map_err(|e| format!("Cannot keep the file that was there: {e}"))?;
    }
    let remote = download_into(&local, location)?;
    store
        .update(|s| {
            s.select(local).remote = Some(remote);
        })
        .map_err(|e| format!("Cannot save the choice: {e}"))
}

/// Starts syncing the database whose file is `file` with `location`, a
/// remote file just made from `uploaded` (the file's bytes) at `revision`:
/// nothing is downloaded, and a change to the file since counts as pending.
pub fn attach(store: &Store, file: &Path, location: crate::remote::Location, revision: String, uploaded: &[u8]) -> Result<(), String> {
    let remote = store::Remote { location, revision: Some(revision), synced: Some(hash_hex(uploaded)) };
    store
        .update(|s| {
            if let Some(known) = s.databases.iter_mut().find(|d| d.file == file) {
                known.remote = Some(remote);
            }
        })
        .map_err(|e| format!("Cannot save the sync state: {e}"))
}

/// What to do when a database is linked to a remote file that differs.
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LinkChoice {
    /// The usual merge, then upload (needs the database unlocked).
    Merge,
    /// The remote file replaces the local one (kept as `.bak`; needs the
    /// database unlocked).
    UseRemote,
    /// The local file replaces the remote one (kept as `.remote.bak`).
    KeepLocal,
}

/// Links the database whose file is `file` to the existing remote file at
/// `location`. The same content links at once; a different one needs a
/// `choice`, and without one nothing changes and `Ok(false)` says they
/// differ. The choice is carried out by the next sync, which it sets up:
/// each is one row of the decision table.
pub fn link(
    store: &Store,
    file: &Path,
    location: crate::remote::Location,
    choice: Option<LinkChoice>,
    key: Option<&crate::dbfile::Snapshot>,
) -> Result<bool, String> {
    let local = working_hash(file).map_err(|_| format!("Cannot read {}", file.display()))?;
    let (bytes, revision) = location.open().download().map_err(|e| e.message())?;
    // With the database unlocked, the remote file must open with its key:
    // otherwise every sync would fail, or Keep this file would replace a
    // database with other credentials.
    if let Some(key) = key {
        key.parse(&bytes).map_err(|e| format!("The remote file does not open with this database's key ({e})"))?;
    }
    let (revision, synced) = if hash_hex(&bytes) == local {
        (Some(revision), Some(local))
    } else {
        match choice {
            None => return Ok(false),
            // Both count as changed: the sync merges them.
            Some(LinkChoice::Merge) => (None, None),
            // Only the remote file counts as changed: the sync takes it.
            Some(LinkChoice::UseRemote) => (None, Some(local)),
            // Only the local file counts as changed: the sync uploads it.
            Some(LinkChoice::KeepLocal) => {
                fs::write(sibling(file, REMOTE_BAK), &bytes).map_err(|e| format!("Cannot keep the remote copy: {e}"))?;
                (Some(revision), None)
            }
        }
    };
    store
        .update(|s| {
            if let Some(known) = s.databases.iter_mut().find(|d| d.file == file) {
                known.remote = Some(store::Remote { location, revision, synced });
            }
        })
        .map_err(|e| format!("Cannot save the sync state: {e}"))?;
    Ok(true)
}

/// Downloads the remote file into `working`; the sync state that goes with it.
fn download_into(working: &Path, location: crate::remote::Location) -> Result<store::Remote, String> {
    let (bytes, revision) = location.open().download().map_err(|e| e.message())?;
    store::write_atomically(working, &bytes).map_err(|e| format!("Cannot write the working copy: {e}"))?;
    Ok(store::Remote { location, revision: Some(revision), synced: Some(hash_hex(&bytes)) })
}

/// `<dir>/<name>`, or `<dir>/<stem> (2).kdbx` and so on: a path no file has
/// and no database in the list uses. (Only for working copies an older
/// version kept in `sync/`, when they stop syncing.)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
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
            self.elsewhere_keyed(key(), key(), change);
        }

        /// Another device opens the remote file with `open`, changes it and
        /// writes it with `save`.
        fn elsewhere_keyed(&self, open: DatabaseKey, save: DatabaseKey, change: impl FnOnce(&mut Database)) {
            let mut db = Database::open(&mut File::open(&self.remote.path).unwrap(), open).unwrap();
            change(&mut db);
            db.config.version = keepass::config::DatabaseVersion::KDB4(1);
            db.save(&mut File::create(&self.remote.path).unwrap(), save).unwrap();
        }

        fn edit_here(&self, title: &str, password: &str) {
            self.session
                .with_mut(|v| {
                    let id = v.listing().entries.into_iter().find(|e| e.title == title).unwrap().id;
                    let mut data = v.edit_data(&id).unwrap();
                    data.password = password.into();
                    v.save_entry(Some(&id), None, &data, false).map(|_| ())
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
    fn a_setting_changed_here_is_merged_with_a_change_there() {
        let s = setup();
        s.session.with_mut(|v| v.set_setting(crate::edit::Setting::Name, "Named here")).unwrap();
        s.elsewhere(|db| db.root_mut().add_entry().set_unprotected(fields::TITLE, "Added on the phone"));
        // No entry of this device's is newer: the setting alone makes it a merge.
        assert!(matches!(s.sync(), Ok(Outcome::Merged(_))));
        let remote = Vault::open(&s.remote.path, Some("test"), None).unwrap();
        assert_eq!(remote.settings().name, "Named here");
        assert!(s.remote_titles().contains(&"Added on the phone".to_string()));
    }

    #[test]
    fn encryption_changed_here_is_kept_in_a_merge() {
        use crate::encryption::{Cipher, Encryption, Kdf};
        let s = setup();
        let wanted = Encryption { cipher: Cipher::ChaCha20, kdf: Kdf::Argon2id, iterations: 2, memory: 8 << 20, parallelism: 1 };
        s.session.with_mut(|v| v.set_encryption(&wanted)).unwrap();
        s.elsewhere(|db| db.root_mut().add_entry().set_unprotected(fields::TITLE, "Added on the phone"));
        // Nothing else changed here: the encryption alone makes it a merge.
        assert!(matches!(s.sync(), Ok(Outcome::Merged(_))));
        let remote = Vault::open(&s.remote.path, Some("test"), None).unwrap();
        assert_eq!(remote.settings().encryption, wanted);
        assert!(s.remote_titles().contains(&"Added on the phone".to_string()));
    }

    #[test]
    fn a_remote_file_still_on_the_old_key_is_merged_and_goes_up_with_the_new_one() {
        let s = setup();
        s.session.with_mut(|v| v.change_key((Some("test"), None), Some("new"), None)).unwrap();
        // The phone, still on the old key, changes the remote file meanwhile.
        s.elsewhere(|db| db.root_mut().add_entry().set_unprotected(fields::TITLE, "Added on the phone"));
        assert!(matches!(s.sync(), Ok(Outcome::Merged(_) | Outcome::Downloaded(_))));
        assert_eq!(s.sync(), Ok(Outcome::UpToDate));
        let remote = Vault::open(&s.remote.path, Some("new"), None).unwrap();
        assert!(remote.listing().entries.iter().any(|e| e.title == "Added on the phone"));
        assert!(Vault::open(&s.remote.path, Some("test"), None).is_err());
    }

    #[test]
    fn an_old_key_copy_coming_back_after_the_upload_is_merged_not_taken() {
        let s = setup();
        let old_copy = fs::read(&s.remote.path).unwrap();
        s.edit_here("Mail", "from the PC");
        s.session.with_mut(|v| v.change_key((Some("test"), None), Some("new"), None)).unwrap();
        s.sync().unwrap(); // the new key goes up
        // The phone, on the old key and without this device's edit, writes its copy back.
        fs::write(&s.remote.path, &old_copy).unwrap();
        s.elsewhere(|db| db.root_mut().add_entry().set_unprotected(fields::TITLE, "Added on the phone"));

        assert!(matches!(s.sync(), Ok(Outcome::Merged(_))));
        let remote = Vault::open(&s.remote.path, Some("new"), None).unwrap();
        assert!(remote.listing().entries.iter().any(|e| e.title == "Added on the phone"));
        let mail = remote.listing().entries.into_iter().find(|e| e.title == "Mail").unwrap().id;
        assert_eq!(remote.field(&mail, fields::PASSWORD).unwrap().as_str(), "from the PC");
        let ours = s.session.read(|v| v.settings().encryption).unwrap();
        assert_eq!(remote.settings().encryption, ours);
        let working = s.store.read(|st| st.current.clone()).unwrap();
        assert!(sibling(&working, crate::dbfile::REMOTE_BAK).exists(), "the remote file is kept before it is replaced");
    }

    #[test]
    fn a_key_changed_later_on_another_device_is_asked_for_and_taken() {
        let s = setup();
        s.elsewhere_keyed(key(), DatabaseKey::new().with_password("phone"), |db| {
            db.meta.master_key_changed = Some(keepass::db::Times::now());
            db.root_mut().add_entry().set_unprotected(fields::TITLE, "Added on the phone");
        });
        assert_eq!(s.sync(), Err(SyncError::OtherKey));

        s.session.with_mut(|v| v.remember_key(DatabaseKey::new().with_password("phone"))).unwrap();
        assert!(s.sync().is_ok());
        // This device takes the newer key: its file opens with it from now on.
        assert!(s.session.read(|v| v.uses_key(&DatabaseKey::new().with_password("phone"))).unwrap().unwrap());
        let working = s.store.read(|st| st.current.clone()).unwrap();
        assert!(Vault::open(&working, Some("phone"), None).is_ok());
        assert!(Vault::open(&working, Some("test"), None).is_err());
        assert!(!s.password_here("Mail").is_empty(), "the database reads as before");
    }

    #[test]
    fn a_remote_file_no_key_opens_is_downloaded_again_only_when_it_changes_or_a_key_is_added() {
        /// Counts the downloads.
        struct Counting<'a>(&'a Folder, Mutex<usize>);
        impl Remote for Counting<'_> {
            fn revision(&self) -> Result<Option<String>, RemoteError> {
                self.0.revision()
            }
            fn download(&self) -> Result<(Vec<u8>, String), RemoteError> {
                *self.1.lock().unwrap() += 1;
                self.0.download()
            }
            fn upload(&self, bytes: &[u8], expected: Option<&str>) -> Result<String, RemoteError> {
                self.0.upload(bytes, expected)
            }
        }
        let s = setup();
        let counting = Counting(&s.remote, Mutex::new(0));
        let downloads = || *counting.1.lock().unwrap();
        let phone = DatabaseKey::new().with_password("phone");
        s.elsewhere_keyed(key(), phone.clone(), |db| db.meta.master_key_changed = Some(keepass::db::Times::now()));

        assert_eq!(sync(&counting, &s.store, &s.session), Err(SyncError::OtherKey));
        assert_eq!(sync(&counting, &s.store, &s.session), Err(SyncError::OtherKey));
        assert_eq!(downloads(), 1, "the same revision is not downloaded again");

        // Another upload on that key: a new revision, downloaded once.
        s.elsewhere_keyed(phone.clone(), phone.clone(), |db| db.root_mut().add_entry().set_unprotected(fields::TITLE, "Added on the phone"));
        assert_eq!(sync(&counting, &s.store, &s.session), Err(SyncError::OtherKey));
        assert_eq!(sync(&counting, &s.store, &s.session), Err(SyncError::OtherKey));
        assert_eq!(downloads(), 2);

        // A key added: downloaded and read with it.
        s.session.with_mut(|v| v.remember_key(phone.clone())).unwrap();
        assert!(sync(&counting, &s.store, &s.session).is_ok());
        assert_eq!(downloads(), 3);
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

    /// A second local database with the fixture's content, not synced, current.
    fn second_database(s: &Setup) -> PathBuf {
        let path = s.store.dir().join("second.kdbx");
        fs::copy(&s.remote.path, &path).unwrap();
        s.store.update(|st| {
            st.select(path.clone());
        })
        .unwrap();
        s.session.set(Some(Vault::open(&path, Some("test"), None).unwrap()));
        path
    }

    #[test]
    fn linking_to_the_same_content_links_at_once() {
        let s = setup();
        let second = second_database(&s);
        assert_eq!(link(&s.store, &second, Location::Folder { path: s.remote.path.clone() }, None, None), Ok(true));
        assert_eq!(s.sync(), Ok(Outcome::UpToDate));
    }

    #[test]
    fn linking_to_a_different_file_asks_and_each_choice_syncs_its_way() {
        for (choice, expected) in [
            (LinkChoice::Merge, "both"),
            (LinkChoice::UseRemote, "remote"),
            (LinkChoice::KeepLocal, "local"),
        ] {
            let s = setup();
            let second = second_database(&s);
            s.edit_here("Mail", "here");
            s.elsewhere(|db| db.root_mut().add_entry().set_unprotected(fields::TITLE, "Added there"));
            let folder = Location::Folder { path: s.remote.path.clone() };
            assert_eq!(link(&s.store, &second, folder.clone(), None, None), Ok(false));
            assert!(s.store.read(|st| st.remote().is_none()), "nothing linked without a choice");
            assert_eq!(link(&s.store, &second, folder, Some(choice), None), Ok(true));
            s.sync().unwrap();
            let titles = s.remote_titles();
            let (added, mine) = (titles.contains(&"Added there".to_string()), s.remote_password("Mail") == "here");
            match expected {
                "both" => assert!(added && mine, "{choice:?}"),
                "remote" => assert!(added && !mine && s.password_here("Mail") != "here", "{choice:?}"),
                _ => assert!(!added && mine && sibling(&second, ".remote.bak").exists(), "{choice:?}"),
            }
        }
    }

    #[test]
    fn a_remote_file_with_another_key_is_not_linked() {
        let s = setup();
        let second = second_database(&s);
        let other = s.remote.path.with_file_name("other-key.kdbx");
        let mut db = Database::open(&mut File::open(&s.remote.path).unwrap(), key()).unwrap();
        db.config.version = keepass::config::DatabaseVersion::KDB4(1);
        db.save(&mut File::create(&other).unwrap(), DatabaseKey::new().with_password("another")).unwrap();
        let snapshot = s.session.read(Vault::snapshot).unwrap().unwrap();
        let refused = link(&s.store, &second, Location::Folder { path: other }, Some(LinkChoice::KeepLocal), Some(&snapshot));
        assert!(refused.unwrap_err().contains("does not open"));
        assert!(s.store.read(|st| st.remote().is_none()));
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
