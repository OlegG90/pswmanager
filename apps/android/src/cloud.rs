//! Cloud stores on the phone (Dropbox, OneDrive, Google Drive). Signing in is
//! the core's ([pswm_core::oauth]); the browser comes back to the app's own
//! scheme instead of a loopback address, as registered with each store: a host
//! per store (`io.github.olegg90.pswmanager://dropbox`, `…://onedrive`), and
//! Google's form for an Android client (`io.github.olegg90.pswmanager:/oauth2redirect`).

use crate::app::{adopt, off_main, Status};
use crate::documents::Picked;
use pswm_core::oauth::Pending;
use pswm_core::remote::{Cloud, CloudFile, RemoteError};
use pswm_core::session::Session;
use pswm_core::store::Store;
use pswm_core::sync::{self, LinkChoice};
use pswm_core::vault::Vault;
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;

/// The scheme `tauri.conf.json` gives the deep-link plugin.
const SCHEME: &str = "io.github.olegg90.pswmanager";

/// Where the browser comes back to from `cloud`'s sign-in.
fn redirect(cloud: Cloud) -> String {
    match cloud {
        Cloud::Dropbox => format!("{SCHEME}://dropbox"),
        Cloud::OneDrive => format!("{SCHEME}://onedrive"),
        Cloud::Google => format!("{SCHEME}:/oauth2redirect"),
    }
}

/// The sign-in waiting for the browser to come back, if one is.
#[derive(Default)]
pub struct SignIn(Mutex<Option<(Cloud, Pending)>>);

/// How a sign-in ended (event `signed-in`).
#[derive(Clone, Serialize)]
struct SignedIn {
    error: Option<String>,
}

/// Sends the browser (the default one) to `cloud`'s sign-in; the answer comes to [on_open_url].
#[tauri::command]
pub fn sign_in(app: AppHandle, cloud: Cloud) -> Result<(), String> {
    let pending = cloud.provider().start(&redirect(cloud))?;
    app.opener().open_url(&pending.url, None::<&str>).map_err(|e| e.to_string())?;
    *app.state::<SignIn>().0.lock().unwrap() = Some((cloud, pending));
    Ok(())
}

/// An address the app was opened with: the end of a sign-in when it comes
/// back to the waiting sign-in's own address.
pub fn on_open_url(app: &AppHandle, url: &str) {
    if !url.starts_with(&format!("{SCHEME}:/")) {
        return;
    }
    let sign_in = app.state::<SignIn>();
    let taken = {
        let mut waiting = sign_in.0.lock().unwrap();
        match waiting.as_ref() {
            // Another store's address, or a stray link: the sign-in goes on waiting.
            Some((cloud, _)) if !url.starts_with(&redirect(*cloud)) => return,
            _ => waiting.take(),
        }
    };
    let Some((cloud, pending)) = taken else {
        // The app was closed while the browser was open: the sign-in starts again.
        let _ = app.emit("signed-in", SignedIn { error: Some("The sign-in was interrupted; sign in again".into()) });
        return;
    };
    let (app, url) = (app.clone(), url.to_string());
    tauri::async_runtime::spawn_blocking(move || {
        // The core checks it answers this sign-in (its state).
        let error = cloud.provider().finish(&pending, &url).err();
        let _ = app.emit("signed-in", SignedIn { error });
    });
}

/// The databases in the app's folder in `cloud`.
#[tauri::command]
pub async fn cloud_files(cloud: Cloud) -> Result<Vec<CloudFile>, String> {
    off_main(move || cloud.list().map_err(|e| e.message())).await
}

/// A database chosen, and why its visible copy could not be made, if it could not.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Opened {
    pub status: Status,
    pub copy_problem: Option<String>,
}

/// Makes a database in `cloud` the one on this phone: its working copy is
/// downloaded into the app's storage and syncs with the store from then on,
/// and its visible copy goes into `folder`. A copy that cannot be made does not
/// undo the rest: the sync sheet offers the folder again.
#[tauri::command]
pub async fn open_cloud_file(app: AppHandle, cloud: Cloud, file: CloudFile, folder: Picked) -> Result<Opened, String> {
    off_main(move || {
        let name = file.name.clone();
        adopt(&app, cloud.location(file))?;
        let copy_problem = crate::visible::place(&app, &folder.uri, &folder.name, &name).err();
        let status = crate::app::status_of(&app.state::<Store>(), &app.state::<Session>());
        Ok(Opened { status, copy_problem })
    })
    .await
}

/// Syncs the database on this phone (synced with no cloud store: a local file,
/// or one that stopped syncing) with `file` in `cloud`, merging the two, or
/// without `file` uploads it there as a new file. The visible copy that
/// stopping made the database's file is its copy again. The sync that carries
/// it out starts at once. A merge needs the database unlocked (its key).
#[tauri::command]
pub async fn sync_with_cloud(app: AppHandle, cloud: Cloud, file: Option<CloudFile>) -> Result<Status, String> {
    off_main(move || {
        let store = app.state::<Store>();
        if store.read(|s| s.remote().is_some_and(|r| r.location.cloud().is_some())) {
            return Err("This database syncs with a cloud store already".into());
        }
        let working = store.read(|s| s.current.clone()).ok_or("Choose a database first")?;
        // The copy that was the file meanwhile, as the last sync with it left it.
        let copy_written = store.read(|s| s.remote().and_then(|r| r.revision.clone()));
        crate::app::while_not_syncing(&app, || {
            match file {
                Some(file) => {
                    let key = app.state::<Session>().read(Vault::snapshot).ok().flatten().ok_or("Unlock the database first")?;
                    sync::link(&store, &working, cloud.location(file), Some(LinkChoice::Merge), Some(&key))?;
                }
                None => {
                    let bytes = std::fs::read(&working).map_err(|e| format!("Cannot read the database: {e}"))?;
                    let name = pswm_core::remote::file_name(&working);
                    let (location, revision) = cloud.create(&name, &bytes).map_err(|e| match e {
                        RemoteError::Changed => format!("{name} is in {} already: choose it instead", cloud.provider().name),
                        e => e.message(),
                    })?;
                    sync::attach(&store, &working, location, revision, &bytes)?;
                }
            }
            crate::visible::restore(&store, copy_written)
        })?;
        crate::app::start_sync(app.clone());
        Ok(crate::app::status_of(&store, &app.state::<Session>()))
    })
    .await
}
