//! Importing from another password manager on this phone (#152) through
//! `TransferPlugin.kt` (Android's Credential Transfer): the export comes
//! straight here, never to the page, and the core reads it into new entries
//! ([pswm_core::cxf]) in a group of their own. And offering the database for
//! export to them (#153): the export itself is `ExportActivity.kt`'s, through
//! `provider.rs`, as Android starts it outside the app's window.

use crate::app::off_main;
use crate::documents::Documents;
use pswm_core::cxf::{self, Import, Skipped};
use pswm_core::documents::DocumentStore;
use pswm_core::file_export::{self, Format};
use pswm_core::file_import;
use pswm_core::gauth;
use pswm_core::qr;
use pswm_core::session::Session;
use pswm_core::vault::Listing;
use serde::{Deserialize, Serialize};
use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, Runtime, Wry};
use zeroize::Zeroizing;

pub struct Transfer<R: Runtime>(PluginHandle<R>);

/// The file the last import read by its path (All files access): the app
/// offers to delete it, as it holds every secret in clear.
#[derive(Default)]
pub struct ImportedFile(Mutex<Option<PathBuf>>);

/// The file waiting for its password (an encrypted Aegis export, #282), with
/// where it was read when by its path (All files access).
type WaitingFile = file_import::WaitingFile<Option<PathBuf>>;

/// An import from a file's next step: done, or the file's password is
/// needed, which [import_file_with_password] takes.
#[derive(Serialize)]
#[serde(tag = "step", rename_all = "camelCase")]
pub enum ImportStep {
    Done(Box<Imported>),
    Password { file: String },
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("transfer")
        .setup(|app, api| {
            app.manage(Transfer(api.register_android_plugin("io.github.olegg90.pswmanager", "TransferPlugin")?));
            app.manage(ImportedFile::default());
            app.manage(WaitingFile::default());
            app.manage(gauth::Session::default());
            Ok(())
        })
        .build()
}

#[derive(Deserialize)]
struct Exported {
    json: String,
}

impl<R: Runtime> Transfer<R> {
    /// The export of the app the user picks; `None` when they went back.
    fn import(&self) -> Result<Option<Zeroizing<String>>, String> {
        match self.0.run_mobile_plugin::<Exported>("importCredentials", ()) {
            Ok(exported) => Ok(Some(Zeroizing::new(exported.json))),
            Err(e) => {
                let message = e.to_string();
                if message.contains("transfer:cancelled") {
                    Ok(None)
                } else if message.contains("transfer:none") {
                    Err("No app on this phone offers its passwords for import".into())
                } else {
                    Err(format!("The import did not work: {message}"))
                }
            }
        }
    }
}

#[derive(Serialize)]
struct ExportArgs<'a> {
    name: &'a str,
}

impl<R: Runtime> Transfer<R> {
    /// Offers the database (named `name` in the system's list) for export.
    pub fn register_export(&self, name: &str) -> Result<(), String> {
        self.0.run_mobile_plugin::<serde_json::Value>("registerExport", ExportArgs { name }).map(|_| ()).map_err(|e| e.to_string())
    }

    /// No longer offers it.
    pub fn clear_export(&self) -> Result<(), String> {
        self.0.run_mobile_plugin::<serde_json::Value>("clearExport", ()).map(|_| ()).map_err(|e| e.to_string())
    }
}

/// Offers the database on the phone for export to another password manager,
/// named `name` in the system's list (once unlocked: the page knows its name
/// then). Where Android's transfer is missing (no Google Play services), it
/// says why; the page lets that pass.
#[tauri::command]
pub async fn register_export(app: AppHandle, name: String) -> Result<(), String> {
    off_main(move || app.state::<Transfer<Wry>>().register_export(&name)).await
}

/// What an import brought.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    /// The app it came from.
    pub exporter: String,
    /// The group the entries were put in.
    pub group: String,
    /// How many entries were added.
    pub added: usize,
    /// What could not be brought over, and why.
    pub skipped: Vec<Skipped>,
    /// The list with the new entries.
    pub listing: Listing,
    /// It came from a file (which holds every secret in clear).
    pub from_file: bool,
    /// The app may delete that file ([delete_imported_file]).
    pub deletable: bool,
}

/// Imports from another app on this phone: the system's list of apps, then
/// everything the chosen one exports as new entries in the group
/// `Imported from <app> <date>`, saved and synced as any change. `None` when
/// the user went back.
#[tauri::command]
pub async fn import_from_app(app: AppHandle) -> Result<Option<Imported>, String> {
    off_main(move || {
        let session = app.state::<Session>();
        if !session.is_unlocked() {
            return Err("Unlock the database first".into());
        }
        let Some(json) = app.state::<Transfer<Wry>>().import()? else { return Ok(None) };
        let import = cxf::parse(&json)?;
        drop(json); // wiped now rather than at the end
        add(&app, import, "The database locked while the other app was open: unlock it and import again", false, None).map(Some)
    })
    .await
}

/// Imports a file another password manager exported (#238): CSV or JSON, see
/// [file_import]. Added as [import_from_app] adds. `None` when no file was
/// picked; a file only its password opens waits for it.
#[tauri::command]
pub async fn import_from_file(app: AppHandle) -> Result<Option<ImportStep>, String> {
    off_main(move || {
        if !app.state::<Session>().is_unlocked() {
            return Err("Unlock the database first".into());
        }
        app.state::<ImportedFile>().0.lock().unwrap().take();
        app.state::<WaitingFile>().cancel();
        let Some((name, content)) = app.state::<Documents<Wry>>().pick_to_read()? else { return Ok(None) };
        import_read(&app, name, Zeroizing::new(content), None).map(Some)
    })
    .await
}

/// Imports what a file read holds, or keeps it waiting for its password.
fn import_read(app: &AppHandle, name: String, content: Zeroizing<Vec<u8>>, path: Option<PathBuf>) -> Result<ImportStep, String> {
    if file_import::needs_password(&content) {
        app.state::<WaitingFile>().hold(name.clone(), content, path);
        return Ok(ImportStep::Password { file: name });
    }
    let import = file_import::parse(&name, &content)?;
    drop(content); // wiped now rather than at the end
    add(app, import, LOCKED_MEANWHILE, true, path).map(|done| ImportStep::Done(Box::new(done)))
}

/// Imports the file waiting for its password with `password` (see
/// [file_import::WaitingFile::open]).
#[tauri::command]
pub async fn import_file_with_password(app: AppHandle, password: String) -> Result<ImportStep, String> {
    off_main(move || {
        let (_, import, path) = app.state::<WaitingFile>().open(&Zeroizing::new(password))?;
        add(&app, import, LOCKED_MEANWHILE, true, path).map(|done| ImportStep::Done(Box::new(done)))
    })
    .await
}

/// Lets go of the file waiting for its password.
#[tauri::command]
pub fn cancel_import(app: AppHandle) {
    app.state::<WaitingFile>().cancel();
}

/// The sheet opens: no codes read yet (see [gauth::Session]).
#[tauri::command]
pub fn gauth_start(app: AppHandle) {
    app.state::<gauth::Session>().start();
}

/// Reads one of Google Authenticator's export codes with the camera
/// (`camera`) or in a picture the user picks, and adds it to those read so
/// far. Nothing when cancelled.
#[tauri::command]
pub async fn gauth_read(app: AppHandle, camera: bool) -> Result<Option<gauth::Progress>, String> {
    off_main(move || {
        let documents = app.state::<Documents<Wry>>();
        let text = if camera {
            let Some(text) = documents.scan_qr()? else { return Ok(None) };
            qr::export_text(text)?
        } else {
            let Some((_, content)) = documents.pick_image()? else { return Ok(None) };
            qr::export_from_image(&Zeroizing::new(content))?
        };
        app.state::<gauth::Session>().add(&text)
    })
    .await
}

/// Imports the accounts of the export codes read so far, as [import_from_app]
/// adds, and lets the codes go.
#[tauri::command]
pub async fn gauth_import(app: AppHandle) -> Result<Imported, String> {
    off_main(move || {
        if !app.state::<Session>().is_unlocked() {
            return Err("Unlock the database first".into());
        }
        let import = app.state::<gauth::Session>().import()?;
        add(&app, import, LOCKED_MEANWHILE, false, None)
    })
    .await
}

/// Lets go of the export codes read so far.
#[tauri::command]
pub fn gauth_cancel(app: AppHandle) {
    app.state::<gauth::Session>().cancel();
}

const LOCKED_MEANWHILE: &str = "The database locked meanwhile: unlock it and import again";

/// Imports the file `name` in a browsed folder (All files access, #207), as
/// [import_from_file] does; the app may then delete it.
#[tauri::command]
pub async fn import_file_at(app: AppHandle, folder: String, name: String) -> Result<ImportStep, String> {
    off_main(move || {
        if !app.state::<Session>().is_unlocked() {
            return Err("Unlock the database first".into());
        }
        app.state::<ImportedFile>().0.lock().unwrap().take();
        app.state::<WaitingFile>().cancel();
        let name = crate::phone_files::file_name(&name)?;
        let path = crate::phone_files::within_storage(Some(&folder))?.join(&name);
        let content = Zeroizing::new(std::fs::read(&path).map_err(|e| format!("Cannot read {name}: {e}"))?);
        import_read(&app, name, content, Some(path))
    })
    .await
}

/// Deletes the file the last import read by its path (the user confirmed): only that one.
#[tauri::command]
pub async fn delete_imported_file(app: AppHandle) -> Result<(), String> {
    off_main(move || {
        let path = app.state::<ImportedFile>().0.lock().unwrap().take().ok_or("No file was imported")?;
        std::fs::remove_file(&path).map_err(|e| format!("Cannot delete {}: {e}", path.display()))
    })
    .await
}

/// Writes the entries in use to a new file (#240) where Android's save picker
/// puts it; `None` when none was chosen.
#[tauri::command]
pub async fn export_to(app: AppHandle, format: Format) -> Result<Option<file_export::Exported>, String> {
    off_main(move || {
        let session = app.state::<Session>();
        let export = session.export(format)?;
        let documents = app.state::<Documents<Wry>>();
        let Some(uri) = documents.pick_to_save(&export.name)? else { return Ok(None) };
        documents.write(&uri, export.content.as_bytes()).map_err(|e| format!("Cannot write {}: {e}", export.name))?;
        session.exported();
        let name = export.name.clone();
        Ok(Some(export.written(name)))
    })
    .await
}

/// Writes the entries in use to a new file `name` in a browsed folder (All
/// files access), never over a file there.
#[tauri::command]
pub async fn export_at(app: AppHandle, format: Format, folder: String, name: String) -> Result<file_export::Exported, String> {
    off_main(move || {
        let session = app.state::<Session>();
        let export = session.export(format)?;
        let name = crate::phone_files::file_name(&name)?;
        let path = crate::phone_files::within_storage(Some(&folder))?.join(&name);
        file_export::write_new(&path, &name, export.content.as_bytes())?;
        session.exported();
        Ok(export.written(name))
    })
    .await
}

/// The name an export's file is offered under.
#[tauri::command]
pub fn export_name(app: AppHandle, format: Format) -> Result<String, String> {
    let name = app.state::<Session>().read(|v| v.database_name())?;
    Ok(pswm_core::file_export::file_name(&name, format))
}

/// Adds what was read as new entries in the group `Imported from <exporter>
/// <date>`, saved and synced as any change; `locked` says why it failed when
/// the database locked on the way. `from_file`: it came from a file; `file`:
/// that file read by its path, which the app may then delete.
fn add(app: &AppHandle, import: Import, locked: &str, from_file: bool, file: Option<PathBuf>) -> Result<Imported, String> {
    let session = app.state::<Session>();
    let group = import.group_name_today();
    let (added, listing) = session
        .with_mut(|v| {
            let added = v.import(&import, &group)?;
            Ok((added, v.listing()))
        })
        .map_err(|e| if session.is_unlocked() { e } else { locked.to_string() })?;
    if added > 0 {
        crate::icons::fetch(app, &listing);
        crate::editing::upload_soon(app);
    }
    let deletable = file.is_some();
    *app.state::<ImportedFile>().0.lock().unwrap() = file;
    Ok(Imported { exporter: import.exporter, group, added, skipped: import.skipped, listing, from_file, deletable })
}
