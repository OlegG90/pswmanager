mod activity;
mod cli;
mod clipboard;
mod credentials;
mod data_dir;
mod dbfile;
mod dropbox;
mod google;
mod onedrive;
mod edit;
mod file_watch;
mod generator;
mod health;
mod icons;
mod opened;
mod oauth;
mod otp;
mod remote;
mod session_watch;
mod settings;
mod store;
mod sync;
mod tray;
mod vault;
mod window;

use activity::Activity;
use serde::{Deserialize, Serialize};
use settings::Settings;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use store::Store;
use tauri::{AppHandle, Emitter, Manager, State, Window, WindowEvent};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use tauri_plugin_opener::OpenerExt;
use edit::EntryData;
use vault::{EntryDetail, Kind, Listing, Vault, Version, VersionDetail};
use zeroize::Zeroizing;

/// How often the inactivity check runs.
const CHECK_EVERY: Duration = Duration::from_secs(10);

/// The unlocked database, if any. Locking drops it, and with it every
/// decrypted value.
#[derive(Default)]
struct Session(Mutex<Option<Vault>>);

impl Session {
    fn with<R>(&self, f: impl FnOnce(&Vault) -> Option<R>) -> Result<R, String> {
        let vault = self.0.lock().unwrap();
        let vault = vault.as_ref().ok_or("The database is locked")?;
        f(vault).ok_or_else(|| edit::NOT_FOUND.into())
    }

    /// Like `with`, for reads that can only fail because the database is locked.
    fn read<R>(&self, f: impl FnOnce(&Vault) -> R) -> Result<R, String> {
        self.0.lock().unwrap().as_ref().map(f).ok_or_else(|| "The database is locked".into())
    }

    fn with_mut<R>(&self, f: impl FnOnce(&mut Vault) -> Result<R, String>) -> Result<R, String> {
        let mut vault = self.0.lock().unwrap();
        f(vault.as_mut().ok_or("The database is locked")?)
    }

    fn set(&self, vault: Option<Vault>) {
        *self.0.lock().unwrap() = vault;
    }

    fn is_unlocked(&self) -> bool {
        self.0.lock().unwrap().is_some()
    }
}

/// Watches the open database's file; dropping the watcher stops it.
#[derive(Default)]
struct FileWatch(Mutex<Option<notify::RecommendedWatcher>>);

/// Something the user should know that happened before the window could say
/// it (the global hotkey could not be registered).
#[derive(Default)]
struct Notice(OnceLock<String>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    /// The file of the database the unlock screen opens.
    database: Option<String>,
    /// Where it is synced to, if anywhere.
    synced_with: Option<String>,
    /// The kind of store it is synced with: `folder`, `dropbox`, `google` or `onedrive`.
    sync_kind: Option<&'static str>,
    key_file: Option<String>,
    /// Every database in the list, the current one among them.
    databases: Vec<DatabaseInfo>,
    unlocked: bool,
    notice: Option<String>,
}

/// A database in the list, as the unlock screen names it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DatabaseInfo {
    file: String,
    /// Its name as last unlocked, or the file name.
    name: String,
    file_name: String,
    /// As last unlocked; empty when it has none.
    description: String,
    sync_kind: Option<&'static str>,
}

#[tauri::command(async)]
fn status(store: State<Store>, session: State<Session>, notice: State<Notice>) -> Status {
    let shown = |p: &Path| p.display().to_string();
    let (database, synced_with, sync_kind, key_file, databases) = store.read(|s| {
        let current = s.current();
        let location = s.remote().map(|r| &r.location);
        let databases = s
            .databases
            .iter()
            .map(|d| DatabaseInfo {
                file: shown(&d.file),
                name: d.title(),
                file_name: remote::file_name(&d.file),
                description: d.description.clone().unwrap_or_default(),
                sync_kind: d.remote.as_ref().map(|r| r.location.kind()),
            })
            .collect();
        (
            current.map(|d| shown(&d.file)),
            location.map(|l| l.describe()),
            location.map(|l| l.kind()),
            current.and_then(|d| d.key_file.as_deref()).map(shown),
            databases,
        )
    });
    let notice = notice.0.get().cloned();
    Status { database, synced_with, sync_kind, key_file, databases, unlocked: session.is_unlocked(), notice }
}

fn pick(window: &Window, name: &str, extensions: &[&str]) -> Result<Option<PathBuf>, String> {
    let mut dialog = window.dialog().file().set_parent(window);
    if !extensions.is_empty() {
        dialog = dialog.add_filter(name, extensions);
    }
    dialog.blocking_pick_file().map(|p| p.into_path().map_err(|e| e.to_string())).transpose()
}

/// Records a change to the chosen files and returns the new status.
fn choose(app: AppHandle, change: impl FnOnce(&mut store::State)) -> Result<Status, String> {
    app.state::<Store>().update(change).map_err(|e| e.to_string())?;
    Ok(status(app.state(), app.state(), app.state()))
}

/// Another database can be chosen only while locked and not syncing. A
/// database left with changes its remote file lacks keeps them, and syncs
/// them when it is opened again.
fn can_switch(app: &AppHandle) -> Result<(), String> {
    if app.state::<Session>().is_unlocked() {
        return Err("Lock the database first".into());
    }
    if sync::is_running(app) {
        return Err("A sync is running; try again in a moment".into());
    }
    Ok(())
}

/// True when some database in the list is synced with `cloud`.
fn cloud_in_use(store: &Store, cloud: remote::Cloud) -> bool {
    store.read(|s| s.databases.iter().any(|d| d.remote.as_ref().and_then(|r| r.location.cloud()) == Some(cloud)))
}

/// After a database left the list or stopped syncing: the cloud it used is
/// signed out if no other database uses it, and the sync status starts afresh.
fn forget_if_unused(app: &AppHandle, cloud: Option<remote::Cloud>) {
    if let Some(cloud) = cloud.filter(|c| !cloud_in_use(&app.state(), *c)) {
        cloud.provider().sign_out();
    }
    sync::reset(app);
}

/// The cloud a database in the list is synced with, if any.
fn cloud_of(store: &Store, file: &Path) -> Option<remote::Cloud> {
    store.read(|s| s.databases.iter().find(|d| d.file == file).and_then(|d| d.remote.as_ref()).and_then(|r| r.location.cloud()))
}

/// Opens a local file: the app does not sync it.
#[tauri::command(async)]
fn pick_database(app: AppHandle, window: Window) -> Result<Status, String> {
    can_switch(&app)?;
    let Some(picked) = pick(&window, "KeePass database", &["kdbx"])? else { return choose(app, |_| {}) };
    open_local_file(&app, picked)
}

/// Makes `path` the current database, adding it to the list (without sync)
/// if it is not there yet.
fn open_local_file(app: &AppHandle, path: PathBuf) -> Result<Status, String> {
    sync::reset(app);
    choose(app.clone(), |s| {
        s.select(path);
    })
}

/// Makes another database in the list the current one.
#[tauri::command(async)]
fn select_database(app: AppHandle, file: String) -> Result<Status, String> {
    can_switch(&app)?;
    sync::reset(&app);
    choose(app, |s| {
        if s.databases.iter().any(|d| d.file == Path::new(&file)) {
            s.current = Some(PathBuf::from(&file));
        }
    })
}

/// Takes a database off the list; its file stays where it is.
#[tauri::command(async)]
fn remove_database(app: AppHandle, file: String) -> Result<Status, String> {
    can_switch(&app)?;
    let store = app.state::<Store>();
    if store.read(|s| s.databases.iter().any(|d| d.file == Path::new(&file) && sync::is_pending(d))) {
        return Err("Changes made here are not in its remote file yet: unlock it, and they are synced first".into());
    }
    let cloud = cloud_of(&store, Path::new(&file));
    let status = choose(app.clone(), |s| s.remove(Path::new(&file)))?;
    forget_if_unused(&app, cloud);
    Ok(status)
}

/// How many old versions these history limits (-1: none) would remove, to
/// ask before lowering one.
#[tauri::command(async)]
fn history_limits_preview(session: State<Session>, max_items: isize, max_size: isize) -> Result<usize, String> {
    session.read(|v| v.versions_over_limits(max_items, max_size))
}

/// Sets the open database's history limits (-1: none); every entry's history
/// is trimmed to them, saved and synced like an edit.
#[tauri::command(async)]
fn set_history_limits(app: AppHandle, session: State<Session>, max_items: isize, max_size: isize) -> Result<vault::DatabaseSettings, String> {
    let settings = session.with_mut(|v| {
        v.set_history_limits(max_items, max_size)?;
        Ok(v.settings())
    })?;
    sync::upload_soon(&app);
    Ok(settings)
}

/// Renames the current database's file (and its backups) in its folder;
/// the list follows, and a synced one keeps its remote file.
#[tauri::command(async)]
fn rename_database_file(app: AppHandle, name: String) -> Result<Status, String> {
    can_switch(&app)?;
    let store = app.state::<Store>();
    let current = store.read(|s| s.current().cloned()).ok_or("Choose a database first")?;
    let from = current.file;
    let to = dbfile::renamed(&from, &name)?;
    if store.read(|s| s.lists(&to, Some(&from))) {
        return Err(format!("{} is already in the list: choose another name", to.display()));
    }
    if matches!(current.remote.map(|r| r.location), Some(remote::Location::Folder { path }) if dbfile::same_file(&path, &to)) {
        return Err("That is the remote file this database syncs with: choose another name".into());
    }
    dbfile::rename(&from, &to)?;
    choose(app, |s| s.rename(&from, to.clone())).inspect_err(|_| {
        // The list could not follow: the files go back to the name it has.
        let _ = dbfile::rename(&to, &from);
    })
}

/// A database named on the command line (at start, or by a second launch),
/// opened when the app may switch to it.
fn open_from_command_line(app: &AppHandle, path: PathBuf) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    can_switch(app)?;
    open_local_file(app, path).map(drop)
}

/// Opens a database from a folder (a LAN share, a NAS): the file there is
/// copied to a place this PC keeps, which then syncs with it.
#[tauri::command(async)]
fn sync_with_folder(app: AppHandle, window: Window) -> Result<Status, String> {
    can_switch(&app)?;
    let Some(path) = pick(&window, "KeePass database", &["kdbx"])? else { return choose(app, |_| {}) };
    let location = remote::Location::Folder { path };
    let Some(local) = save_as(&app, &window, &location.file_name())? else { return choose(app, |_| {}) };
    sync::start(&app.state::<Store>(), location, local)?;
    sync::reset(&app);
    choose(app, |_| {})
}

/// Asks where a database file goes on this PC; the dialog starts in
/// `Documents\PswManager` (created if missing), with `name` filled in.
fn save_as(app: &AppHandle, window: &Window, name: &str) -> Result<Option<PathBuf>, String> {
    let mut dialog = window.dialog().file().set_parent(window).add_filter("KeePass database", &["kdbx"]).set_file_name(name);
    if let Ok(documents) = app.path().document_dir() {
        let folder = documents.join("PswManager");
        let _ = std::fs::create_dir_all(&folder);
        dialog = dialog.set_directory(folder);
    }
    let picked = dialog.blocking_save_file().map(|p| p.into_path().map_err(|e| e.to_string())).transpose()?;
    // The dialog may hand back a name without the extension.
    Ok(picked.map(dbfile::with_kdbx))
}

/// Where a new database, or the local file of one opened from a cloud
/// store, goes; `None` when the dialog was cancelled. With `fresh` (a new
/// database) a file already there is refused, whatever the dialog asked.
#[tauri::command(async)]
fn pick_new_file(app: AppHandle, window: Window, name: String, fresh: bool) -> Result<Option<String>, String> {
    let picked = save_as(&app, &window, &name)?;
    if fresh && picked.as_ref().is_some_and(|p| p.exists()) {
        return Err("A file with that name is already there: choose a name that is free".into());
    }
    Ok(picked.map(|p| p.display().to_string()))
}

/// A key file for a new database; `None` when the dialog was cancelled.
#[tauri::command(async)]
fn pick_key_file_path(window: Window) -> Result<Option<String>, String> {
    Ok(pick(&window, "Key file", &[])?.map(|p| p.display().to_string()))
}

/// Makes a new key file where the user chooses (never over a file already
/// there); its path, or `None` when the dialog was cancelled.
#[tauri::command(async)]
fn create_key_file(window: Window) -> Result<Option<String>, String> {
    let dialog = window.dialog().file().set_parent(&window).add_filter("Key file", &["keyx"]).set_file_name("PswManager.keyx");
    let Some(picked) = dialog.blocking_save_file() else { return Ok(None) };
    let mut path = picked.into_path().map_err(|e| e.to_string())?;
    if path.extension().is_none() {
        path.set_extension("keyx");
    }
    if path.exists() {
        return Err(format!("{} is already there: choose a name that is free", path.display()));
    }
    vault::create_key_file(&path)?;
    Ok(Some(path.display().to_string()))
}

/// Gives the open database a new master password and / or key file, after
/// the current master password (with the key file it has now) proved right.
/// An empty password means none; `key_file` is the one to use from now on.
/// A synced database's file goes up at once.
#[tauri::command(async)]
fn change_master_key(app: AppHandle, current: String, password: String, key_file: Option<String>) -> Result<Status, String> {
    let (current, password) = (Zeroizing::new(current), Zeroizing::new(password));
    let store = app.state::<Store>();
    let known = store.read(|s| s.current().cloned()).ok_or("Choose a database first")?;
    let session = app.state::<Session>();
    let given = (!current.is_empty()).then_some(current.as_str());
    if !session.read(|v| v.has_key(given, known.key_file.as_deref()))?? {
        return Err("The current master password is not right".into());
    }
    let key_file = key_file.map(PathBuf::from);
    let password = (!password.is_empty()).then_some(password.as_str());
    session.with_mut(|v| v.change_key(password, key_file.as_deref()))?;
    let status = choose(app.clone(), |s| {
        if let Some(d) = s.current_mut() {
            d.key_file = key_file;
        }
    })?;
    if known.remote.is_some() {
        sync::request(&app);
    }
    Ok(status)
}

/// Creates a new, empty database at `file` and adds it to the list as the
/// current one.
#[tauri::command(async)]
fn create_database(app: AppHandle, file: String, password: String, key_file: Option<String>) -> Result<Status, String> {
    can_switch(&app)?;
    let password = Zeroizing::new(password);
    let (file, key_file) = (PathBuf::from(file), key_file.map(PathBuf::from));
    let name = file.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Passwords".into());
    Vault::create(&file, &name, (!password.is_empty()).then_some(password.as_str()), key_file.as_deref())?;
    sync::reset(&app);
    choose(app, |s| s.select(file).key_file = key_file)
}

/// The local database a new cloud file can start from, if one is open.
fn local_database(store: &Store) -> Option<PathBuf> {
    store.read(|s| s.current().filter(|d| d.remote.is_none()).map(|d| d.file.clone()))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CloudFiles {
    /// The databases the app can reach there.
    files: Vec<remote::CloudFile>,
    /// The local database's name, when it can be uploaded instead.
    upload: Option<String>,
}

/// Signs in to a cloud store in the browser, then lists the databases there
/// and the local one that could be uploaded instead.
#[tauri::command(async)]
fn sign_in_to_cloud(app: AppHandle, cloud: remote::Cloud) -> Result<CloudFiles, String> {
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
fn cancel_cloud(app: AppHandle, cloud: remote::Cloud) {
    oauth::cancel_sign_in();
    if !cloud_in_use(&app.state(), cloud) {
        cloud.provider().sign_out();
    }
}

/// Syncs with `file` in the cloud store; without one, the local database is
/// uploaded there first (never over a file already there).
#[tauri::command(async)]
fn sync_with_cloud(
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
fn stop_sync(app: AppHandle) -> Result<Status, String> {
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
enum SyncTarget {
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
fn link_database(app: AppHandle, target: SyncTarget, choice: Option<sync::LinkChoice>) -> Result<bool, String> {
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
fn upload_to_folder(app: AppHandle, window: Window) -> Result<Status, String> {
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
fn pick_remote_file(window: Window) -> Result<Option<String>, String> {
    Ok(pick(&window, "KeePass database", &["kdbx"])?.map(|p| p.display().to_string()))
}

#[tauri::command(async)]
fn sync_now(app: AppHandle) {
    sync::request(&app);
}

#[tauri::command]
fn sync_status(app: AppHandle) -> sync::Status {
    sync::status(&app)
}

#[tauri::command(async)]
fn pick_key_file(app: AppHandle, window: Window) -> Result<Status, String> {
    let picked = pick(&window, "Key file", &[])?;
    choose(app, |s| {
        if let (Some(d), Some(picked)) = (s.current_mut(), picked) {
            d.key_file = Some(picked);
        }
    })
}

#[tauri::command(async)]
fn clear_key_file(app: AppHandle) -> Result<Status, String> {
    choose(app, |s| {
        if let Some(d) = s.current_mut() {
            d.key_file = None;
        }
    })
}

#[tauri::command(async)]
fn unlock(app: AppHandle, store: State<Store>, session: State<Session>, password: String) -> Result<Listing, String> {
    let password = Zeroizing::new(password);
    sync::ensure_working_copy(&store)?;
    let current = store.read(|s| s.current().cloned()).ok_or("Choose a database first")?;
    let (database, key_file, synced) = (current.file, current.key_file, current.remote.is_some());
    let password = (!password.is_empty()).then_some(password.as_str());
    let vault = Vault::open(&database, password, key_file.as_deref())?;
    let listing = vault.listing();
    fetch_icons(&app, &listing);
    show_database(&app, &listing.database);
    // Touched first, so the inactivity check never sees a fresh vault as idle.
    app.state::<Activity>().touch();
    session.set(Some(vault));
    if synced {
        // Only this app writes the working copy; the remote file is what changes.
        sync::request(&app);
    } else {
        watch_database(&app, &database);
    }
    Ok(listing)
}

/// Picks up changes other devices make while the database is open. Without a
/// watcher (a folder that cannot be watched) the window-shown check remains.
fn watch_database(app: &AppHandle, database: &std::path::Path) {
    let on_change = app.clone();
    let watcher = file_watch::watch(database, move || check_disk(&on_change))
        .map_err(|e| eprintln!("Cannot watch the database folder: {e}"))
        .ok();
    *app.state::<FileWatch>().0.lock().unwrap() = watcher;
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiskChange {
    listing: Listing,
    /// Entries that differ from what the window showed.
    changed: Vec<String>,
}

/// Reads the database file again if it changed on disk, and tells the window:
/// `database-changed` with the new listing, or `database-error` when the file
/// cannot be read now (nothing is saved until it can). Runs in the background;
/// the slow part (deriving the key) runs without holding the vault, so the
/// window, tray and hotkey stay responsive.
fn check_disk(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let session = app.state::<Session>();
        // A lock meanwhile makes any error moot.
        let report = |message: String| {
            if session.is_unlocked() {
                let _ = app.emit("database-error", message);
            }
        };
        let Ok(Some(since)) = session.read(Vault::snapshot) else { return }; // locked
        let read = match since.read_changed() {
            Ok(Some(read)) => read,
            Ok(None) => return,
            Err(message) => return report(message),
        };
        let adopted = session.with_mut(|v| {
            let Some(changed) = v.adopt(&since, read) else { return Ok(None) };
            v.save_pending()?;
            Ok(Some(changed))
        });
        match adopted {
            Ok(Some(changed)) => show_changes(&app, changed),
            Ok(None) => {}
            Err(message) => report(message),
        }
    });
}

/// Tells the window the database changed under it (another device's change
/// arrived): `database-changed` with the new listing.
fn show_changes(app: &AppHandle, changed: Vec<String>) {
    let Ok(listing) = app.state::<Session>().read(Vault::listing) else { return }; // locked meanwhile
    fetch_icons(app, &listing);
    show_database(app, &listing.database);
    let _ = app.emit("database-changed", DiskChange { listing, changed });
}

/// The open database's name in the window title, and its name and
/// description remembered for the unlock screen.
fn show_database(app: &AppHandle, settings: &vault::DatabaseSettings) {
    let store = app.state::<Store>();
    let _ = store.update_if(|s| s.current_mut().is_some_and(|k| k.remember(&settings.name, &settings.description)));
    let title = store.read(|s| s.current().map(store::Known::title));
    window::set_title(app, title.as_deref());
}

/// Fetches the listed sites' missing icons in the background, if the user
/// allows it, and tells the window about each one that arrives.
fn fetch_icons(app: &AppHandle, listing: &Listing) {
    let store = app.state::<Store>();
    if !Settings::of(&store).download_icons() {
        return;
    }
    // Only the entries in use: not the recycle bin's or the templates' sites.
    let mut hosts: Vec<String> = listing.entries.iter().filter(|e| e.kind == Kind::Entry).filter_map(|e| e.host.clone()).collect();
    hosts.sort();
    hosts.dedup();
    let (app, data_dir) = (app.clone(), store.dir().to_path_buf());
    std::thread::spawn(move || {
        icons::Cache::in_data_dir(&data_dir).fetch_missing(hosts, |host| {
            let _ = app.emit("icon-ready", host);
        });
    });
}

/// Async like the rest: a save in progress holds the session, and waiting
/// for it must not freeze the window.
#[tauri::command(async)]
fn lock(app: AppHandle) {
    lock_now(&app);
}

/// Drops the database and every decrypted value, clears our clipboard copy,
/// and tells the window to show the unlock screen.
fn lock_now(app: &AppHandle) {
    let session = app.state::<Session>();
    if session.is_unlocked() {
        session.set(None);
        *app.state::<FileWatch>().0.lock().unwrap() = None;
        window::set_title(app, None);
        let _ = app.emit("locked", ());
        // What changed here goes up; a merge waits for the next unlock.
        sync::request(app);
    }
    clipboard::clear_if_ours();
    opened::clean(&opened::folder());
}

/// The window reports use, which keeps the database unlocked.
#[tauri::command]
fn touch(activity: State<Activity>) {
    activity.touch();
}

/// Esc with nothing left to close: back to the tray.
#[tauri::command]
fn hide_window(app: AppHandle) {
    window::hide(&app);
}

#[tauri::command(async)]
fn listing(session: State<Session>) -> Result<Listing, String> {
    session.read(Vault::listing)
}

/// The settings kept in the open database's file.
#[tauri::command(async)]
fn database_settings(session: State<Session>) -> Result<vault::DatabaseSettings, String> {
    session.read(Vault::settings)
}

/// Changes a setting kept in the open database's file; the change is saved
/// and synced like an edit.
#[tauri::command(async)]
fn set_database_setting(app: AppHandle, session: State<Session>, setting: edit::Setting, value: String) -> Result<vault::DatabaseSettings, String> {
    let settings = session.with_mut(|v| {
        v.set_setting(setting, &value)?;
        Ok(v.settings())
    })?;
    show_database(&app, &settings);
    sync::upload_soon(&app);
    Ok(settings)
}

#[tauri::command(async)]
fn entry(session: State<Session>, id: String) -> Result<EntryDetail, String> {
    session.with(|v| v.detail(&id))
}

/// The entry's older versions: when and what changed, never values.
#[tauri::command(async)]
fn entry_history(session: State<Session>, id: String) -> Result<Vec<Version>, String> {
    session.with(|v| v.history(&id))
}

/// One older version, read only, secrets masked.
#[tauri::command(async)]
fn entry_version(session: State<Session>, id: String, index: usize) -> Result<VersionDetail, String> {
    session.with(|v| v.version(&id, index))
}

/// Makes an older version the entry's current content; saves and syncs as an edit.
#[tauri::command(async)]
fn restore_version(app: AppHandle, session: State<Session>, id: String, index: usize, saved: Option<String>) -> Result<Listing, String> {
    let listing = session.with_mut(|v| {
        v.restore_version(&id, index, saved.as_deref())?;
        Ok(v.listing())
    })?;
    sync::upload_soon(&app);
    Ok(listing)
}

/// One field's value (of an older version with `version`), for showing it.
/// Asked for only on "reveal".
#[tauri::command(async)]
fn reveal(session: State<Session>, id: String, field: String, version: Option<usize>) -> Result<String, String> {
    session.with(|v| v.field_in(&id, version, &field)).map(|value| value.to_string())
}

/// Copies a field without the value passing through the frontend. Returns
/// the seconds until the clipboard is cleared.
#[tauri::command(async)]
fn copy_field(store: State<Store>, session: State<Session>, id: String, field: String, version: Option<usize>) -> Result<u64, String> {
    let value = session.with(|v| v.field_in(&id, version, &field))?;
    if value.is_empty() {
        return Err(format!("{field} is empty"));
    }
    let clear_after = Settings::of(&store).clear_clipboard_after();
    clipboard::copy(value, clear_after)?;
    Ok(clear_after.as_secs())
}

/// Opens the entry's URL in the default browser (web addresses only).
#[tauri::command(async)]
fn open_url(app: AppHandle, session: State<Session>, id: String) -> Result<(), String> {
    let url = session.with(|v| Some(v.web_url(&id)))?.ok_or("The entry has no web address")?;
    app.opener().open_url(url.as_str(), None::<&str>).map_err(|e| e.to_string())
}

/// The entry with every value, secrets included, for the editor.
#[tauri::command(async)]
fn edit_entry(session: State<Session>, id: String) -> Result<EntryData, String> {
    session.with(|v| v.edit_data(&id))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    id: String,
    listing: Listing,
    /// Fields another device also changed; this edit replaced them.
    conflicts: Vec<String>,
}

/// Creates (no `id`) or changes an entry, saves the file and returns the new
/// listing with the entry's id.
#[tauri::command(async)]
fn save_entry(
    app: AppHandle,
    session: State<Session>,
    id: Option<String>,
    base: Option<EntryData>,
    data: EntryData,
    template: Option<bool>,
) -> Result<Saved, String> {
    let saved = session.with_mut(|v| {
        let (id, conflicts) = v.save_entry(id.as_deref(), base.as_ref(), &data, template.unwrap_or(false))?;
        Ok(Saved { id, listing: v.listing(), conflicts })
    })?;
    fetch_icons(&app, &saved.listing);
    // Tags alone can wait for the next sync (hiding, locking, quitting).
    if !base.as_ref().is_some_and(|base| edit::only_tags_changed(base, &data)) {
        sync::upload_soon(&app);
    }
    Ok(saved)
}

/// Saves one of the entry's files where the user chooses. The content goes
/// from the database to the file without passing through the frontend.
/// False when the user cancelled.
#[tauri::command(async)]
fn save_attachment(window: Window, session: State<Session>, id: String, name: String, version: Option<usize>) -> Result<bool, String> {
    let data = session.with(|v| v.attachment(&id, version, &name))?;
    // A name is a file name, but other clients may store a path.
    let file_name = name.rsplit(['/', '\\']).next().unwrap_or(&name);
    let chosen = window.dialog().file().set_parent(&window).set_file_name(file_name).blocking_save_file();
    let Some(path) = chosen.map(|p| p.into_path().map_err(|e| e.to_string())).transpose()? else { return Ok(false) };
    std::fs::write(&path, &*data).map_err(|e| format!("Cannot save the file: {e}"))?;
    Ok(true)
}

/// Opens one of the entry's files in the app Windows uses for its type,
/// from a read-only copy that is deleted when the database locks.
#[tauri::command(async)]
fn open_attachment(app: AppHandle, session: State<Session>, id: String, name: String, version: Option<usize>) -> Result<(), String> {
    if opened::is_runnable(&name) {
        return Err("Programs and scripts are not opened from the database; save the file to run it".into());
    }
    let data = session.with(|v| v.attachment(&id, version, &name))?;
    let path = opened::write(&opened::folder(), &name, &data).map_err(|e| format!("Cannot open the file: {e}"))?;
    app.opener().open_path(path.to_string_lossy(), None::<&str>).map_err(|e| format!("Cannot open the file: {e}"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Attached {
    /// The name the file got in the entry.
    name: String,
    listing: Listing,
}

/// A file the user picked to attach: its name and content.
type Picked = (String, Zeroizing<Vec<u8>>);

/// A file the user picks to attach; nothing when cancelled.
fn pick_attachment(window: &Window) -> Result<Option<Picked>, String> {
    let Some(path) = pick(window, "", &[])? else { return Ok(None) };
    let size = std::fs::metadata(&path).map_err(|e| format!("Cannot read the file: {e}"))?.len();
    if size > edit::MAX_ATTACHMENT as u64 {
        return Err(edit::too_big());
    }
    let data = Zeroizing::new(std::fs::read(&path).map_err(|e| format!("Cannot read the file: {e}"))?);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(Some((name, data)))
}

/// Replaces the content of one of the entry's files with a file the user
/// picks (the entry's history keeps the old content), saves the database
/// and returns the new listing; nothing when the user cancelled.
#[tauri::command(async)]
fn replace_attachment(app: AppHandle, window: Window, session: State<Session>, id: String, name: String) -> Result<Option<Listing>, String> {
    let Some((_, data)) = pick_attachment(&window)? else { return Ok(None) };
    let listing = session.with_mut(|v| {
        v.replace_attachment(&id, &name, &data)?;
        Ok(v.listing())
    })?;
    sync::upload_soon(&app);
    Ok(Some(listing))
}

/// Attaches a file the user picks to an entry and saves the database; the
/// content never passes through the frontend. Nothing when the user cancelled.
#[tauri::command(async)]
fn attach_file(app: AppHandle, window: Window, session: State<Session>, id: String) -> Result<Option<Attached>, String> {
    let Some((name, data)) = pick_attachment(&window)? else { return Ok(None) };
    let attached = session.with_mut(|v| {
        let name = v.attach(&id, &name, &data)?;
        Ok(Attached { name, listing: v.listing() })
    })?;
    sync::upload_soon(&app);
    Ok(Some(attached))
}

/// Removes a file from an entry (the entry's history keeps it), saves the
/// database and returns the new listing.
#[tauri::command(async)]
fn remove_attachment(app: AppHandle, session: State<Session>, id: String, name: String) -> Result<Listing, String> {
    let listing = session.with_mut(|v| {
        v.detach(&id, &name)?;
        Ok(v.listing())
    })?;
    sync::upload_soon(&app);
    Ok(listing)
}

/// Renames a file of an entry (the entry's history keeps the old name),
/// saves the database and returns the new name with the new listing.
#[tauri::command(async)]
fn rename_attachment(app: AppHandle, session: State<Session>, id: String, from: String, to: String) -> Result<Attached, String> {
    let renamed = session.with_mut(|v| {
        let name = v.rename_attachment(&id, &from, &to)?;
        Ok(Attached { name, listing: v.listing() })
    })?;
    sync::upload_soon(&app);
    Ok(renamed)
}

/// Gives entries a tag or takes it off (the star is the tag Favorite),
/// saves the file and returns the new listing. Tags wait for the next sync.
#[tauri::command(async)]
fn set_tag(session: State<Session>, ids: Vec<String>, tag: String, on: bool) -> Result<Listing, String> {
    session.with_mut(|v| {
        v.set_tag(&ids, &tag, on)?;
        Ok(v.listing())
    })
}

/// Renames a tag in every entry; tags wait for the next sync.
#[tauri::command(async)]
fn rename_tag(session: State<Session>, from: String, to: String) -> Result<Listing, String> {
    session.with_mut(|v| {
        v.rename_tag(&from, &to)?;
        Ok(v.listing())
    })
}

/// Takes a tag off every entry; tags wait for the next sync.
#[tauri::command(async)]
fn remove_tag(session: State<Session>, tag: String) -> Result<Listing, String> {
    session.with_mut(|v| {
        v.remove_tag(&tag)?;
        Ok(v.listing())
    })
}

/// Puts entries from the recycle bin back where they were, saves the file and
/// returns the new listing.
#[tauri::command(async)]
fn restore_entries(app: AppHandle, session: State<Session>, ids: Vec<String>) -> Result<Listing, String> {
    let listing = session.with_mut(|v| {
        v.restore(&ids)?;
        Ok(v.listing())
    })?;
    sync::upload_soon(&app);
    Ok(listing)
}

/// Removes entries in the recycle bin for good, saves the file and returns
/// the new listing.
#[tauri::command(async)]
fn delete_for_good(app: AppHandle, session: State<Session>, ids: Vec<String>) -> Result<Listing, String> {
    let listing = session.with_mut(|v| {
        v.delete_for_good(&ids)?;
        Ok(v.listing())
    })?;
    sync::upload_soon(&app);
    Ok(listing)
}

/// Removes everything in the recycle bin for good, saves the file and returns
/// the new listing.
#[tauri::command(async)]
fn empty_trash(app: AppHandle, session: State<Session>) -> Result<Listing, String> {
    let listing = session.with_mut(|v| {
        v.empty_bin()?;
        Ok(v.listing())
    })?;
    sync::upload_soon(&app);
    Ok(listing)
}

/// Moves entries to the recycle bin, saves the file and returns the new listing.
#[tauri::command(async)]
fn delete_entries(app: AppHandle, session: State<Session>, ids: Vec<String>) -> Result<Listing, String> {
    let listing = session.with_mut(|v| {
        v.delete_entries(&ids)?;
        Ok(v.listing())
    })?;
    sync::upload_soon(&app);
    Ok(listing)
}

/// An image file for an entry's own icon, as base64; `None` when cancelled.
#[tauri::command(async)]
fn pick_icon_image(window: Window) -> Result<Option<String>, String> {
    let Some(path) = pick(&window, "Image", &["png", "jpg", "jpeg", "gif", "webp"])? else { return Ok(None) };
    let bytes = std::fs::read(&path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    edit::check_icon_image(&bytes)?;
    Ok(Some(base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)))
}

/// The entry's current TOTP code, or nothing when it has no secret.
#[tauri::command(async)]
fn totp(session: State<Session>, id: String) -> Result<Option<otp::Code>, String> {
    session.read(|v| v.totp(&id))?
}

/// Copies the current TOTP code; returns the seconds until the clipboard is cleared.
#[tauri::command(async)]
fn copy_totp(store: State<Store>, session: State<Session>, id: String) -> Result<u64, String> {
    let code = session.read(|v| v.totp(&id))??.ok_or("The entry has no TOTP secret")?;
    let clear_after = Settings::of(&store).clear_clipboard_after();
    clipboard::copy(Zeroizing::new(code.code), clear_after)?;
    Ok(clear_after.as_secs())
}

#[tauri::command(async)]
fn generate_password(options: generator::Options) -> Result<String, String> {
    generator::generate(&options).map(|password| password.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Strength {
    /// 0 (guessed at once) to 4 (very hard).
    score: u8,
    /// How long an offline attack on a slow hash would take, e.g. "3 hours".
    crack_time: String,
}

#[tauri::command(async)]
fn password_strength(password: String) -> Strength {
    let password = Zeroizing::new(password);
    let estimate = zxcvbn::zxcvbn(&password, &[]);
    Strength {
        score: estimate.score().into(),
        crack_time: estimate.crack_times().offline_slow_hashing_1e4_per_second().to_string(),
    }
}

/// Reused, weak and old passwords; worked out here, so no password leaves the backend.
#[tauri::command(async)]
fn password_health(session: State<Session>) -> Result<health::Health, String> {
    session.read(Vault::health)
}

/// The settings screen's values.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsView {
    #[serde(flatten)]
    settings: settings::View,
    start_with_windows: bool,
}

fn settings_view(app: &AppHandle) -> SettingsView {
    SettingsView { settings: Settings::of(&app.state()).view(), start_with_windows: tray::autostart_enabled(app) }
}

#[tauri::command(async)]
fn settings(app: AppHandle) -> SettingsView {
    settings_view(&app)
}

/// Changes one setting; it applies at once, since every use reads it anew.
#[tauri::command(async)]
fn set_setting(app: AppHandle, name: String, value: serde_json::Value) -> Result<SettingsView, String> {
    if name == "startWithWindows" {
        tray::set_autostart(&app, value.as_bool().ok_or("Start with Windows is on or off")?)?;
    } else if name == "hotkey" {
        change_hotkey(&app, value.as_str().ok_or("A hotkey is a key combination")?)?;
    } else {
        Settings::of(&app.state()).set(&name, value)?;
        if name == "theme" {
            window::apply_theme(&app);
        }
    }
    Ok(settings_view(&app))
}

/// A cached site icon as a `data:` URL, or nothing yet.
#[tauri::command(async)]
fn icon(store: State<Store>, host: String) -> Option<String> {
    icons::Cache::in_data_dir(store.dir()).get(&host).and_then(|bytes| icons::data_url(&bytes))
}

/// Locks once the database has been left alone for `lockAfterMinutes`.
fn watch_inactivity(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(CHECK_EVERY);
        if !app.state::<Session>().is_unlocked() {
            continue;
        }
        let idle = app.state::<Activity>().idle_for();
        if Settings::of(&app.state()).lock_after().is_some_and(|limit| idle >= limit) {
            lock_now(&app);
        }
    });
}

/// Registers the show / hide hotkey from the `hotkey` setting. A key taken by
/// another app is reported on the unlock screen instead of failing the start.
fn register_hotkey(app: &AppHandle) {
    let hotkey = Settings::of(&app.state()).hotkey();
    if let Err(e) = listen_to(app, &hotkey) {
        let _ = app.state::<Notice>().0.set(format!("The hotkey {hotkey} is not available: {e}"));
    }
}

/// Shows or hides the window on `hotkey`.
fn listen_to(app: &AppHandle, hotkey: &str) -> Result<(), String> {
    let shortcut: Shortcut = hotkey.parse().map_err(|e| format!("{hotkey} is not a key combination: {e}"))?;
    app.global_shortcut()
        .on_shortcut(shortcut, |app, _, event| {
            if event.state == ShortcutState::Pressed {
                window::toggle(app, true);
            }
        })
        .map_err(|e| e.to_string())
}

/// Moves the show / hide hotkey to `hotkey`, kept only once it is
/// registered: a combination another app holds is refused and the old one
/// stays.
fn change_hotkey(app: &AppHandle, hotkey: &str) -> Result<(), String> {
    let old = Settings::of(&app.state()).hotkey();
    let same = |a: &str, b: &str| a.parse::<Shortcut>().ok().zip(b.parse::<Shortcut>().ok()).is_some_and(|(a, b)| a == b);
    if !same(&old, hotkey) {
        let modified = hotkey.parse::<Shortcut>().is_ok_and(|s| {
            s.mods.intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER)
        });
        if !modified {
            return Err("A hotkey needs Ctrl, Alt or Win, so typing does not trigger it".into());
        }
        listen_to(app, hotkey).map_err(|e| format!("{hotkey} is not available — another app may use it ({e})"))?;
        let _ = app.global_shortcut().unregister(old.as_str());
    }
    Settings::of(&app.state()).set("hotkey", hotkey.into())
}

/// Locks when Windows locks or the session is disconnected (`lockOnSessionLock`).
fn lock_on_session_lock(app: AppHandle) {
    session_watch::watch(move || {
        if Settings::of(&app.state()).lock_on_session_lock() {
            lock_now(&app);
        }
    });
}

pub fn run() {
    let cwd = std::env::current_dir().unwrap_or_default();
    let options = match cli::parse(std::env::args().skip(1), &cwd) {
        Ok(cli::Command::Run(options)) => options,
        Ok(cli::Command::Help) => return cli::print(cli::USAGE),
        Ok(cli::Command::Version) => return cli::print(&format!("PswManager {}", env!("CARGO_PKG_VERSION"))),
        Err(message) => {
            cli::print(&format!("{message}\n\n{}", cli::USAGE));
            std::process::exit(2);
        }
    };
    let state_file = options.data_dir.as_deref().map_or_else(data_dir::resolve_state_file, data_dir::in_folder);
    tauri::Builder::default()
        // Must come first: a second launch hands over to this process and exits.
        // A second "Start with Windows" launch leaves the running app as it is.
        .plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
            let Ok(cli::Command::Run(second)) = cli::parse(args.into_iter().skip(1), Path::new(&cwd)) else { return };
            if second.autostart {
                return;
            }
            window::show(app);
            if let Some(database) = second.database {
                // The window is up: tell it what happened.
                let _ = match open_from_command_line(app, database) {
                    Ok(()) => app.emit("status-changed", ()),
                    Err(message) => app.emit("notice", message),
                };
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::Builder::new().arg(cli::AUTOSTART).build())
        .manage(Session::default())
        .manage(Activity::default())
        .manage(Notice::default())
        .manage(FileWatch::default())
        .manage(sync::Syncer::default())
        .invoke_handler(tauri::generate_handler![
            status,
            pick_database,
            sync_with_folder,
            link_database,
            upload_to_folder,
            pick_remote_file,
            pick_new_file,
            pick_key_file_path,
            create_database,
            select_database,
            remove_database,
            sign_in_to_cloud,
            sync_with_cloud,
            cancel_cloud,
            stop_sync,
            sync_now,
            sync_status,
            pick_key_file,
            clear_key_file,
            unlock,
            lock,
            listing,
            database_settings,
            set_database_setting,
            rename_database_file,
            history_limits_preview,
            set_history_limits,
            create_key_file,
            change_master_key,
            entry,
            reveal,
            entry_history,
            entry_version,
            restore_version,
            copy_field,
            open_url,
            icon,
            touch,
            hide_window,
            edit_entry,
            save_entry,
            delete_entries,
            restore_entries,
            delete_for_good,
            empty_trash,
            set_tag,
            rename_tag,
            remove_tag,
            save_attachment,
            open_attachment,
            attach_file,
            remove_attachment,
            rename_attachment,
            replace_attachment,
            pick_icon_image,
            totp,
            copy_totp,
            generate_password,
            password_strength,
            settings,
            set_setting,
            password_health,
        ])
        .on_window_event(|window, event| {
            // Closing the window only hides it; Quit is in the tray menu.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                window::hide(window.app_handle());
            }
        })
        .setup(move |app| {
            app.manage(Store::load(state_file));
            opened::clean(&opened::folder()); // copies left by a crash
            let handle = app.handle();
            if let Some(database) = options.database {
                // The window is not up yet: the unlock screen shows the reason.
                if let Err(message) = open_from_command_line(handle, database) {
                    let _ = handle.state::<Notice>().0.set(message);
                }
            }
            window::open(handle, !options.autostart)?;
            tray::create(handle)?;
            register_hotkey(handle);
            watch_inactivity(handle.clone());
            lock_on_session_lock(handle.clone());
            sync::start_clock(handle.clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running PswManager");
}
