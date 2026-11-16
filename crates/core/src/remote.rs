//! Where a synced database lives besides this PC: a store the app reads and
//! writes whole files in, each version marked by a revision.

use crate::dbfile::{hash_hex, hex, replace_file, sibling};
use crate::modules;
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
    /// A document the platform reaches by a reference ([crate::documents]); `name` is for people.
    Document { uri: String, name: String },
}

/// A cloud store one signs in to.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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
    /// This store's id, as a [modules::StorePlugin] names it.
    fn id(self) -> &'static str {
        match self {
            Cloud::Dropbox => DROPBOX,
            Cloud::Google => GOOGLE,
            Cloud::OneDrive => ONEDRIVE,
        }
    }

    pub fn provider(self) -> &'static crate::oauth::Provider {
        plugin(self.id()).provider().expect("a cloud has an OAuth provider")
    }

    /// The databases the app can reach there.
    pub fn list(self) -> Result<Vec<CloudFile>, RemoteError> {
        plugin(self.id()).list()
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
        plugin(self.id()).create(name, bytes)
    }
}

impl Location {
    /// The cloud store this is in; `None` for a folder or a document.
    pub fn cloud(&self) -> Option<Cloud> {
        match self {
            Location::Folder { .. } | Location::Document { .. } => None,
            Location::Dropbox { .. } => Some(Cloud::Dropbox),
            Location::GoogleDrive { .. } => Some(Cloud::Google),
            Location::OneDrive { .. } => Some(Cloud::OneDrive),
        }
    }

    pub fn open(&self) -> Box<dyn Remote> {
        plugin(self.kind()).open(self)
    }

    /// The remote file's name, which the working copy takes.
    pub fn file_name(&self) -> String {
        match self {
            Location::Folder { path } => file_name(path),
            Location::Dropbox { path } => file_name(Path::new(path.rsplit('/').next().unwrap_or_default())),
            Location::GoogleDrive { name, .. } | Location::OneDrive { name, .. } | Location::Document { name, .. } => {
                file_name(Path::new(name))
            }
        }
    }

    /// For the window: what kind of store this is.
    pub fn kind(&self) -> &'static str {
        match self {
            Location::Folder { .. } => "folder",
            Location::Dropbox { .. } => "dropbox",
            Location::GoogleDrive { .. } => "google",
            Location::OneDrive { .. } => "onedrive",
            Location::Document { .. } => "document",
        }
    }

    /// For the window: where the database is synced to.
    pub fn describe(&self) -> String {
        plugin(self.kind()).describe(self)
    }

    /// For status lines: "the folder", "Dropbox".
    pub fn name(&self) -> &'static str {
        plugin(self.kind()).name()
    }
}

/// The store ids, as [Location::kind] and a [modules::StorePlugin]'s id name them.
const FOLDER: &str = "folder";
const DROPBOX: &str = "dropbox";
const GOOGLE: &str = "google";
const ONEDRIVE: &str = "onedrive";
const DOCUMENT: &str = "document";

/// The built-in store plugins, one per [Location] kind.
pub(crate) fn store_plugins() -> &'static [&'static dyn modules::StorePlugin] {
    static PLUGINS: &[&'static dyn modules::StorePlugin] = &[&FolderPlugin, &DropboxPlugin, &GooglePlugin, &OneDrivePlugin, &DocumentPlugin];
    PLUGINS
}

/// The plugin for `id`, which is a [Location] kind or a [Cloud] id; every id
/// has one, as [store_plugins] lists them.
fn plugin(id: &str) -> &'static dyn modules::StorePlugin {
    store_plugins().iter().find(|plugin| plugin.id() == id).copied().expect("a plugin for every store kind")
}

struct FolderPlugin;
struct DropboxPlugin;
struct GooglePlugin;
struct OneDrivePlugin;
struct DocumentPlugin;

impl modules::StorePlugin for FolderPlugin {
    fn id(&self) -> &'static str {
        FOLDER
    }
    fn needs_network(&self) -> bool {
        false
    }
    fn list(&self) -> Result<Vec<CloudFile>, RemoteError> {
        Err(RemoteError::Failed("A folder has no list of databases".into()))
    }
    fn create(&self, _name: &str, _bytes: &[u8]) -> Result<(Location, String), RemoteError> {
        Err(RemoteError::Failed("A file in a folder is not created here".into()))
    }
    fn open(&self, location: &Location) -> Box<dyn Remote> {
        let Location::Folder { path } = location else { unreachable!("the folder plugin opens a folder") };
        Box::new(Folder { path: path.clone() })
    }
    fn provider(&self) -> Option<&'static crate::oauth::Provider> {
        None
    }
    fn describe(&self, location: &Location) -> String {
        let Location::Folder { path } = location else { unreachable!("the folder plugin describes a folder") };
        path.display().to_string()
    }
    fn name(&self) -> &'static str {
        "the folder"
    }
}

impl modules::StorePlugin for DropboxPlugin {
    fn id(&self) -> &'static str {
        DROPBOX
    }
    fn needs_network(&self) -> bool {
        true
    }
    fn list(&self) -> Result<Vec<CloudFile>, RemoteError> {
        Ok(crate::dropbox::list_databases()?.into_iter().map(|path| CloudFile { name: path.trim_start_matches('/').to_string(), id: path }).collect())
    }
    fn create(&self, name: &str, bytes: &[u8]) -> Result<(Location, String), RemoteError> {
        let path = format!("/{name}");
        let revision = crate::dropbox::Dropbox { path: path.clone() }.upload(bytes, None)?;
        Ok((Location::Dropbox { path }, revision))
    }
    fn open(&self, location: &Location) -> Box<dyn Remote> {
        let Location::Dropbox { path } = location else { unreachable!("the Dropbox plugin opens a Dropbox file") };
        Box::new(crate::dropbox::Dropbox { path: path.clone() })
    }
    fn provider(&self) -> Option<&'static crate::oauth::Provider> {
        Some(&crate::dropbox::DROPBOX)
    }
    fn describe(&self, location: &Location) -> String {
        let Location::Dropbox { path } = location else { unreachable!("the Dropbox plugin describes a Dropbox file") };
        format!("Dropbox: {path}")
    }
    fn name(&self) -> &'static str {
        "Dropbox"
    }
}

impl modules::StorePlugin for GooglePlugin {
    fn id(&self) -> &'static str {
        GOOGLE
    }
    fn needs_network(&self) -> bool {
        true
    }
    fn list(&self) -> Result<Vec<CloudFile>, RemoteError> {
        Ok(crate::google::list_databases()?.into_iter().map(|(id, name)| CloudFile { id, name }).collect())
    }
    fn create(&self, name: &str, bytes: &[u8]) -> Result<(Location, String), RemoteError> {
        let (id, revision) = crate::google::create(name, bytes)?;
        Ok((Location::GoogleDrive { id, name: name.to_string() }, revision))
    }
    fn open(&self, location: &Location) -> Box<dyn Remote> {
        let Location::GoogleDrive { id, .. } = location else { unreachable!("the Google Drive plugin opens a Drive file") };
        Box::new(crate::google::GoogleDrive { id: id.clone() })
    }
    fn provider(&self) -> Option<&'static crate::oauth::Provider> {
        Some(&crate::google::GOOGLE)
    }
    fn describe(&self, location: &Location) -> String {
        let Location::GoogleDrive { name, .. } = location else { unreachable!("the Google Drive plugin describes a Drive file") };
        format!("Google Drive: {}/{name}", crate::google::FOLDER)
    }
    fn name(&self) -> &'static str {
        "Google Drive"
    }
}

impl modules::StorePlugin for OneDrivePlugin {
    fn id(&self) -> &'static str {
        ONEDRIVE
    }
    fn needs_network(&self) -> bool {
        true
    }
    fn list(&self) -> Result<Vec<CloudFile>, RemoteError> {
        Ok(crate::onedrive::list_databases()?.into_iter().map(|(id, name)| CloudFile { id, name }).collect())
    }
    fn create(&self, name: &str, bytes: &[u8]) -> Result<(Location, String), RemoteError> {
        let (id, revision) = crate::onedrive::create(name, bytes)?;
        Ok((Location::OneDrive { id, name: name.to_string() }, revision))
    }
    fn open(&self, location: &Location) -> Box<dyn Remote> {
        let Location::OneDrive { id, .. } = location else { unreachable!("the OneDrive plugin opens a OneDrive file") };
        Box::new(crate::onedrive::OneDrive { id: id.clone() })
    }
    fn provider(&self) -> Option<&'static crate::oauth::Provider> {
        Some(&crate::onedrive::ONEDRIVE)
    }
    fn describe(&self, location: &Location) -> String {
        let Location::OneDrive { name, .. } = location else { unreachable!("the OneDrive plugin describes a OneDrive file") };
        format!("OneDrive: {}/{name}", crate::onedrive::FOLDER)
    }
    fn name(&self) -> &'static str {
        "OneDrive"
    }
}

impl modules::StorePlugin for DocumentPlugin {
    fn id(&self) -> &'static str {
        DOCUMENT
    }
    fn needs_network(&self) -> bool {
        false
    }
    fn list(&self) -> Result<Vec<CloudFile>, RemoteError> {
        Err(RemoteError::Failed("A document has no list of databases".into()))
    }
    fn create(&self, _name: &str, _bytes: &[u8]) -> Result<(Location, String), RemoteError> {
        Err(RemoteError::Failed("A document is not created here".into()))
    }
    fn open(&self, location: &Location) -> Box<dyn Remote> {
        let Location::Document { uri, .. } = location else { unreachable!("the document plugin opens a document") };
        Box::new(crate::documents::DocumentFile { uri: uri.clone() })
    }
    fn provider(&self) -> Option<&'static crate::oauth::Provider> {
        None
    }
    fn describe(&self, location: &Location) -> String {
        let Location::Document { name, .. } = location else { unreachable!("the document plugin describes a document") };
        name.clone()
    }
    fn name(&self) -> &'static str {
        "the file"
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

    #[test]
    fn every_location_kind_has_its_plugin() {
        let locations = [
            Location::Folder { path: PathBuf::from("a.kdbx") },
            Location::Dropbox { path: "/a.kdbx".into() },
            Location::GoogleDrive { id: "1".into(), name: "a.kdbx".into() },
            Location::OneDrive { id: "1".into(), name: "a.kdbx".into() },
            Location::Document { uri: "content://a".into(), name: "a.kdbx".into() },
        ];
        for location in locations {
            assert!(store_plugins().iter().any(|plugin| plugin.id() == location.kind()), "{} has a plugin", location.kind());
            assert!(!location.name().is_empty());
            assert!(!location.describe().is_empty());
            let _ = location.open();
        }
    }
}
