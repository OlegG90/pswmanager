//! A new database made on the phone (#146): the core makes it as on Windows,
//! with a master password, and it lives where the user chooses when it is
//! made: in a cloud store, uploaded to the app's folder there (with a visible
//! copy on the phone), or as a local file in a folder on the phone. It is
//! never put over a file already there.

use crate::app::{off_main, status_of};
use crate::cloud::Opened;
use crate::documents::{Documents, Picked};
use pswm_core::dbfile::hash_hex;
use pswm_core::documents::DocumentStore;
use pswm_core::remote::{Cloud, Location, RemoteError};
use pswm_core::session::Session;
use pswm_core::store::{Remote, Store};
use pswm_core::sync;
use pswm_core::vault::Vault;
use serde::Deserialize;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, Wry};
use zeroize::Zeroizing;

/// Where the new database lives.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Place {
    /// In `cloud`, with its visible copy in `folder`.
    Cloud { cloud: Cloud, folder: Picked },
    /// A local file in `folder`.
    Folder { folder: Picked },
}

/// Makes a new database named `name` with `password` and puts it in `place`;
/// the unlock screen comes next. Nothing is left behind when it fails.
#[tauri::command]
pub async fn create_database(app: AppHandle, name: String, password: String, place: Place) -> Result<Opened, String> {
    let password = Zeroizing::new(password);
    off_main(move || {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err("A database needs a name".into());
        }
        if password.is_empty() {
            return Err("A database needs a master password".into());
        }
        let file_name = format!("{}.kdbx", file_safe(&name));
        let store = app.state::<Store>();
        let taken: Vec<PathBuf> = store.read(|s| s.databases.iter().map(|k| k.file.clone()).collect());
        let working = sync::free_path(&store.dir().join("databases"), &file_name, &taken);
        Vault::create(&working, &name, Some(&password), None)?;
        let placed = put(&app, &working, &file_name, &place);
        if placed.is_err() {
            let _ = std::fs::remove_file(&working);
        }
        let copy_problem = placed?;
        Ok(Opened { status: status_of(&store, &app.state::<Session>()), copy_problem })
    })
    .await
}

/// Puts the new database's `working` copy in `place` and makes it the
/// database on the phone; why its visible copy could not be made, if it could not.
fn put(app: &AppHandle, working: &PathBuf, file_name: &str, place: &Place) -> Result<Option<String>, String> {
    let bytes = std::fs::read(working).map_err(|e| format!("Cannot read the new database: {e}"))?;
    let hash = hash_hex(&bytes);
    let remote = match place {
        Place::Cloud { cloud, .. } => {
            let (location, revision) = cloud.create(file_name, &bytes).map_err(|e| match e {
                RemoteError::Changed => format!("{file_name} is in {} already: choose another name", cloud.provider().name),
                e => e.message(),
            })?;
            Remote { location, revision: Some(revision), synced: Some(hash) }
        }
        Place::Folder { folder } => {
            let documents = app.state::<Documents<Wry>>();
            if documents.find(&folder.uri, file_name)?.is_some() {
                return Err(format!("{file_name} is in {} already: choose another name", folder.name));
            }
            let uri = documents.child(&folder.uri, file_name)?;
            documents.write(&uri, &bytes)?;
            let location = Location::Document { uri, name: file_name.to_string() };
            Remote { location, revision: Some(hash.clone()), synced: Some(hash) }
        }
    };
    let store = app.state::<Store>();
    store.update(|s| s.select(working.clone()).remote = Some(remote)).map_err(|e| format!("Cannot save the new database: {e}"))?;
    Ok(match place {
        Place::Cloud { folder, .. } => crate::visible::place(app, &folder.uri, &folder.name, file_name).err(),
        Place::Folder { .. } => None,
    })
}

/// `name` as a file name: characters no file name may hold become `-`.
fn file_safe(name: &str) -> String {
    name.chars().map(|c| if r#"/\:*?"<>|"#.contains(c) || c.is_control() { '-' } else { c }).collect()
}
