//! The database file on disk: opened once with its key, and saved so that the
//! file is always either the old version or the complete new one.

use keepass::error::{DatabaseKeyError, DatabaseOpenError};
use keepass::{Database, DatabaseKey};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const CHANGED_ON_DISK: &str =
    "The database file was changed by another program or device since it was opened. Lock and unlock to load it; \
     your change was not saved.";

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

    /// Writes `db` over the file. Refuses when the file changed since it was
    /// read (merging comes later), never leaves a half-written file, checks
    /// the new file reads back as exactly `db` before it replaces the old one,
    /// and keeps the old one as `<name>.bak`.
    pub fn save(&mut self, db: &Database) -> Result<(), String> {
        let unchanged = |current: &[u8]| {
            if hash(current) == self.hash {
                Ok(())
            } else {
                Err(CHANGED_ON_DISK.to_string())
            }
        };
        let read = || fs::read(&self.path).map_err(|e| format!("Cannot read the database file: {e}"));
        unchanged(&read()?)?;
        let mut bytes = Vec::new();
        db.save(&mut bytes, self.key.clone()).map_err(|e| format!("Cannot write the database: {e}"))?;
        let reread = Database::parse(&bytes, self.key.clone())
            .map_err(|e| format!("The new file did not open again ({e}); nothing was saved"))?;
        if reread != *db {
            return Err("The new file did not read back the same; nothing was saved".into());
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
            return Err(format!("Cannot save the database: {e}"));
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

pub const UNORDERED: &str = "This database stores its elements in an unusual order, which PswManager cannot read      safely. Open it in KeePassXC and save it once, or convert it again with the current sic2kdbx.";

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
        file.save(&db).unwrap();

        let (reopened, _) = DbFile::open(&dir.path().join("t.kdbx"), key()).unwrap();
        assert_eq!(reopened.num_entries(), 1);
        assert_eq!(fs::read(dir.path().join("t.kdbx.bak")).unwrap(), before);
        assert!(!dir.path().join("t.kdbx.pswm-tmp").exists());
        // The next save starts from the file this one wrote.
        file.save(&db).unwrap();
    }

    #[test]
    fn refuses_to_overwrite_a_file_changed_elsewhere() {
        let dir = tempfile::tempdir().unwrap();
        let (db, mut file) = saved(dir.path(), &Database::new());
        let mut other = Database::new();
        other.root_mut().add_entry();
        other.save(&mut File::create(dir.path().join("t.kdbx")).unwrap(), key()).unwrap();
        let theirs = fs::read(dir.path().join("t.kdbx")).unwrap();

        assert_eq!(file.save(&db).unwrap_err(), CHANGED_ON_DISK);
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
