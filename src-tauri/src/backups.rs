//! Backups of the current database: where they go, how often, and one now.
use pswm_core::{backup, store};
use pswm_core::store::Store;
use serde::Serialize;
use tauri::{AppHandle, Manager, State, Window};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

/// The current database's backup, as the settings show it; times are seconds
/// since 1970.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BackupInfo {
    folder: Option<String>,
    every_days: u32,
    last: Option<u64>,
    next: Option<u64>,
    error: Option<String>,
    /// The intervals offered, in days.
    intervals: [u32; 4],
}

/// The intervals offered, in days; 0 is never.
const BACKUP_EVERY: [u32; 4] = [0, 2, 7, 30];

/// The current database's backup; `None` without a database.
#[tauri::command]
pub fn backup(store: State<Store>) -> Option<BackupInfo> {
    backup_info(&store)
}

/// [backup], for the commands that change it.
fn backup_info(store: &Store) -> Option<BackupInfo> {
    store.read(|s| {
        let b = s.current()?.backup.clone().unwrap_or_default();
        let folder = b.folder.as_ref().map(|f| f.display().to_string());
        Some(BackupInfo { folder, every_days: b.every_days, last: b.last, next: backup::next(&b), error: b.error, intervals: BACKUP_EVERY })
    })
}

/// Changes the current database's backup, then backs it up if that makes it due.
fn change_backup(store: &Store, change: impl FnOnce(&mut store::Backup)) -> Result<Option<BackupInfo>, String> {
    let database = store.read(|s| s.current.clone()).ok_or("Choose a database first")?;
    store
        .update(|s| {
            if let Some(d) = s.current_mut() {
                change(d.backup.get_or_insert_with(Default::default));
            }
        })
        .map_err(|e| format!("Cannot save the setting: {e}"))?;
    backup::run_due(store, backup::now(), Some(&database));
    Ok(backup_info(store))
}

#[tauri::command(async)]
pub fn set_backup_every(store: State<Store>, days: u32) -> Result<Option<BackupInfo>, String> {
    if !BACKUP_EVERY.contains(&days) {
        return Err(format!("Back up every {days} days is not offered"));
    }
    change_backup(&store, |b| b.every_days = days)
}

/// Asks for the folder the current database's copy goes to.
#[tauri::command(async)]
pub fn pick_backup_folder(store: State<Store>, window: Window) -> Result<Option<BackupInfo>, String> {
    let Some(folder) = window.dialog().file().set_parent(&window).blocking_pick_folder() else { return Ok(backup_info(&store)) };
    let folder = folder.into_path().map_err(|e| e.to_string())?;
    // Two databases of one file name would overwrite each other's copy.
    let taken = store.read(|s| backup::copy_taken(s, s.current.as_deref()?, &folder));
    if let Some(other) = taken {
        return Err(format!("{other} keeps its copy there under the same name: choose another folder"));
    }
    // A new folder has no copy yet: one goes there now.
    change_backup(&store, |b| (b.folder, b.error, b.last) = (Some(folder), None, None))
}

/// Backs up the current database now.
#[tauri::command(async)]
pub fn backup_now(store: State<Store>) -> Result<Option<BackupInfo>, String> {
    let database = store.read(|s| s.current.clone()).ok_or("Choose a database first")?;
    backup::run(&store, &database)?;
    Ok(backup_info(&store))
}

/// Opens the current database's backup folder in Explorer.
#[tauri::command]
pub fn show_backup_folder(app: AppHandle, store: State<Store>) -> Result<(), String> {
    let folder = store.read(|s| s.current()?.backup.as_ref()?.folder.clone()).ok_or("Choose a backup folder first")?;
    app.opener().open_path(folder.to_string_lossy(), None::<&str>).map_err(|e| format!("Cannot open the folder: {e}"))
}

/// Backs up the databases that are due, at start and every hour while the app runs.
pub(crate) fn start_backup_clock(app: AppHandle) {
    std::thread::spawn(move || loop {
        backup::run_due(&app.state::<Store>(), backup::now(), None);
        std::thread::sleep(backup::CHECK_EVERY);
    });
}
