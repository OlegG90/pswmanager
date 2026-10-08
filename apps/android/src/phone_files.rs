//! Files and folders on the phone (#207). Android's pickers hide some folders
//! from every app (one named Data is refused). So, with All files access
//! turned on, the phone's folders are browsed here, for every file the app
//! opens or writes: a key file (chosen, or made), a database, a folder for a
//! new database or the visible copy, a file to attach, an attachment saved.
//! What is kept is then the path, as a `file://` URI ([crate::documents::local]),
//! which the document layer reads and writes as it does a picked document.
//! Without the access, Android's pickers are used as before; a key file can
//! also be chosen in a folder picked with Android's folder picker.

use crate::app::{self, off_main, Status};
use crate::documents::{path_uri, Documents, Picked};
use pswm_core::documents::DocumentStore;
use pswm_core::session::Session;
use pswm_core::store::Store;
use pswm_core::vault;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, Wry};

/// A folder picked for a key file, and the files in it.
#[derive(Serialize)]
pub struct KeyFolder {
    folder: Picked,
    files: Vec<String>,
}

/// A folder for a key file, picked with Android's folder picker (its access
/// is kept), with its files; `None` when cancelled.
#[tauri::command]
pub async fn pick_key_folder(app: AppHandle) -> Result<Option<KeyFolder>, String> {
    off_main(move || {
        let documents = app.state::<Documents<Wry>>();
        let Some(folder) = documents.pick_folder()? else { return Ok(None) };
        let mut files = documents.files(&folder.uri)?;
        // Key files first, then the rest by name.
        files.sort_by_key(|name| (!is_key_file(name), name.to_lowercase()));
        Ok(Some(KeyFolder { folder, files }))
    })
    .await
}

fn is_key_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".keyx") || lower.ends_with(".key")
}

/// The key file `name` in a picked folder.
#[tauri::command]
pub async fn key_file_in(app: AppHandle, folder: String, name: String) -> Result<Picked, String> {
    off_main(move || {
        let uri = app.state::<Documents<Wry>>().find(&folder, &name)?.ok_or(format!("{name} is not in the folder any more"))?;
        Ok(Picked { uri, name })
    })
    .await
}

/// Makes a new key file named `name` in a picked folder, never over a file
/// already there.
#[tauri::command]
pub async fn create_key_file_in(app: AppHandle, folder: String, name: String) -> Result<Picked, String> {
    off_main(move || {
        let name = file_name(&name)?;
        let documents = app.state::<Documents<Wry>>();
        if documents.find(&folder, &name)?.is_some() {
            return Err(format!("{name} is already there: choose another name"));
        }
        let uri = documents.child(&folder, &name)?;
        documents.write(&uri, vault::new_key_file()?.as_bytes())?;
        Ok(Picked { uri, name })
    })
    .await
}

/// The database is unlocked with `key_file` from now on (the unlock screen's
/// *Use a key file…*).
#[tauri::command]
pub fn use_key_file(app: AppHandle, key_file: Picked) -> Result<Status, String> {
    let store = app.state::<Store>();
    app::remember_key_file(&store, &key_file)?;
    Ok(app::status_of(&store, &app.state::<Session>()))
}

/// Where browsing the phone's files starts, and what it never leaves.
const STORAGE: &str = "/storage/emulated/0";

/// Whether the app may browse the phone's files (All files access).
#[tauri::command]
pub async fn all_files_access(app: AppHandle) -> Result<bool, String> {
    off_main(move || app.state::<Documents<Wry>>().all_files_access()).await
}

/// Opens Android's page that turns All files access on for this app.
#[tauri::command]
pub async fn ask_all_files_access(app: AppHandle) -> Result<(), String> {
    off_main(move || app.state::<Documents<Wry>>().ask_all_files_access()).await
}

/// A folder of the phone's storage, browsed for a key file.
#[derive(Serialize)]
pub struct Browsed {
    path: String,
    /// The folder above, none at the storage's top.
    up: Option<String>,
    folders: Vec<String>,
    files: Vec<String>,
}

/// `path` (none: the storage's top) as a folder within the phone's storage.
fn within_storage(path: Option<&str>) -> Result<PathBuf, String> {
    let storage = Path::new(STORAGE).canonicalize().map_err(|e| format!("Cannot open the phone's storage: {e}"))?;
    let path = path.map_or_else(|| storage.clone(), PathBuf::from);
    let canonical = path.canonicalize().map_err(|e| format!("Cannot open {}: {e}", path.display()))?;
    if !canonical.starts_with(&storage) {
        return Err("Only the phone's own storage is browsed".into());
    }
    Ok(canonical)
}

/// A file's name as given for a key file: trimmed, and a name, not a path.
fn file_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." || name.contains('/') {
        return Err("Give the key file a name".into());
    }
    Ok(name.to_string())
}


/// The folders and files in `path` (none: the storage's top), with All files
/// access on; hidden ones (a leading dot) are left out.
#[tauri::command]
pub async fn browse(path: Option<String>) -> Result<Browsed, String> {
    off_main(move || {
        let folder = within_storage(path.as_deref())?;
        let (mut folders, mut files) = (Vec::new(), Vec::new());
        for entry in std::fs::read_dir(&folder).map_err(|e| format!("Cannot read {}: {e}", folder.display()))?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => folders.push(name),
                Ok(kind) if kind.is_file() => files.push(name),
                _ => {}
            }
        }
        folders.sort_by_key(|n| n.to_lowercase());
        files.sort_by_key(|name| (!is_key_file(name), name.to_lowercase()));
        let top = Path::new(STORAGE).canonicalize().map_err(|e| e.to_string())?;
        let up = (folder != top).then(|| folder.parent()).flatten().map(|p| p.display().to_string());
        Ok(Browsed { path: folder.display().to_string(), up, folders, files })
    })
    .await
}

/// The key file `name` in a browsed folder, kept by its path.
#[tauri::command]
pub async fn key_file_at(folder: String, name: String) -> Result<Picked, String> {
    off_main(move || {
        let name = file_name(&name)?;
        let path = within_storage(Some(&folder))?.join(&name);
        if !path.is_file() {
            return Err(format!("{name} is not in the folder any more"));
        }
        Ok(Picked { uri: path_uri(&path), name })
    })
    .await
}

/// The database `name` in a browsed folder becomes the database on this
/// phone, synced with that file by its path (as a folder on Windows is).
#[tauri::command]
pub async fn open_database_at(app: AppHandle, folder: String, name: String) -> Result<Status, String> {
    off_main(move || {
        let name = file_name(&name)?;
        let path = within_storage(Some(&folder))?.join(&name);
        if !path.is_file() {
            return Err(format!("{name} is not in the folder any more"));
        }
        app::adopt(&app, pswm_core::remote::Location::Folder { path })
    })
    .await
}

/// Makes a new key file named `name` in a browsed folder, never over a file
/// already there.
#[tauri::command]
pub async fn create_key_file_at(folder: String, name: String) -> Result<Picked, String> {
    off_main(move || {
        let name = file_name(&name)?;
        let path = within_storage(Some(&folder))?.join(&name);
        if path.exists() {
            return Err(format!("{name} is already there: choose another name"));
        }
        vault::create_key_file(&path)?;
        Ok(Picked { uri: path_uri(&path), name })
    })
    .await
}

/// The file `name` in a browsed folder, staged to attach (up to 20 MB).
#[tauri::command]
pub async fn attach_file_at(app: AppHandle, folder: String, name: String) -> Result<pswm_core::staged::StagedFile, String> {
    off_main(move || {
        let name = file_name(&name)?;
        let path = within_storage(Some(&folder))?.join(&name);
        let size = std::fs::metadata(&path).map_err(|e| format!("Cannot read {name}: {e}"))?.len();
        pswm_core::edit::check_size(size)?;
        let content = std::fs::read(&path).map_err(|e| format!("Cannot read {name}: {e}"))?;
        app.state::<Session>().staged().add(name, zeroize::Zeroizing::new(content))
    })
    .await
}

/// Saves an attachment (of an older version with `version`) as `file_name` in
/// a browsed folder, never over a file already there.
#[tauri::command]
pub async fn save_attachment_at(app: AppHandle, id: String, name: String, version: Option<usize>, folder: String, file_name: String) -> Result<(), String> {
    let data = app.state::<Session>().with(|v| v.attachment(&id, version, &name))?;
    off_main(move || {
        let file_name = self::file_name(&file_name)?;
        let path = within_storage(Some(&folder))?.join(&file_name);
        if path.exists() {
            return Err(format!("{file_name} is already there: choose another name"));
        }
        std::fs::write(&path, &*data).map_err(|e| format!("Cannot save {file_name}: {e}"))
    })
    .await
}

/// A browsed folder, as the folders Android's folder picker gives are kept
/// (for a new database, the visible copy).
#[tauri::command]
pub async fn folder_at(path: String) -> Result<Picked, String> {
    off_main(move || {
        let folder = within_storage(Some(&path))?;
        let name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Phone".into());
        Ok(Picked { uri: path_uri(&folder), name })
    })
    .await
}
