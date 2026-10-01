//! A database file the app cannot reach by a path (on Android, a document
//! another app or the user's folder holds): the core works on a copy in the
//! app's own storage, as on a file of its own, and this keeps the copy and the
//! document in step. [Mirror::pull] brings in what changed in the document (the
//! core then reads it again and merges, as with a file another program
//! changed); [Mirror::push] writes the copy's changes out, never over a change
//! it has not seen.

use crate::dbfile::hash_hex;
use crate::store::write_atomically;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// When a document was last changed and how long it is: enough to tell that
/// it changed, as Android's documents give no hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    /// Milliseconds since 1970.
    pub modified: i64,
    pub size: u64,
}

/// A document, as the platform reaches it.
pub trait Document {
    fn stamp(&self) -> Result<Stamp, String>;
    fn read(&self) -> Result<Vec<u8>, String>;
    /// Replaces its content.
    fn write(&self, bytes: &[u8]) -> Result<(), String>;
}

/// What the copy and the document were when last in step; kept between runs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InStep {
    pub stamp: Option<Stamp>,
    /// The copy's content then.
    pub hash: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum MirrorError {
    /// The document changed since the last pull: pull (and merge) first.
    Changed,
    Failed(String),
}

pub struct Mirror<D> {
    pub document: D,
    /// Where the document's content goes before each push, if anywhere (its `.bak`).
    pub backup: Option<D>,
    /// The copy the core works on.
    pub copy: PathBuf,
    pub in_step: InStep,
}

impl<D: Document> Mirror<D> {
    /// Brings the document's content into the copy when the document changed
    /// (or there is no copy yet): whether it did.
    pub fn pull(&mut self) -> Result<bool, String> {
        let stamp = self.document.stamp()?;
        if self.in_step.stamp == Some(stamp) && self.copy.exists() {
            return Ok(false);
        }
        let bytes = self.document.read()?;
        write_atomically(&self.copy, &bytes).map_err(|e| format!("Cannot keep the database's copy: {e}"))?;
        // A change made while it was read shows as another stamp at the next pull.
        self.in_step = InStep { stamp: Some(stamp), hash: Some(hash_hex(&bytes)) };
        Ok(true)
    }

    /// Writes the copy into the document when the copy changed since they were
    /// in step: whether it did. The document's content goes to the backup first,
    /// and what was written is read back.
    pub fn push(&mut self) -> Result<bool, MirrorError> {
        let bytes = fs::read(&self.copy).map_err(|e| MirrorError::Failed(format!("Cannot read the database's copy: {e}")))?;
        let hash = hash_hex(&bytes);
        if self.in_step.hash.as_ref() == Some(&hash) {
            return Ok(false);
        }
        if self.in_step.stamp != Some(self.document.stamp().map_err(MirrorError::Failed)?) {
            return Err(MirrorError::Changed);
        }
        if let Some(backup) = &self.backup {
            backup.write(&self.document.read().map_err(MirrorError::Failed)?).map_err(MirrorError::Failed)?;
        }
        self.document.write(&bytes).map_err(MirrorError::Failed)?;
        if self.document.read().map_err(MirrorError::Failed)? != bytes {
            return Err(MirrorError::Failed("The database file did not read back as written".into()));
        }
        self.in_step = InStep { stamp: Some(self.document.stamp().map_err(MirrorError::Failed)?), hash: Some(hash) };
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A document in memory; each write is a new `modified`.
    #[derive(Clone, Default)]
    struct Fake(Rc<RefCell<(Vec<u8>, i64)>>);

    impl Fake {
        fn holding(bytes: &[u8]) -> Self {
            let fake = Fake::default();
            fake.write(bytes).unwrap();
            fake
        }
        fn bytes(&self) -> Vec<u8> {
            self.0.borrow().0.clone()
        }
    }

    impl Document for Fake {
        fn stamp(&self) -> Result<Stamp, String> {
            let (bytes, modified) = &*self.0.borrow();
            Ok(Stamp { modified: *modified, size: bytes.len() as u64 })
        }
        fn read(&self) -> Result<Vec<u8>, String> {
            Ok(self.bytes())
        }
        fn write(&self, bytes: &[u8]) -> Result<(), String> {
            let mut inner = self.0.borrow_mut();
            inner.0 = bytes.to_vec();
            inner.1 += 1;
            Ok(())
        }
    }

    fn mirror(dir: &tempfile::TempDir, document: Fake, backup: Option<Fake>) -> Mirror<Fake> {
        Mirror { document, backup, copy: dir.path().join("db").join("base.kdbx"), in_step: InStep::default() }
    }

    #[test]
    fn the_first_pull_makes_the_copy_and_later_ones_only_what_changed() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = mirror(&dir, Fake::holding(b"one"), None);
        assert_eq!(m.pull(), Ok(true));
        assert_eq!(fs::read(&m.copy).unwrap(), b"one");
        assert_eq!(m.pull(), Ok(false));
        m.document.write(b"two").unwrap();
        assert_eq!(m.pull(), Ok(true));
        assert_eq!(fs::read(&m.copy).unwrap(), b"two");
    }

    #[test]
    fn a_missing_copy_is_pulled_again() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = mirror(&dir, Fake::holding(b"one"), None);
        m.pull().unwrap();
        fs::remove_file(&m.copy).unwrap();
        assert_eq!(m.pull(), Ok(true));
    }

    #[test]
    fn a_push_writes_only_a_changed_copy_and_backs_up_the_document_first() {
        let dir = tempfile::tempdir().unwrap();
        let backup = Fake::default();
        let mut m = mirror(&dir, Fake::holding(b"one"), Some(backup.clone()));
        m.pull().unwrap();
        assert_eq!(m.push(), Ok(false));
        fs::write(&m.copy, b"edited").unwrap();
        assert_eq!(m.push(), Ok(true));
        assert_eq!(m.document.bytes(), b"edited");
        assert_eq!(backup.bytes(), b"one");
        // In step again: nothing more to push or pull.
        assert_eq!(m.push(), Ok(false));
        assert_eq!(m.pull(), Ok(false));
    }

    #[test]
    fn a_push_never_overwrites_a_change_it_has_not_pulled() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = mirror(&dir, Fake::holding(b"one"), None);
        m.pull().unwrap();
        fs::write(&m.copy, b"edited here").unwrap();
        m.document.write(b"edited there").unwrap();
        assert_eq!(m.push(), Err(MirrorError::Changed));
        assert_eq!(m.document.bytes(), b"edited there");
    }

    #[test]
    fn in_step_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let document = Fake::holding(b"one");
        let mut m = mirror(&dir, document.clone(), None);
        m.pull().unwrap();
        let kept = serde_json::to_string(&m.in_step).unwrap();
        let mut again = Mirror { in_step: serde_json::from_str(&kept).unwrap(), ..mirror(&dir, document, None) };
        assert_eq!(again.pull(), Ok(false));
        assert_eq!(again.push(), Ok(false));
    }
}
