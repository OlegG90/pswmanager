//! Files the platform reaches by a reference rather than a path (on Android,
//! a document in a folder or a file the user picked). As a database's remote
//! file ([DocumentFile]) such a document is synced like any other: the core
//! works on its own copy, and the sync decides what goes where. The platform
//! installs its store once at start, as with [crate::secrets].

use crate::dbfile::hash_hex;
use crate::remote::{Remote, RemoteError};
use std::sync::OnceLock;

pub trait DocumentStore: Send + Sync {
    /// The document's content, or `None` when it is gone.
    fn read(&self, uri: &str) -> Result<Option<Vec<u8>>, String>;
    /// Replaces the document's content.
    fn write(&self, uri: &str, bytes: &[u8]) -> Result<(), String>;
}

static STORE: OnceLock<Box<dyn DocumentStore>> = OnceLock::new();

/// Sets how documents are reached: once, at start; only the first call counts.
pub fn install(store: Box<dyn DocumentStore>) {
    let _ = STORE.set(store);
}

fn store() -> Result<&'static dyn DocumentStore, RemoteError> {
    STORE.get().map(|s| s.as_ref()).ok_or_else(|| RemoteError::Failed("This device has no documents".into()))
}

/// A document as a database's remote file. Its revision is the hash of its
/// content: Android's modification times and sizes cannot always tell an edit.
pub struct DocumentFile {
    pub uri: String,
}

impl DocumentFile {
    fn read(&self) -> Result<Option<Vec<u8>>, RemoteError> {
        store()?.read(&self.uri).map_err(RemoteError::Failed)
    }
}

impl Remote for DocumentFile {
    fn revision(&self) -> Result<Option<String>, RemoteError> {
        Ok(self.read()?.map(|bytes| hash_hex(&bytes)))
    }

    fn download(&self) -> Result<(Vec<u8>, String), RemoteError> {
        let bytes = self.read()?.ok_or_else(|| RemoteError::Failed("The database file is gone".into()))?;
        let revision = hash_hex(&bytes);
        Ok((bytes, revision))
    }

    /// Written in place (a document cannot be renamed over) and read back;
    /// written once more when it did not read back as written.
    fn upload(&self, bytes: &[u8], expected: Option<&str>) -> Result<String, RemoteError> {
        if self.revision()?.as_deref() != expected {
            return Err(RemoteError::Changed);
        }
        let store = store()?;
        for _ in 0..2 {
            store.write(&self.uri, bytes).map_err(RemoteError::Failed)?;
            if self.read()?.as_deref() == Some(bytes) {
                return Ok(hash_hex(bytes));
            }
        }
        Err(RemoteError::Failed("The database file did not read back as written".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Documents in memory; a write to `torn:` keeps only half the bytes once.
    #[derive(Default)]
    struct Memory(Mutex<HashMap<String, Vec<u8>>>, Mutex<bool>);

    impl DocumentStore for Memory {
        fn read(&self, uri: &str) -> Result<Option<Vec<u8>>, String> {
            Ok(self.0.lock().unwrap().get(uri).cloned())
        }
        fn write(&self, uri: &str, bytes: &[u8]) -> Result<(), String> {
            let mut torn = self.1.lock().unwrap();
            let kept = if uri.starts_with("torn:") && !*torn { &bytes[..bytes.len() / 2] } else { bytes };
            *torn |= uri.starts_with("torn:");
            self.0.lock().unwrap().insert(uri.to_string(), kept.to_vec());
            Ok(())
        }
    }

    /// The tests share one store (only the first install counts), so each uses its own documents.
    fn file(uri: &str) -> DocumentFile {
        install(Box::new(Memory::default()));
        DocumentFile { uri: uri.into() }
    }

    #[test]
    fn a_document_is_uploaded_only_over_the_revision_expected() {
        let doc = file("mem:one");
        assert_eq!(doc.revision(), Ok(None));
        let first = doc.upload(b"first", None).unwrap();
        assert_eq!(doc.download(), Ok((b"first".to_vec(), first.clone())));
        assert_eq!(doc.upload(b"stale", None), Err(RemoteError::Changed));
        let second = doc.upload(b"second", Some(&first)).unwrap();
        assert_eq!(doc.revision(), Ok(Some(second)));
        assert_eq!(doc.upload(b"stale", Some(&first)), Err(RemoteError::Changed));
    }

    #[test]
    fn a_write_that_does_not_read_back_is_written_again() {
        let doc = file("torn:two");
        let revision = doc.upload(b"the whole database", None).unwrap();
        assert_eq!(doc.download(), Ok((b"the whole database".to_vec(), revision)));
    }

    #[test]
    fn a_missing_document_has_no_revision_and_cannot_be_downloaded() {
        let doc = file("mem:gone");
        assert_eq!(doc.revision(), Ok(None));
        assert!(matches!(doc.download(), Err(RemoteError::Failed(_))));
    }
}
