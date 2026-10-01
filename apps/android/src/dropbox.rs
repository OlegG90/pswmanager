//! Dropbox on the phone. Signing in is the core's ([pswm_core::oauth]); the
//! browser comes back to the app's own scheme instead of a loopback address,
//! as registered with the Dropbox app.

use crate::app::{off_main, status_of, Status};
use pswm_core::dropbox::DROPBOX;
use pswm_core::oauth::Pending;
use pswm_core::remote::{Cloud, CloudFile};
use pswm_core::session::Session;
use pswm_core::store::Store;
use pswm_core::sync;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;

pub const REDIRECT: &str = "io.github.olegg90.pswmanager://dropbox";

/// The sign-in waiting for the browser to come back, if one is.
#[derive(Default)]
pub struct SignIn(Mutex<Option<Pending>>);

/// How a sign-in ended (event `signed-in`).
#[derive(Clone, Serialize)]
struct SignedIn {
    error: Option<String>,
}

/// Sends the browser to Dropbox's sign-in; the answer comes to [answer].
#[tauri::command]
pub fn sign_in_to_dropbox(app: AppHandle) -> Result<(), String> {
    let pending = DROPBOX.start(REDIRECT)?;
    app.opener().open_url(&pending.url, None::<&str>).map_err(|e| e.to_string())?;
    *app.state::<SignIn>().0.lock().unwrap() = Some(pending);
    Ok(())
}

/// An address the app was opened with: the end of a sign-in when it is ours.
pub fn answer(app: &AppHandle, url: &str) {
    if !url.starts_with(REDIRECT) {
        return;
    }
    let Some(pending) = app.state::<SignIn>().0.lock().unwrap().take() else {
        // The app was closed while the browser was open: the sign-in starts again.
        let _ = app.emit("signed-in", SignedIn { error: Some("The Dropbox sign-in was interrupted; sign in again".into()) });
        return;
    };
    let (app, url) = (app.clone(), url.to_string());
    tauri::async_runtime::spawn_blocking(move || {
        let error = DROPBOX.finish(&pending, &url).err();
        let _ = app.emit("signed-in", SignedIn { error });
    });
}

/// The databases in the app's Dropbox folder.
#[tauri::command]
pub async fn dropbox_files() -> Result<Vec<CloudFile>, String> {
    off_main(|| Cloud::Dropbox.list().map_err(|e| e.message())).await
}

/// Makes a database in Dropbox the one on this phone: its working copy is
/// downloaded into the app's storage, and it syncs with Dropbox from then on.
#[tauri::command]
pub async fn open_dropbox_file(app: AppHandle, file: CloudFile) -> Result<Status, String> {
    off_main(move || {
        let store = app.state::<Store>();
        let taken: Vec<PathBuf> = store.read(|s| s.databases.iter().map(|k| k.file.clone()).collect());
        let local = sync::free_path(&store.dir().join("databases"), &file.name, &taken);
        sync::start(&store, Cloud::Dropbox.location(file), local)?;
        Ok(status_of(&store, &app.state::<Session>()))
    })
    .await
}
