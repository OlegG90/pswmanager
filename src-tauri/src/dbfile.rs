//! The database file on disk: opened once with its key, and saved so that the
//! file is always either the old version or the complete new one.

use keepass::config::DatabaseVersion;
use keepass::db::{CustomIconRef, EntryRef, GroupRef, Icon};
use keepass::error::{DatabaseKeyError, DatabaseOpenError};
use keepass::{Database, DatabaseKey};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Why a save did not happen.
#[derive(Debug, PartialEq)]
pub enum SaveError {
    /// The file changed since it was last read or written: read it again
    /// ([DbFile::reload]) and redo the change on top.
    Changed,
    Failed(String),
}

impl From<String> for SaveError {
    fn from(message: String) -> Self {
        SaveError::Failed(message)
    }
}

/// Where the database lives, the key to write it with (zeroed on drop), and
/// what the file held when it was last read or written.
pub struct DbFile {
    path: PathBuf,
    key: DatabaseKey,
    hash: [u8; 32],
}

fn hash(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

impl DbFile {
    pub fn open(path: &Path, key: DatabaseKey) -> Result<(Database, DbFile), String> {
        let bytes = fs::read(path).map_err(|e| format!("Cannot open the database: {e}"))?;
        let db = Database::parse(&bytes, key.clone()).map_err(|e| open_error(&e))?;
        Ok((db, DbFile { path: path.to_path_buf(), key, hash: hash(&bytes) }))
    }

    /// The file as it is now, if it changed since it was last read or written
    /// (another device, a sync client); see [Snapshot::read_changed].
    #[cfg(test)]
    pub fn reload(&mut self) -> Result<Option<Database>, String> {
        let since = self.snapshot();
        Ok(since.read_changed()?.and_then(|read| self.adopt(&since, read)))
    }

    /// What reading the file again needs, so the slow part (deriving the key)
    /// can run without holding anything; see [Snapshot::read_changed].
    pub fn snapshot(&self) -> Snapshot {
        Snapshot { path: self.path.clone(), key: self.key.clone(), hash: self.hash }
    }

    /// Takes a file read with [Snapshot::read_changed], unless the file was
    /// read or written again since the snapshot (then that is newer).
    pub fn adopt(&mut self, since: &Snapshot, read: Read) -> Option<Database> {
        (self.hash == since.hash).then(|| {
            self.hash = read.hash;
            read.db
        })
    }

    /// Writes `db` over the file. Refuses when the file changed since it was
    /// last read or written (the caller reloads and retries), never leaves a half-written file, checks
    /// the new file reads back as exactly `db` before it replaces the old one,
    /// and keeps the old one as `<name>.bak`. The file is written as KDBX 4.1,
    /// the only version keepass-rs writes, and `db` says so afterwards; the
    /// cipher and key derivation stay as they were.
    pub fn save(&mut self, db: &mut Database) -> Result<(), SaveError> {
        db.config.version = DatabaseVersion::KDB4(1);
        let db = &*db;
        let unchanged = |current: &[u8]| if hash(current) == self.hash { Ok(()) } else { Err(SaveError::Changed) };
        let read = || fs::read(&self.path).map_err(|e| SaveError::Failed(format!("Cannot read the database file: {e}")));
        unchanged(&read()?)?;
        let mut bytes = Vec::new();
        db.save(&mut bytes, self.key.clone()).map_err(|e| format!("Cannot write the database: {e}"))?;
        let reread = Database::parse(&bytes, self.key.clone())
            .map_err(|e| format!("The new file did not open again ({e}); nothing was saved"))?;
        if !same_content(&reread, db) {
            return Err(SaveError::Failed("The new file did not read back the same; nothing was saved".into()));
        }
        // Writing and checking takes seconds (the key is derived twice): look
        // again, so a change a sync client brought in meanwhile is not lost.
        let current = read()?;
        unchanged(&current)?;

        let tmp = self.sibling(".pswm-tmp");
        let written = (|| {
            let mut file = File::create(&tmp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::write(self.sibling(".bak"), &current)?;
            fs::rename(&tmp, &self.path)
        })();
        if let Err(e) = written {
            let _ = fs::remove_file(&tmp);
            return Err(SaveError::Failed(format!("Cannot save the database: {e}")));
        }
        self.hash = hash(&bytes);
        Ok(())
    }

    /// `base.kdbx` → `base.kdbx<suffix>`, in the same folder (so the rename
    /// stays on one drive).
    fn sibling(&self, suffix: &str) -> PathBuf {
        let mut name = self.path.file_name().unwrap_or_default().to_os_string();
        name.push(suffix);
        self.path.with_file_name(name)
    }
}

/// A database file as it was when the snapshot was taken, with its key.
pub struct Snapshot {
    path: PathBuf,
    key: DatabaseKey,
    hash: [u8; 32],
}

/// A changed file, read and decrypted.
pub struct Read {
    db: Database,
    hash: [u8; 32],
}

impl Snapshot {
    /// The file as it is now, if it changed since the snapshot. A file that
    /// cannot be read — the key changed elsewhere, or a sync client is half-way
    /// through writing it — is an error; saving refuses until it can be read.
    pub fn read_changed(&self) -> Result<Option<Read>, String> {
        let bytes = fs::read(&self.path).map_err(|e| format!("Cannot read the database file: {e}"))?;
        let now = hash(&bytes);
        if now == self.hash {
            return Ok(None);
        }
        let db = Database::parse(&bytes, self.key.clone())
            .map_err(|e| format!("The database file changed on disk and cannot be read now: {}", open_error(&e)))?;
        Ok(Some(Read { db, hash: now }))
    }
}

/// True when two databases hold the same data — everything a KDBX file
/// stores, compared field by field. `Database`'s own equality cannot be used:
/// keepass-rs also keeps in-memory bookkeeping the file does not store (the
/// group each history version was made in, custom-icon back-references per
/// history index), which an edit leaves different from a saved-and-read copy.
fn same_content(a: &Database, b: &Database) -> bool {
    a.config == b.config
        && a.meta == b.meta
        && a.deleted_objects == b.deleted_objects
        && a.num_groups() == b.num_groups()
        && a.num_entries() == b.num_entries()
        && icon_pool(a) == icon_pool(b)
        && a.iter_all_groups().all(|g| b.group(g.id()).is_some_and(|h| same_group(&g, &h)))
        && a.iter_all_entries().all(|e| {
            b.entry(e.id()).is_some_and(|f| e.parent().id() == f.parent().id() && same_version(&e, &f) && same_history(&e, &f))
        })
}

fn icon_pool(db: &Database) -> Vec<Vec<u8>> {
    let mut data: Vec<Vec<u8>> = db.iter_all_custom_icons().map(|i| i.data.clone()).collect();
    data.sort();
    data
}

/// An icon as the file keeps it: a built-in number or custom image data.
fn icon_of(icon: Option<&Icon>, custom: Option<CustomIconRef<'_>>) -> (Option<usize>, Option<Vec<u8>>) {
    let builtin = match icon {
        Some(Icon::BuiltIn(n)) => Some(*n),
        _ => None,
    };
    (builtin, custom.map(|c| c.data.clone()))
}

fn same_group(a: &GroupRef<'_>, b: &GroupRef<'_>) -> bool {
    a.parent().map(|p| p.id()) == b.parent().map(|p| p.id())
        && a.name == b.name
        && a.notes == b.notes
        && a.tags == b.tags
        && a.times == b.times
        && a.custom_data == b.custom_data
        && a.is_expanded == b.is_expanded
        && a.default_autotype_sequence == b.default_autotype_sequence
        && a.enable_autotype == b.enable_autotype
        && a.enable_searching == b.enable_searching
        && icon_of(a.icon(), a.custom_icon()) == icon_of(b.icon(), b.custom_icon())
}

fn same_history(a: &EntryRef<'_>, b: &EntryRef<'_>) -> bool {
    let count = |e: &EntryRef<'_>| e.history.as_ref().map_or(0, |h| h.get_entries().len());
    count(a) == count(b)
        && (0..count(a)).all(|i| match (a.historical(i), b.historical(i)) {
            (Some(x), Some(y)) => same_version(&x, &y),
            _ => false,
        })
}

/// One version of an entry: everything but its place and history.
fn same_version(a: &EntryRef<'_>, b: &EntryRef<'_>) -> bool {
    let attachments = |e: &EntryRef<'_>| {
        let mut all: Vec<(String, Vec<u8>)> = e.attachments_named().map(|(n, att)| (n.to_string(), att.data.get().clone())).collect();
        all.sort();
        all
    };
    a.fields == b.fields
        && a.tags == b.tags
        && a.times == b.times
        && a.custom_data == b.custom_data
        && a.autotype == b.autotype
        && a.foreground_color == b.foreground_color
        && a.background_color == b.background_color
        && a.override_url == b.override_url
        && a.quality_check == b.quality_check
        && icon_of(a.icon(), a.custom_icon()) == icon_of(b.icon(), b.custom_icon())
        && attachments(a) == attachments(b)
}

pub const UNORDERED: &str = "This database stores its elements in an unusual order, which PswManager cannot read \
     safely. Open it in KeePassXC and save it once, or convert it again with the current sic2kdbx.";

fn open_error(e: &DatabaseOpenError) -> String {
    match e {
        DatabaseOpenError::Key(DatabaseKeyError::IncorrectKey) => "Wrong password or key file".into(),
        DatabaseOpenError::Key(DatabaseKeyError::EmptyKey) => "Enter the password or choose a key file".into(),
        DatabaseOpenError::Io(e) => format!("Cannot read the database: {e}"),
        DatabaseOpenError::UnsupportedVersion => "This database version is not supported".into(),
        // keepass-rs reads a strict element order; a file with an entry's
        // fields or a group's children out of order (older sic2kdbx) fails here
        // rather than having its protected values decrypted in the wrong order.
        other if other.to_string().contains("duplicate field") => UNORDERED.into(),
        other => format!("Cannot open the database: {other}"),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn key() -> DatabaseKey {
        DatabaseKey::new().with_password("pw")
    }

    /// Saves `db` to a new file in `dir` and opens it.
    pub fn saved(dir: &Path, db: &Database) -> (Database, DbFile) {
        let path = dir.join("t.kdbx");
        db.save(&mut File::create(&path).unwrap(), key()).unwrap();
        DbFile::open(&path, key()).unwrap()
    }

    #[test]
    fn saves_atomically_and_keeps_the_previous_version() {
        let dir = tempfile::tempdir().unwrap();
        let (mut db, mut file) = saved(dir.path(), &Database::new());
        let before = fs::read(dir.path().join("t.kdbx")).unwrap();
        db.root_mut().add_entry().set_unprotected("Title", "New");
        file.save(&mut db).unwrap();

        let (reopened, _) = DbFile::open(&dir.path().join("t.kdbx"), key()).unwrap();
        assert_eq!(reopened.num_entries(), 1);
        assert_eq!(fs::read(dir.path().join("t.kdbx.bak")).unwrap(), before);
        assert!(!dir.path().join("t.kdbx.pswm-tmp").exists());
        // The next save starts from the file this one wrote.
        file.save(&mut db).unwrap();
    }

    #[test]
    fn an_edited_entry_with_its_own_icon_saves() {
        let dir = tempfile::tempdir().unwrap();
        let mut start = Database::new();
        start.root_mut().add_entry().set_icon_custom_new(vec![0x89, b'P', b'N', b'G', 0, 0]);
        let (mut db, mut file) = saved(dir.path(), &start);
        let id = db.iter_all_entries().next().unwrap().id();
        db.entry_mut(id).unwrap().track_changes().set_unprotected("Title", "edited");
        file.save(&mut db).unwrap();
        let (reopened, _) = DbFile::open(&dir.path().join("t.kdbx"), key()).unwrap();
        assert!(same_content(&reopened, &db));
    }

    #[test]
    fn reload_reads_a_file_changed_elsewhere_once() {
        let dir = tempfile::tempdir().unwrap();
        let (_, mut file) = saved(dir.path(), &Database::new());
        assert!(file.reload().unwrap().is_none());
        let mut other = Database::new();
        other.root_mut().add_entry();
        other.save(&mut File::create(dir.path().join("t.kdbx")).unwrap(), key()).unwrap();
        assert_eq!(file.reload().unwrap().unwrap().num_entries(), 1);
        assert!(file.reload().unwrap().is_none());
    }

    #[test]
    fn an_unreadable_file_is_reported_and_blocks_saving() {
        let dir = tempfile::tempdir().unwrap();
        let (mut db, mut file) = saved(dir.path(), &Database::new());
        fs::write(dir.path().join("t.kdbx"), b"half-synced").unwrap();
        assert!(file.reload().unwrap_err().contains("cannot be read now"));
        assert_eq!(file.save(&mut db), Err(SaveError::Changed));
        assert_eq!(fs::read(dir.path().join("t.kdbx")).unwrap(), b"half-synced");
    }

    #[test]
    fn refuses_to_overwrite_a_file_changed_elsewhere() {
        let dir = tempfile::tempdir().unwrap();
        let (mut db, mut file) = saved(dir.path(), &Database::new());
        let mut other = Database::new();
        other.root_mut().add_entry();
        other.save(&mut File::create(dir.path().join("t.kdbx")).unwrap(), key()).unwrap();
        let theirs = fs::read(dir.path().join("t.kdbx")).unwrap();

        assert_eq!(file.save(&mut db), Err(SaveError::Changed));
        assert_eq!(fs::read(dir.path().join("t.kdbx")).unwrap(), theirs);
    }

    #[test]
    fn reports_a_wrong_password() {
        let dir = tempfile::tempdir().unwrap();
        saved(dir.path(), &Database::new());
        let wrong = DbFile::open(&dir.path().join("t.kdbx"), DatabaseKey::new().with_password("nope"));
        assert_eq!(wrong.err().unwrap(), "Wrong password or key file");
    }
}
