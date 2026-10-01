//! A synced database's visible copy (`docs/spec-android.md`): the working copy
//! written into a folder the user picked, after every change, so the user sees
//! the database and can back it up. It is never read back (other devices use
//! the store), and never written over a change something else made to it.

use crate::documents::Documents;
use pswm_core::dbfile::{hash_hex, BAK};
use pswm_core::documents::DocumentStore;
use pswm_core::store::Store;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager, Wry};

/// Where it is kept in the state file (the phone has one database).
const KEY: &str = "visibleCopy";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VisibleCopy {
    /// The folder's name, for messages.
    folder_name: String,
    uri: String,
    /// The hash of what the app wrote there last; `None` before the first write.
    written: Option<String>,
}

fn get(store: &Store) -> Option<VisibleCopy> {
    store.read(|s| s.settings.get(KEY).cloned()).and_then(|v| serde_json::from_value(v).ok())
}

fn set(store: &Store, copy: Option<&VisibleCopy>) -> Result<(), String> {
    store
        .update(|s| match copy {
            Some(copy) => {
                s.settings.insert(KEY.into(), serde_json::to_value(copy).unwrap_or(Value::Null));
            }
            None => {
                s.settings.remove(KEY);
            }
        })
        .map_err(|e| format!("Cannot save the copy's place: {e}"))
}

/// Whether `folder` already has a file named `name` (the screen says it will
/// be kept as `.bak` first).
pub fn taken(app: &AppHandle, folder: &str, name: &str) -> Result<bool, String> {
    Ok(app.state::<Documents<Wry>>().find(folder, name)?.is_some())
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
    let store = app.state::<Store>();
    set(&store, Some(&VisibleCopy { folder_name: folder_name.into(), uri, written: None }))?;
    refresh(app).map_or(Ok(()), Err)
}

/// The folder the visible copy is in, if there is one.
pub fn folder_name(store: &Store) -> Option<String> {
    get(store).map(|c| c.folder_name)
}

/// Forgets the visible copy's place; the file stays.
pub fn forget(store: &Store) -> Result<(), String> {
    set(store, None)
}

fn working(app: &AppHandle) -> Result<Option<Vec<u8>>, String> {
    let Some(file) = app.state::<Store>().read(|s| s.current.clone()) else { return Ok(None) };
    std::fs::read(&file).map(Some).map_err(|e| format!("Cannot read the working copy: {e}"))
}

/// Writes the working copy into the visible copy when they differ: `None`
/// when that went well (or there is no visible copy), else what went wrong.
pub fn refresh(app: &AppHandle) -> Option<String> {
    let store = app.state::<Store>();
    let mut copy = get(&store)?;
    let result = (|| {
        let Some(bytes) = working(app)? else { return Ok(()) };
        let hash = hash_hex(&bytes);
        let documents = app.state::<Documents<Wry>>();
        let there = documents.read(&copy.uri)?.map(|b| hash_hex(&b));
        if there.as_ref() == Some(&hash) {
            copy.written = Some(hash);
            return set(&store, Some(&copy));
        }
        if copy.written.is_some() && there.is_some() && there != copy.written {
            return Err(format!("The copy in {} was changed by something else, so it is not updated", copy.folder_name));
        }
        documents.write(&copy.uri, &bytes)?;
        if documents.read(&copy.uri)?.as_deref() != Some(&bytes) {
            return Err(format!("The copy in {} did not read back as written", copy.folder_name));
        }
        copy.written = Some(hash);
        set(&store, Some(&copy))
    })();
    result.err()
}
