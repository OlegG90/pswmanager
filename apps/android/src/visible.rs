//! A synced database's visible copy (`docs/spec-android.md`): the working copy
//! written into a folder the user picked, after every change, so the user sees
//! the database and can back it up. It is never read back (other devices use
//! the store), and never written over a change something else made to it.

use crate::documents::Documents;
use pswm_core::dbfile::{hash_hex, BAK};
use pswm_core::documents::DocumentStore;
use pswm_core::remote::Location;
use pswm_core::store::{Remote, Store};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager, Wry};

/// Where it is kept in the state file (the phone has one database).
const KEY: &str = "visibleCopy";
/// Where it waits while syncing with the store is stopped (the copy is then
/// the database's file), to come back when it syncs with a store again.
const STOPPED: &str = "stoppedCopy";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VisibleCopy {
    /// The folder the user picked, and its name for messages.
    folder: String,
    folder_name: String,
    /// The file's name there.
    name: String,
    /// The file, as last found or made.
    uri: String,
    /// The hash of what the app wrote there last; `None` before the first write.
    written: Option<String>,
}

fn get(store: &Store) -> Option<VisibleCopy> {
    get_at(store, KEY)
}

fn get_at(store: &Store, key: &str) -> Option<VisibleCopy> {
    store.read(|s| s.settings.get(key).cloned()).and_then(|v| serde_json::from_value(v).ok())
}

fn set(store: &Store, copy: Option<&VisibleCopy>) -> Result<(), String> {
    set_at(store, KEY, copy)
}

fn set_at(store: &Store, key: &str, copy: Option<&VisibleCopy>) -> Result<(), String> {
    store
        .update(|s| match copy {
            Some(copy) => {
                s.settings.insert(key.into(), serde_json::to_value(copy).unwrap_or(Value::Null));
            }
            None => {
                s.settings.remove(key);
            }
        })
        .map_err(|e| format!("Cannot save the copy's place: {e}"))
}

/// Whether `folder` already has a file named `name` (the screen says it will
/// be kept as `.bak` first).
#[tauri::command]
pub async fn copy_name_taken(app: AppHandle, folder: String, name: String) -> Result<bool, String> {
    crate::app::off_main(move || Ok(app.state::<Documents<Wry>>().find(&folder, &name)?.is_some())).await
}

/// Makes `folder` the place of the database's visible copy and writes it; a
/// different file of the same name there is kept as `<name>.bak` first.
pub fn place(app: &AppHandle, folder: &str, folder_name: &str, name: &str) -> Result<(), String> {
    let documents = app.state::<Documents<Wry>>();
    let uri = documents.child(folder, name)?;
    if let Some(there) = documents.read(&uri)?.filter(|b| !b.is_empty()) {
        if Some(hash_hex(&there)) != working(app)?.map(|w| hash_hex(&w)) {
            documents.write(&documents.child(folder, &format!("{name}{BAK}"))?, &there)?;
        }
    }
    let mut copy = VisibleCopy { folder: folder.into(), folder_name: folder_name.into(), name: name.into(), uri, written: None };
    let result = write(app, &mut copy);
    set(&app.state::<Store>(), Some(&copy))?;
    result
}

/// The folder the visible copy is in, if there is one.
pub fn folder_name(store: &Store) -> Option<String> {
    get(store).map(|c| c.folder_name)
}

/// Forgets the visible copy's place (and one kept while syncing is stopped); the file stays.
pub fn forget(store: &Store) -> Result<(), String> {
    set(store, None)?;
    set_at(store, STOPPED, None)
}

/// The visible copy as the database's file from now on (syncing with the
/// store stopped): a document it syncs with like a local file, as it was last
/// written there, so changes the copy lacks go there at the next sync and a
/// change something else made to it is merged. `None` without a copy.
pub fn into_local_file(store: &Store) -> Result<Option<Remote>, String> {
    let Some(copy) = get(store) else { return Ok(None) };
    set_at(store, STOPPED, Some(&copy))?;
    set(store, None)?;
    let location = Location::Document { uri: copy.uri, name: copy.name };
    Ok(Some(Remote { location, revision: copy.written.clone(), synced: copy.written }))
}

fn working(app: &AppHandle) -> Result<Option<Vec<u8>>, String> {
    let Some(file) = app.state::<Store>().read(|s| s.current.clone()) else { return Ok(None) };
    std::fs::read(&file).map(Some).map_err(|e| format!("Cannot read the working copy: {e}"))
}

/// Syncing with a store again: the copy that was the database's file meanwhile
/// is its visible copy again, as it is now (the next sync writes the merged
/// database there).
pub fn restore(app: &AppHandle) -> Result<(), String> {
    let store = app.state::<Store>();
    let Some(mut copy) = get_at(&store, STOPPED) else { return Ok(()) };
    copy.written = app.state::<Documents<Wry>>().read(&copy.uri)?.map(|b| hash_hex(&b));
    set(&store, Some(&copy))?;
    set_at(&store, STOPPED, None)
}

/// Writes the working copy into the visible copy when they differ (after a
/// sync or a save): `None` when that went well or there is no visible copy,
/// else what went wrong.
pub fn refresh(app: &AppHandle) -> Option<String> {
    let store = app.state::<Store>();
    let mut copy = get(&store)?;
    let result = write(app, &mut copy);
    set(&store, Some(&copy)).err().or(result.err())
}

/// The working copy into `copy`, unless it is the same already or something
/// else changed it since the app last wrote it. A file the user removed is
/// made again.
fn write(app: &AppHandle, copy: &mut VisibleCopy) -> Result<(), String> {
    let Some(bytes) = working(app)? else { return Ok(()) };
    let hash = hash_hex(&bytes);
    let documents = app.state::<Documents<Wry>>();
    let there = documents.read(&copy.uri)?.map(|b| hash_hex(&b));
    if there.as_ref() == Some(&hash) {
        copy.written = Some(hash);
        return Ok(());
    }
    match there {
        None => copy.uri = documents.child(&copy.folder, &copy.name)?,
        Some(_) if copy.written.is_some() && there != copy.written => {
            return Err(format!("The copy in {} was changed by something else, so it is not updated", copy.folder_name));
        }
        Some(_) => {}
    }
    documents.write(&copy.uri, &bytes)?;
    if documents.read(&copy.uri)?.as_deref() != Some(&bytes) {
        return Err(format!("The copy in {} did not read back as written", copy.folder_name));
    }
    copy.written = Some(hash);
    Ok(())
}
