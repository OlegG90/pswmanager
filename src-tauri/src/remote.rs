//! Where a synced database lives besides this PC: a store the app reads and
//! writes whole files in, each version marked by a revision.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

/// Why talking to a store failed.
#[derive(Debug, PartialEq)]
pub enum RemoteError {
    /// The remote file is no longer at the revision the upload expected.
    Changed,
    /// The store cannot be reached now; trying again later may work.
    Offline(String),
    Failed(String),
}

impl RemoteError {
    pub fn message(&self) -> String {
        match self {
            RemoteError::Changed => "The remote file changed meanwhile".into(),
            RemoteError::Offline(message) | RemoteError::Failed(message) => message.clone(),
        }
    }
}

/// One remote file.
pub trait Remote: Send + Sync {
    /// The file's current revision, or `None` when there is no file.
    fn revision(&self) -> Result<Option<String>, RemoteError>;
    /// The file and its revision.
    fn download(&self) -> Result<(Vec<u8>, String), RemoteError>;
    /// Replaces the file if it is still at `expected` (`None`: there is no
    /// file yet), and returns the new revision; otherwise [RemoteError::Changed].
    fn upload(&self, bytes: &[u8], expected: Option<&str>) -> Result<String, RemoteError>;
}

/// Which store a database is synced with, as the state file keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Location {
    /// A file in a folder: a LAN share, a NAS, another drive.
    Folder { path: PathBuf },
}

impl Location {
    pub fn open(&self) -> Box<dyn Remote> {
        match self {
            Location::Folder { path } => Box::new(Folder { path: path.clone() }),
        }
    }

    /// The remote file's name, which the working copy takes.
    pub fn file_name(&self) -> String {
        match self {
            Location::Folder { path } => path.file_name().map_or("database.kdbx".into(), |n| n.to_string_lossy().into_owned()),
        }
    }

    /// For the window: where the database is synced to.
    pub fn describe(&self) -> String {
        match self {
            Location::Folder { path } => path.display().to_string(),
        }
    }

    /// The file to open directly once syncing stops.
    pub fn local_path(&self) -> PathBuf {
        match self {
            Location::Folder { path } => path.clone(),
        }
    }

    /// For status lines: "the folder", "Dropbox".
    pub fn name(&self) -> &'static str {
        match self {
            Location::Folder { .. } => "the folder",
        }
    }
}

pub fn hash_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// A file in a folder; its revision is the hash of its content.
pub struct Folder {
    pub path: PathBuf,
}

impl Folder {
    fn read(&self) -> Result<Option<Vec<u8>>, RemoteError> {
        match fs::read(&self.path) {
            Ok(bytes) => Ok(Some(bytes)),
            // A missing file in a folder that is there: nothing uploaded yet.
            Err(e) if e.kind() == ErrorKind::NotFound && self.folder().is_dir() => Ok(None),
            Err(e) => Err(RemoteError::Offline(format!("Cannot reach {}: {e}", self.path.display()))),
        }
    }

    fn folder(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new("."))
    }
}

impl Remote for Folder {
    fn revision(&self) -> Result<Option<String>, RemoteError> {
        Ok(self.read()?.map(|bytes| hash_hex(&bytes)))
    }

    fn download(&self) -> Result<(Vec<u8>, String), RemoteError> {
        let bytes = self.read()?.ok_or_else(|| RemoteError::Failed(format!("{} is not there", self.path.display())))?;
        let revision = hash_hex(&bytes);
        Ok((bytes, revision))
    }

    /// Written beside the file and renamed over it, so a reader never sees
    /// half of it. The check and the rename are not one step: another device
    /// writing in between is not caught, which one person's devices rarely do.
    fn upload(&self, bytes: &[u8], expected: Option<&str>) -> Result<String, RemoteError> {
        if self.revision()?.as_deref() != expected {
            return Err(RemoteError::Changed);
        }
        // Named for this upload only: another device may be uploading too.
        let mut unique = [0u8; 8];
        getrandom::fill(&mut unique).map_err(|e| RemoteError::Failed(format!("No random numbers: {e}")))?;
        let tmp = crate::dbfile::sibling(&self.path, &format!(".{}.pswm-tmp", hash_hex(&unique)[..16].to_owned()));
        let written = (|| {
            let mut file = File::create(&tmp)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            fs::rename(&tmp, &self.path)
        })();
        if let Err(e) = written {
            let _ = fs::remove_file(&tmp);
            return Err(RemoteError::Offline(format!("Cannot write {}: {e}", self.path.display())));
        }
        Ok(hash_hex(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_uploads_only_over_the_expected_revision() {
        let dir = tempfile::tempdir().unwrap();
        let folder = Folder { path: dir.path().join("base.kdbx") };
        assert_eq!(folder.revision(), Ok(None));
        let first = folder.upload(b"one", None).unwrap();
        assert_eq!(folder.download().unwrap(), (b"one".to_vec(), first.clone()));
        assert_eq!(folder.upload(b"two", None), Err(RemoteError::Changed));
        let second = folder.upload(b"two", Some(&first)).unwrap();
        assert_eq!(folder.upload(b"three", Some(&first)), Err(RemoteError::Changed));
        assert_eq!(folder.revision(), Ok(Some(second)));
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1, "a temporary file was left behind");
    }

    #[test]
    fn a_folder_that_is_not_there_is_offline() {
        let dir = tempfile::tempdir().unwrap();
        let folder = Folder { path: dir.path().join("gone").join("base.kdbx") };
        assert!(matches!(folder.revision(), Err(RemoteError::Offline(_))));
    }
}
