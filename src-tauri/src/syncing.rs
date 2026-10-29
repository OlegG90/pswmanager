//! Where a database syncs: a folder or a cloud store, linked or let go, and a
//! sync now. The syncs themselves run in `sync`.
use crate::databases::{can_switch, choose, cloud_in_use, cloud_of, forget_if_unused, save_as, Status};
use crate::{hello, pick, sync};
use pswm_core::{dbfile, oauth, remote};
use pswm_core::session::Session;
use pswm_core::store::Store;
use pswm_core::vault::Vault;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager, Window};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

/// Opens a database from a folder (a LAN share, a NAS): the file there is
/// copied to a place this PC keeps, which then syncs with it.
#[tauri::command(async)]
pub fn sync_with_folder(app: AppHandle, window: Window) -> Result<Status, String> {
    can_switch(&app)?;
    let Some(path) = pick(&window, "KeePass database", &["kdbx"])? else { return choose(app, |_| {}) };
    let location = remote::Location::Folder { path };
    let Some(local) = save_as(&app, &window, &location.file_name())? else { return choose(app, |_| {}) };
    sync::start(&app.state::<Store>(), location, local)?;
    sync::reset(&app);
    choose(app, |_| {})
}

/// The local database a new cloud file can start from, if one is open.
fn local_database(store: &Store) -> Option<PathBuf> {
    store.read(|s| s.current().filter(|d| d.remote.is_none()).map(|d| d.file.clone()))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CloudFiles {
    /// The databases the app can reach there.
    files: Vec<remote::CloudFile>,
    /// The local database's name, when it can be uploaded instead.
    upload: Option<String>,
}

/// Signs in to a cloud store in the browser, then lists the databases there
/// and the local one that could be uploaded instead.
#[tauri::command(async)]
pub fn sign_in_to_cloud(app: AppHandle, cloud: remote::Cloud) -> Result<CloudFiles, String> {
    // Also from the settings of an unlocked database (Upload, Link); opening
    // one from the store checks the switch itself.
    if sync::is_running(&app) {
        return Err("A sync is running; try again in a moment".into());
    }
    cloud.provider().sign_in(|url| app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string()))?;
    let files = cloud.list().map_err(|e| e.message())?;
    let upload = local_database(&app.state::<Store>()).map(|p| remote::file_name(&p));
    Ok(CloudFiles { files, upload })
}

/// The user gave up on a cloud store: stops waiting for the browser, and
/// signs out unless the database is already synced with that store.
#[tauri::command(async)]
pub fn cancel_cloud(app: AppHandle, cloud: remote::Cloud) {
    oauth::cancel_sign_in();
    if !cloud_in_use(&app.state(), cloud) {
        cloud.provider().sign_out();
    }
}

/// Syncs with `file` in the cloud store; without one, the local database is
/// uploaded there first (never over a file already there).
#[tauri::command(async)]
pub fn sync_with_cloud(
    app: AppHandle,
    cloud: remote::Cloud,
    file: Option<remote::CloudFile>,
    local: Option<String>,
) -> Result<Status, String> {
    let store = app.state::<Store>();
    match file {
        // Opened from the store: downloaded to the file the user chose.
        Some(file) => {
            can_switch(&app)?;
            let local = local.ok_or("Choose where the file goes on this PC")?;
            sync::start(&store, cloud.location(file), PathBuf::from(local))?;
        }
        // The current local database, uploaded (unlocked or not): it syncs with the new remote file.
        None => {
            if sync::is_running(&app) {
                return Err("A sync is running; try again in a moment".into());
            }
            let current = local_database(&store).ok_or("Open a local file first")?;
            let bytes = std::fs::read(&current).map_err(|e| format!("Cannot read {}: {e}", current.display()))?;
            let name = remote::file_name(&current);
            let (location, revision) = match cloud.create(&name, &bytes) {
                Ok(created) => created,
                Err(remote::RemoteError::Changed) => return Err(format!("{name} is already in {}: choose it instead", cloud.provider().name)),
                Err(e) => return Err(e.message()),
            };
            sync::attach(&store, &current, location, revision, &bytes)?;
        }
    }
    sync::reset(&app);
    choose(app, |_| {})
}

/// Stops syncing the current database: its file stays, as a plain local
/// file (a working copy an older version kept in `sync/` moves out into the
/// data folder first, where syncing again cannot overwrite it). Refused while
/// the file has changes the remote file lacks.
#[tauri::command(async)]
pub fn stop_sync(app: AppHandle) -> Result<Status, String> {
    if sync::is_running(&app) {
        return Err("A sync is running; try again in a moment".into());
    }
    let store = app.state::<Store>();
    if sync::has_pending(&store) {
        return Err("Changes made here are not in the remote file yet: unlock, and they are synced first".into());
    }
    let Some(file) = store.read(|s| s.current().filter(|d| d.remote.is_some()).map(|d| d.file.clone())) else {
        return choose(app, |_| {});
    };
    let in_sync_folder = file.starts_with(store.dir().join("sync"));
    if in_sync_folder && app.state::<Session>().is_unlocked() {
        return Err("Lock the database first: its file moves out of the app's data folder".into());
    }
    let kept = if in_sync_folder { sync::keep_as_local(&store, &file)? } else { file.clone() };
    if kept != file {
        hello::forget(&file);
    }
    let cloud = cloud_of(&store, &file);
    // The same entry, in place: its key file and position stay.
    let status = choose(app.clone(), |s| {
        if let Some(known) = s.databases.iter_mut().find(|d| d.file == file) {
            known.file = kept.clone();
            known.remote = None;
        }
        s.current = Some(kept);
    })?;
    forget_if_unused(&app, cloud);
    Ok(status)
}

/// Where the current database could sync: a database file in a folder,
/// picked with the file dialog (to link to), or where a new one goes (to
/// upload to).
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum SyncTarget {
    Folder { path: String },
    Cloud { cloud: remote::Cloud, file: remote::CloudFile },
}

impl SyncTarget {
    fn location(self) -> remote::Location {
        match self {
            SyncTarget::Folder { path } => remote::Location::Folder { path: PathBuf::from(path) },
            SyncTarget::Cloud { cloud, file } => cloud.location(file),
        }
    }
}

/// The current database, when it does not sync yet.
fn unsynced_database(store: &Store) -> Result<PathBuf, String> {
    match store.read(|s| s.current().map(|d| (d.file.clone(), d.remote.is_some()))) {
        None => Err("Choose a database first".into()),
        Some((_, true)) => Err("This database is synced already: stop syncing first".into()),
        Some((file, false)) => Ok(file),
    }
}

/// Syncs the current database with an existing remote file. When the two
/// differ, `choice` says what to do; without one nothing changes and the
/// answer is `false`. The sync that carries it out starts at once.
#[tauri::command(async)]
pub fn link_database(app: AppHandle, target: SyncTarget, choice: Option<sync::LinkChoice>) -> Result<bool, String> {
    let store = app.state::<Store>();
    let file = unsynced_database(&store)?;
    let location = target.location();
    if let remote::Location::Folder { path } = &location {
        if store.read(|s| s.lists(path, None)) {
            return Err("That file is a database in the list: link to a file of its own".into());
        }
    }
    let key = app.state::<Session>().read(Vault::snapshot).ok().flatten();
    let linked = sync::link(&store, &file, location, choice, key.as_ref())?;
    if linked {
        sync::reset(&app);
        sync::request(&app);
    }
    Ok(linked)
}

/// Puts the current database into a folder as a new file (never over one
/// already there) and syncs with it.
#[tauri::command(async)]
pub fn upload_to_folder(app: AppHandle, window: Window) -> Result<Status, String> {
    let store = app.state::<Store>();
    let file = unsynced_database(&store)?;
    let Some(target) = pick_new_place(&window, &remote::file_name(&file))? else { return choose(app, |_| {}) };
    let bytes = std::fs::read(&file).map_err(|e| format!("Cannot read {}: {e}", file.display()))?;
    use remote::Remote as _;
    let folder = remote::Folder { path: target.clone() };
    let revision = match folder.upload(&bytes, None) {
        Ok(revision) => revision,
        Err(remote::RemoteError::Changed) => return Err(format!("{} is already there: link to it instead", target.display())),
        Err(e) => return Err(e.message()),
    };
    sync::attach(&store, &file, remote::Location::Folder { path: target }, revision, &bytes)?;
    sync::reset(&app);
    choose(app, |_| {})
}

/// A place for a new file anywhere (a LAN share, say); `None` when cancelled.
fn pick_new_place(window: &Window, name: &str) -> Result<Option<PathBuf>, String> {
    let dialog = window.dialog().file().set_parent(window).add_filter("KeePass database", &["kdbx"]).set_file_name(name);
    let picked = dialog.blocking_save_file().map(|p| p.into_path().map_err(|e| e.to_string())).transpose()?;
    Ok(picked.map(dbfile::with_kdbx))
}

/// A database file in a folder to link to; `None` when cancelled.
#[tauri::command(async)]
pub fn pick_remote_file(window: Window) -> Result<Option<String>, String> {
    Ok(pick(&window, "KeePass database", &["kdbx"])?.map(|p| p.display().to_string()))
}

#[tauri::command(async)]
pub fn sync_now(app: AppHandle) {
    sync::request(&app);
}

#[tauri::command]
pub fn sync_status(app: AppHandle) -> sync::Status {
    sync::status(&app)
}
