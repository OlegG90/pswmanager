//! Dropbox on the phone. Signing in is the core's ([pswm_core::oauth]); the
//! browser comes back to the app's own scheme instead of a loopback address,
//! as registered with the Dropbox app.

use crate::app::{adopt, off_main, Status};
use pswm_core::dropbox::DROPBOX;
use pswm_core::oauth::Pending;
use pswm_core::remote::{Cloud, CloudFile};
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;

/// Its scheme is the one `tauri.conf.json` gives the deep-link plugin.
pub const REDIRECT: &str = "io.github.olegg90.pswmanager://dropbox";

/// The sign-in waiting for the browser to come back, if one is.
#[derive(Default)]
pub struct SignIn(Mutex<Option<Pending>>);

/// How a sign-in ended (event `signed-in`).
#[derive(Clone, Serialize)]
struct SignedIn {
    error: Option<String>,
}

/// Sends the browser (the default one) to Dropbox's sign-in; the answer comes to [on_open_url].
#[tauri::command]
pub fn sign_in_to_dropbox(app: AppHandle) -> Result<(), String> {
    let pending = DROPBOX.start(REDIRECT)?;
    app.opener().open_url(&pending.url, None::<&str>).map_err(|e| e.to_string())?;
    *app.state::<SignIn>().0.lock().unwrap() = Some(pending);
    Ok(())
}

/// An address the app was opened with: the end of a sign-in when it is ours.
pub fn on_open_url(app: &AppHandle, url: &str) {
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
    off_main(move || adopt(&app, Cloud::Dropbox.location(file))).await
}
