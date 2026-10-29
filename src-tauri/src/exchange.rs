//! Entries brought in and taken out: imports from a file or from Google
//! Authenticator, and exports to a file.
use crate::databases::{hello_ready, CANCELLED};
use crate::entries::fetch_icons;
use crate::{hello, pick, sync};
use crate::read_qr;
use pswm_core::{cxf, file_export, file_import, gauth, qr, vault};
use pswm_core::session::Session;
use pswm_core::store::Store;
use pswm_core::vault::Listing;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, Window};
use tauri_plugin_dialog::DialogExt;
use zeroize::Zeroizing;

/// What an import from a file brought in.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileImported {
    /// The file's name, which the group is named after.
    file: String,
    group: String,
    added: usize,
    skipped: Vec<cxf::Skipped>,
    listing: Listing,
}

/// An import's next step: done, or the file's password is needed (an
/// encrypted Aegis export, #282), which [import_file_with_password] takes.
#[derive(Serialize)]
#[serde(tag = "step", rename_all = "camelCase")]
pub(crate) enum ImportStep {
    Done(Box<FileImported>),
    Password { file: String },
}

/// The file the last import read: the window offers to delete it, as it holds
/// every secret in clear.
#[derive(Default)]
pub(crate) struct ImportedFile(Mutex<Option<PathBuf>>);

/// The file waiting for its password, with where it was read.
pub(crate) type WaitingFile = file_import::WaitingFile<PathBuf>;

/// Imports a file another password manager exported (#238, see
/// [file_import]): the user picks it, and everything in it is added as new
/// entries in the group `Imported from <file> <date>`, saved and synced as
/// one change. `None` when no file was picked; a file only its password opens
/// waits for it ([ImportStep::Password]).
#[tauri::command(async)]
pub fn import_file(app: AppHandle, window: Window) -> Result<Option<ImportStep>, String> {
    if !app.state::<Session>().is_unlocked() {
        return Err("Unlock the database first".into());
    }
    // Only the file this import reads may be deleted after it.
    app.state::<ImportedFile>().0.lock().unwrap().take();
    app.state::<WaitingFile>().cancel();
    let Some(path) = pick(&window, "Exported passwords (CSV, JSON)", &["csv", "json"])? else { return Ok(None) };
    let file = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
    let content = Zeroizing::new(std::fs::read(&path).map_err(|e| format!("Cannot read {file}: {e}"))?);
    if file_import::needs_password(&content) {
        app.state::<WaitingFile>().hold(file.clone(), content, path);
        return Ok(Some(ImportStep::Password { file }));
    }
    let import = file_import::parse(&file, &content)?;
    drop(content); // wiped now rather than at the end
    add_imported(&app, file, path, import).map(|done| Some(ImportStep::Done(Box::new(done))))
}

/// Imports the file waiting for its password with `password` (see
/// [file_import::WaitingFile::open]).
#[tauri::command(async)]
pub fn import_file_with_password(app: AppHandle, password: String) -> Result<ImportStep, String> {
    let (file, import, path) = app.state::<WaitingFile>().open(&Zeroizing::new(password))?;
    add_imported(&app, file, path, import).map(|done| ImportStep::Done(Box::new(done)))
}

/// Lets go of the file waiting for its password.
#[tauri::command(async)]
pub fn cancel_import(app: AppHandle) {
    app.state::<WaitingFile>().cancel();
}

/// Adds what the file read holds, as [import_file] says.
fn add_imported(app: &AppHandle, file: String, path: PathBuf, import: cxf::Import) -> Result<FileImported, String> {
    let done = add_entries(app, file, import)?;
    *app.state::<ImportedFile>().0.lock().unwrap() = Some(path);
    Ok(done)
}

/// Adds `import`'s entries in a group of their own (`source` names where they
/// came from: the file, or the app), saved and synced as one change.
fn add_entries(app: &AppHandle, source: String, import: cxf::Import) -> Result<FileImported, String> {
    let group = import.group_name_today();
    let (added, listing) = app.state::<Session>().with_mut(|v| {
        let added = v.import(&import, &group)?;
        Ok((added, v.listing()))
    })?;
    if added > 0 {
        fetch_icons(app, &listing);
        sync::upload_soon(app);
    }
    Ok(FileImported { file: source, group, added, skipped: import.skipped, listing })
}

/// The page opens: no codes read yet (see [gauth::Session]).
#[tauri::command(async)]
pub fn gauth_start(app: AppHandle) {
    app.state::<gauth::Session>().start();
}

/// Reads one of Google Authenticator's export codes (as [scan_totp_qr]
/// reads one; a snip of a code read already is passed over for a picture)
/// and adds it to those read so far. Nothing when cancelled or the page is
/// gone.
#[tauri::command(async)]
pub fn gauth_read(app: AppHandle, window: Window) -> Result<Option<gauth::Progress>, String> {
    let session = app.state::<gauth::Session>();
    let Some(text) = read_qr(&window, qr::export_from_rgba, qr::export_from_image, |text| !session.is_read(text))? else { return Ok(None) };
    session.add(&text)
}

/// Imports the accounts of the export codes read so far, as a file's
/// entries are imported, and lets the codes go.
#[tauri::command(async)]
pub fn gauth_import(app: AppHandle) -> Result<FileImported, String> {
    if !app.state::<Session>().is_unlocked() {
        return Err("Unlock the database first".into());
    }
    app.state::<gauth::Session>().import(|import| add_entries(&app, import.exporter.clone(), import))
}

/// Lets go of the export codes read so far.
#[tauri::command(async)]
pub fn gauth_cancel(app: AppHandle) {
    app.state::<gauth::Session>().cancel();
}

/// Deletes the file the last import read (the user confirmed): only that one.
#[tauri::command(async)]
pub fn delete_imported_file(app: AppHandle) -> Result<(), String> {
    let path = app.state::<ImportedFile>().0.lock().unwrap().take().ok_or("No file was imported")?;
    std::fs::remove_file(&path).map_err(|e| format!("Cannot delete {}: {e}", path.display()))
}

/// Checks the user again before an export: the master password (with the
/// database's key file), or with none, Windows Hello. Every secret goes into
/// the file in clear, so being unlocked is not enough.
#[tauri::command(async)]
pub fn confirm_export(app: AppHandle, password: Option<String>) -> Result<(), String> {
    let store = app.state::<Store>();
    let current = store.read(|s| s.current().cloned()).ok_or("Choose a database first")?;
    let password = match password.map(Zeroizing::new) {
        Some(password) => Some(password),
        None => {
            if !hello_ready(app.state()) {
                return Err("Enter the master password".into());
            }
            hello::unseal(&current.file).map_err(|failure| match failure {
                hello::Failure::Cancelled => CANCELLED.to_string(),
                hello::Failure::NotSealed => "Enter the master password".into(),
                hello::Failure::Other(message) => message,
            })?
        }
    };
    let key = vault::key(password.as_deref().map(String::as_str).filter(|p| !p.is_empty()), current.key_file.as_deref())?;
    if !app.state::<Session>().read(|v| v.uses_key(&key))?? {
        return Err("The master password is not right".into());
    }
    app.state::<Session>().allow_export()
}

/// Writes the entries in use to a new file the user names (#240, see
/// [file_export]), once [confirm_export] allowed it. `None` when no file was
/// chosen. Never over a file that is there.
#[tauri::command(async)]
pub fn export_file(app: AppHandle, window: Window, format: file_export::Format) -> Result<Option<file_export::Exported>, String> {
    let session = app.state::<Session>();
    let export = session.export(format)?;
    let (filter, extensions): (&str, &[&str]) = match format {
        file_export::Format::Csv => ("CSV", &["csv"]),
        file_export::Format::Cxf => ("Credential Exchange (JSON)", &["json"]),
    };
    let dialog = window.dialog().file().set_parent(&window).add_filter(filter, extensions).set_file_name(&export.name);
    let Some(path) = dialog.blocking_save_file().map(|p| p.into_path().map_err(|e| e.to_string())).transpose()? else { return Ok(None) };
    let shown = path.display().to_string();
    file_export::write_new(&path, &shown, export.content.as_bytes())?;
    session.exported();
    Ok(Some(export.written(shown)))
}
