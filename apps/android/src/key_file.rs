//! Key files chosen or made through a folder (#207). Android's file picker
//! lists nothing in some folders (for any app), while its folder picker
//! offers them: so the user picks the folder, and the app lists its files and
//! makes a new one in it. The app gets that one folder; the key file stays
//! where it is. What is kept is the document inside the folder, read at each
//! unlock, as a key file picked the old way (a document) still is.

use crate::app::{self, off_main, Status};
use crate::documents::{Documents, Picked};
use pswm_core::documents::DocumentStore;
use pswm_core::session::Session;
use pswm_core::store::Store;
use pswm_core::vault;
use serde::Serialize;
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
        let name = name.trim().to_string();
        if name.is_empty() || name.contains('/') {
            return Err("Give the key file a name".into());
        }
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
