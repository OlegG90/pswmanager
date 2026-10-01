//! Where a synced database lives besides this PC: a store the app reads and
//! writes whole files in, each version marked by a revision.

use crate::dbfile::{hash_hex, hex, replace_file, sibling};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// Why talking to a store failed.
#[derive(Debug, PartialEq)]
pub enum RemoteError {
    /// The remote file is no longer at the revision the upload expected.
    Changed,
    /// The store cannot be reached now; trying again later may work.
    Offline(String),
    /// The account needs signing in again.
    SignIn(String),
    Failed(String),
}

impl RemoteError {
    pub fn message(&self) -> String {
        match self {
            RemoteError::Changed => "The remote file changed meanwhile".into(),
            RemoteError::Offline(message) | RemoteError::SignIn(message) | RemoteError::Failed(message) => message.clone(),
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
    /// A file in the app's Dropbox folder, by its path there (`/base.kdbx`).
    Dropbox { path: String },
    /// A file in the Drive's PswManager folder, by its id; `name` is for people.
    GoogleDrive { id: String, name: String },
    /// A file in OneDrive's app folder, by its item id; `name` is for people.
    OneDrive { id: String, name: String },
}

/// A cloud store one signs in to.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Cloud {
    Dropbox,
    Google,
    #[serde(rename = "onedrive")]
    OneDrive,
}

/// A database file in a cloud store: its id there (a path for Dropbox) and name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloudFile {
    pub id: String,
    pub name: String,
}

impl Cloud {
    pub fn provider(self) -> &'static crate::oauth::Provider {
        match self {
            Cloud::Dropbox => &crate::dropbox::DROPBOX,
            Cloud::Google => &crate::google::GOOGLE,
            Cloud::OneDrive => &crate::onedrive::ONEDRIVE,
        }
    }

    /// The databases the app can reach there.
    pub fn list(self) -> Result<Vec<CloudFile>, RemoteError> {
        Ok(match self {
            Cloud::Dropbox => crate::dropbox::list_databases()?
                .into_iter()
                .map(|path| CloudFile { name: path.trim_start_matches('/').to_string(), id: path })
                .collect(),
            Cloud::Google => crate::google::list_databases()?.into_iter().map(|(id, name)| CloudFile { id, name }).collect(),
            Cloud::OneDrive => crate::onedrive::list_databases()?.into_iter().map(|(id, name)| CloudFile { id, name }).collect(),
        })
    }

    pub fn location(self, file: CloudFile) -> Location {
        match self {
            Cloud::Dropbox => Location::Dropbox { path: file.id },
            Cloud::Google => Location::GoogleDrive { id: file.id, name: file.name },
            Cloud::OneDrive => Location::OneDrive { id: file.id, name: file.name },
        }
    }

    /// Uploads a new file there and returns where it is and its revision;
    /// one of that name already there is never replaced ([RemoteError::Changed]).
    pub fn create(self, name: &str, bytes: &[u8]) -> Result<(Location, String), RemoteError> {
        match self {
            Cloud::Dropbox => {
                let path = format!("/{name}");
                let revision = crate::dropbox::Dropbox { path: path.clone() }.upload(bytes, None)?;
                Ok((Location::Dropbox { path }, revision))
            }
            Cloud::Google => {
                let (id, revision) = crate::google::create(name, bytes)?;
                Ok((Location::GoogleDrive { id, name: name.to_string() }, revision))
            }
            Cloud::OneDrive => {
                let (id, revision) = crate::onedrive::create(name, bytes)?;
                Ok((Location::OneDrive { id, name: name.to_string() }, revision))
            }
        }
    }
}

impl Location {
    /// The cloud store this is in; `None` for a folder.
    pub fn cloud(&self) -> Option<Cloud> {
        match self {
            Location::Folder { .. } => None,
            Location::Dropbox { .. } => Some(Cloud::Dropbox),
            Location::GoogleDrive { .. } => Some(Cloud::Google),
            Location::OneDrive { .. } => Some(Cloud::OneDrive),
        }
    }

    pub fn open(&self) -> Box<dyn Remote> {
        match self {
            Location::Folder { path } => Box::new(Folder { path: path.clone() }),
            Location::Dropbox { path } => Box::new(crate::dropbox::Dropbox { path: path.clone() }),
            Location::GoogleDrive { id, .. } => Box::new(crate::google::GoogleDrive { id: id.clone() }),
            Location::OneDrive { id, .. } => Box::new(crate::onedrive::OneDrive { id: id.clone() }),
        }
    }

    /// The remote file's name, which the working copy takes.
    pub fn file_name(&self) -> String {
        match self {
            Location::Folder { path } => file_name(path),
            Location::Dropbox { path } => file_name(Path::new(path.rsplit('/').next().unwrap_or_default())),
            Location::GoogleDrive { name, .. } | Location::OneDrive { name, .. } => file_name(Path::new(name)),
        }
    }

    /// For the window: what kind of store this is.
    pub fn kind(&self) -> &'static str {
        match self {
            Location::Folder { .. } => "folder",
            Location::Dropbox { .. } => "dropbox",
            Location::GoogleDrive { .. } => "google",
            Location::OneDrive { .. } => "onedrive",
        }
    }

    /// For the window: where the database is synced to.
    pub fn describe(&self) -> String {
        match self {
            Location::Folder { path } => path.display().to_string(),
            Location::Dropbox { path } => format!("Dropbox: {path}"),
            Location::GoogleDrive { name, .. } => format!("Google Drive: {}/{name}", crate::google::FOLDER),
            Location::OneDrive { name, .. } => format!("OneDrive: {}/{name}", crate::onedrive::FOLDER),
        }
    }

    /// For status lines: "the folder", "Dropbox".
    pub fn name(&self) -> &'static str {
        match self {
            Location::Folder { .. } => "the folder",
            Location::Dropbox { .. } => "Dropbox",
            Location::GoogleDrive { .. } => "Google Drive",
            Location::OneDrive { .. } => "OneDrive",
        }
    }
}

/// A file's name, or `database.kdbx` for a path without one.
pub fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| !n.is_empty()).unwrap_or_else(|| "database.kdbx".into())
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
        let tmp = sibling(&self.path, &format!(".{}.pswm-tmp", hex(&unique)));
        replace_file(&self.path, &tmp, bytes)
            .map_err(|e| RemoteError::Offline(format!("Cannot write {}: {e}", self.path.display())))?;
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
