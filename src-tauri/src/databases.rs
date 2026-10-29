//! The databases in the list and the one open: choosing, creating and renaming
//! them, their keys and encryption, unlocking and locking, and what changes in
//! the file on disk.
use crate::activity::Activity;
use crate::{browser_host, chooser, clipboard, file_watch, hello, need_key, pick, sync, window, AutoTypeTarget, FileWatch, Notice};
use crate::entries::fetch_icons;
use pswm_core::{dbfile, edit, encryption, gauth, opened, remote, store, vault};
use pswm_core::dbfile::OpenError;
use pswm_core::session::{KeyNeeded, Session};
use pswm_core::settings::Settings;
use pswm_core::store::Store;
use pswm_core::vault::{Listing, Vault};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager, State, Window};
use tauri_plugin_dialog::DialogExt;
use zeroize::Zeroizing;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Status {
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
pub fn status(store: State<Store>, session: State<Session>, notice: State<Notice>) -> Status {
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

/// Records a change to the chosen files and returns the new status.
pub(crate) fn choose(app: AppHandle, change: impl FnOnce(&mut store::State)) -> Result<Status, String> {
    app.state::<Store>().update(change).map_err(|e| e.to_string())?;
    Ok(status(app.state(), app.state(), app.state()))
}

/// Another database can be chosen only while locked and not syncing. A
/// database left with changes its remote file lacks keeps them, and syncs
/// them when it is opened again.
pub(crate) fn can_switch(app: &AppHandle) -> Result<(), String> {
    if app.state::<Session>().is_unlocked() {
        return Err("Lock the database first".into());
    }
    if sync::is_running(app) {
        return Err("A sync is running; try again in a moment".into());
    }
    Ok(())
}

/// Opens a local file: the app does not sync it.
#[tauri::command(async)]
pub fn pick_database(app: AppHandle, window: Window) -> Result<Status, String> {
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
pub fn select_database(app: AppHandle, file: String) -> Result<Status, String> {
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
pub fn remove_database(app: AppHandle, file: String) -> Result<Status, String> {
    can_switch(&app)?;
    let store = app.state::<Store>();
    if store.read(|s| s.databases.iter().any(|d| d.file == Path::new(&file) && sync::is_pending(d))) {
        return Err("Changes made here are not in its remote file yet: unlock it, and they are synced first".into());
    }
    let cloud = cloud_of(&store, Path::new(&file));
    let status = choose(app.clone(), |s| s.remove(Path::new(&file)))?;
    forget_if_unused(&app, cloud);
    hello::forget(Path::new(&file));
    Ok(status)
}

/// How many old versions these history limits (-1: none) would remove, to
/// ask before lowering one.
#[tauri::command(async)]
pub fn history_limits_preview(session: State<Session>, max_items: isize, max_size: isize) -> Result<usize, String> {
    session.read(|v| v.versions_over_limits(max_items, max_size))
}

/// Sets the open database's history limits (-1: none); every entry's history
/// is trimmed to them, saved and synced like an edit.
#[tauri::command(async)]
pub fn set_history_limits(app: AppHandle, session: State<Session>, max_items: isize, max_size: isize) -> Result<vault::DatabaseSettings, String> {
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
pub fn rename_database_file(app: AppHandle, name: String) -> Result<Status, String> {
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
pub(crate) fn open_from_command_line(app: &AppHandle, path: PathBuf) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    can_switch(app)?;
    open_local_file(app, path).map(drop)
}

/// Asks where a database file goes on this PC; the dialog starts in
/// `Documents\PswManager` (created if missing), with `name` filled in.
pub(crate) fn save_as(app: &AppHandle, window: &Window, name: &str) -> Result<Option<PathBuf>, String> {
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
pub fn pick_new_file(app: AppHandle, window: Window, name: String, fresh: bool) -> Result<Option<String>, String> {
    let picked = save_as(&app, &window, &name)?;
    if fresh && picked.as_ref().is_some_and(|p| p.exists()) {
        return Err("A file with that name is already there: choose a name that is free".into());
    }
    Ok(picked.map(|p| p.display().to_string()))
}

/// A key file for a new database; `None` when the dialog was cancelled.
#[tauri::command(async)]
pub fn pick_key_file_path(window: Window) -> Result<Option<String>, String> {
    Ok(pick(&window, "Key file", &[])?.map(|p| p.display().to_string()))
}

/// A password as the key takes it: an empty one is none.
fn non_empty(text: &str) -> Option<&str> {
    Some(text).filter(|t| !t.is_empty())
}

/// How long unlocking takes on this PC with this encryption, in
/// milliseconds; as slow as an unlock.
#[tauri::command(async)]
pub fn encryption_unlock_time(encryption: encryption::Encryption) -> Result<u64, String> {
    encryption::unlock_time(&encryption).map(|time| time.as_millis() as u64)
}

/// Gives the open database another cipher and / or key derivation; saved
/// and synced like an edit.
#[tauri::command(async)]
pub fn set_encryption(app: AppHandle, session: State<Session>, encryption: encryption::Encryption) -> Result<vault::DatabaseSettings, String> {
    let settings = session.with_mut(|v| {
        v.set_encryption(&encryption)?;
        Ok(v.settings())
    })?;
    sync::upload_soon(&app);
    Ok(settings)
}

/// Makes a new key file where the user chooses (never over a file already
/// there); its path, or `None` when the dialog was cancelled.
#[tauri::command(async)]
pub fn create_key_file(window: Window) -> Result<Option<String>, String> {
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
/// A synced database syncs first, and its file goes up at once after.
#[tauri::command(async)]
pub fn change_master_key(app: AppHandle, current: String, password: String, key_file: Option<String>) -> Result<Status, String> {
    let (current, password) = (Zeroizing::new(current), Zeroizing::new(password));
    let store = app.state::<Store>();
    let known = store.read(|s| s.current().cloned()).ok_or("Choose a database first")?;
    let current = (non_empty(&current), known.key_file.as_deref());
    let key_file = key_file.map(PathBuf::from);
    // In step with the remote file first: after the change, the remote file,
    // still on the old key, opens no more.
    sync::sync_first(&app).map_err(|e| format!("The database could not sync first ({e}); the key is unchanged"))?;
    app.state::<Session>().with_mut(|v| v.change_key(current, non_empty(&password), key_file.as_deref()))?;
    // Sealed again at the next unlock with the new master password.
    hello::forget(&known.file);
    let status = choose(app.clone(), |s| {
        if let Some(d) = s.current_mut() {
            d.key_file = key_file;
        }
    })?;
    sync::request(&app);
    Ok(status)
}

/// The key of a copy that opened with no key known — the remote file, or the
/// file on this PC another program replaced — after another device changed
/// it. The copy is read again with it (synced, or read from disk): the key
/// follows the newer change, so the database may take it from now on, and
/// then the list takes its key file. Refused when it does not open the copy.
#[tauri::command(async)]
pub fn enter_other_key(app: AppHandle, password: String, key_file: Option<String>) -> Result<Status, String> {
    let password = Zeroizing::new(password);
    let key_file = key_file.map(PathBuf::from);
    let given = vault::key(non_empty(&password), key_file.as_deref())?;
    let session = app.state::<Session>();
    session.with_mut(|v| v.remember_key(given.clone()))?;
    // The file on this PC first (another program may have replaced it), then
    // the remote file. Reading derives keys: done without holding the database.
    let read_again = || -> Result<(), OpenError> {
        let Some(since) = session.read(Vault::snapshot)? else { return Ok(()) };
        if let Some(read) = since.read_changed()? {
            if let Some(changed) = adopt(&session, &since, read)? {
                show_changes(&app, changed);
            }
        }
        sync::sync_first(&app)
    };
    if let Err(e) = read_again() {
        // A key that opened nothing is not kept.
        session.with_mut(|v| v.forget_key(&given))?;
        return Err(match e {
            OpenError::OtherKey(_) => "This master password or key file does not open it either".into(),
            OpenError::Other(message) => message,
        });
    }
    need_key(&app, |k| k.local = false); // the remote file's is the sync's
    let key_taken = session.read(|v| v.uses_key(&given))??;
    if let Some(file) = current_file(&app.state()).filter(|_| key_taken) {
        hello::forget(&file);
    }
    choose(app, |s| {
        if let Some(d) = s.current_mut().filter(|_| key_taken) {
            d.key_file = key_file;
        }
    })
}

/// Creates a new, empty database at `file` and adds it to the list as the
/// current one.
#[tauri::command(async)]
pub fn create_database(app: AppHandle, file: String, password: String, key_file: Option<String>) -> Result<Status, String> {
    can_switch(&app)?;
    let password = Zeroizing::new(password);
    let (file, key_file) = (PathBuf::from(file), key_file.map(PathBuf::from));
    let name = file.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Passwords".into());
    Vault::create(&file, &name, (!password.is_empty()).then_some(password.as_str()), key_file.as_deref())?;
    sync::reset(&app);
    choose(app, |s| s.select(file).key_file = key_file)
}

#[tauri::command]
pub fn key_needed(session: State<Session>) -> KeyNeeded {
    session.key_needed()
}

#[tauri::command(async)]
pub fn pick_key_file(app: AppHandle, window: Window) -> Result<Status, String> {
    let picked = pick(&window, "Key file", &[])?;
    // The password Windows Hello kept went with the key file it had.
    if let Some(file) = current_file(&app.state()).filter(|_| picked.is_some()) {
        hello::forget(&file);
    }
    choose(app, |s| {
        if let (Some(d), Some(picked)) = (s.current_mut(), picked) {
            d.key_file = Some(picked);
        }
    })
}

#[tauri::command(async)]
pub fn clear_key_file(app: AppHandle) -> Result<Status, String> {
    if let Some(file) = current_file(&app.state()) {
        hello::forget(&file);
    }
    choose(app, |s| {
        if let Some(d) = s.current_mut() {
            d.key_file = None;
        }
    })
}

/// Unlocks with the master password and / or the key file. With Windows Hello
/// unlock on and no password sealed for it, or the sealed one due, the
/// password is then sealed (Windows Hello asks once; declining changes
/// nothing else).
#[tauri::command(async)]
pub fn unlock(app: AppHandle, password: String) -> Result<Listing, String> {
    let password = Zeroizing::new(password);
    let password = non_empty(&password);
    let (listing, database) = open(&app, password)?;
    let store = app.state::<Store>();
    let settings = Settings::of(&store);
    if settings.biometric_unlock() && !hello::ready(&database, settings.password_every()) && hello::available() {
        // After the window has the database: Windows Hello's prompt waits for the user.
        let password = password.map(|p| Zeroizing::new(p.to_string()));
        std::thread::spawn(move || window::without_topmost(&app, chooser::LABEL, || hello::seal(&database, password.as_deref().map(String::as_str))));
    }
    Ok(listing)
}

/// The current database's file.
fn current_file(store: &Store) -> Option<PathBuf> {
    store.read(|s| s.current().map(|d| d.file.clone()))
}

/// Whether the unlock screen offers Windows Hello: it is on, and a password
/// is sealed for the database that is not due.
#[tauri::command(async)]
pub fn hello_ready(store: State<Store>) -> bool {
    let settings = Settings::of(&store);
    settings.biometric_unlock()
        && current_file(&store).is_some_and(|file| hello::ready(&file, settings.password_every()))
        && hello::available()
}

/// The answer when the user closed Windows Hello's prompt.
pub(crate) const CANCELLED: &str = "cancelled";

/// Unlocks with the password sealed for Windows Hello. One that no longer
/// opens the database (its key changed elsewhere) goes: the master password
/// is asked for, and the new one sealed.
#[tauri::command(async)]
pub fn unlock_with_hello(app: AppHandle) -> Result<Listing, String> {
    const MASTER: &str = "Unlock with the master password";
    if !hello_ready(app.state()) {
        return Err(MASTER.into());
    }
    let database = current_file(&app.state()).ok_or("Choose a database first")?;
    let password = window::without_topmost(&app, chooser::LABEL, || hello::unseal(&database)).map_err(|failure| match failure {
        hello::Failure::Cancelled => CANCELLED.to_string(),
        hello::Failure::NotSealed => MASTER.into(),
        hello::Failure::Other(message) => message,
    })?;
    match open(&app, password.as_deref().map(String::as_str)) {
        Err(e) if e == dbfile::WRONG_KEY => {
            hello::forget(&database);
            Err("The password Windows Hello kept no longer opens the database: unlock with the master password".into())
        }
        opened => opened.map(|(listing, _)| listing),
    }
}

/// Opens the current database with this password and its key file, and makes
/// it the session's; returns its listing and file.
pub(crate) fn open(app: &AppHandle, password: Option<&str>) -> Result<(Listing, PathBuf), String> {
    let (store, session) = (app.state::<Store>(), app.state::<Session>());
    sync::ensure_working_copy(&store)?;
    let current = store.read(|s| s.current().cloned()).ok_or("Choose a database first")?;
    let (database, key_file, synced) = (current.file, current.key_file, current.remote.is_some());
    // Another device may have moved a synced database to a new key meanwhile (#203).
    let vault = Vault::open_current(&store, &database, vault::key(password, key_file.as_deref())?)?;
    let listing = vault.listing();
    fetch_icons(app, &listing);
    show_database(app, &listing.database);
    // Touched first, so the inactivity check never sees a fresh vault as idle.
    app.state::<Activity>().touch();
    session.set(Some(vault));
    browser_host::tell(true);
    // The main window and Auto-Type's chooser each unlock; the other follows.
    let _ = app.emit("unlocked", ());
    if synced {
        // Only this app writes the working copy; the remote file is what changes.
        sync::request(app);
    } else {
        watch_database(app, &database);
    }
    Ok((listing, database))
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
pub(crate) fn check_disk(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let session = app.state::<Session>();
        // A lock meanwhile makes any error moot.
        let report = |error: OpenError| {
            if session.is_unlocked() {
                // A file on a key this device does not know: kept, so the window offers to enter it.
                if let OpenError::OtherKey(_) = error {
                    need_key(&app, |k| k.local = true);
                }
                let _ = app.emit("database-error", error.to_string());
            }
        };
        let Ok(Some(since)) = session.read(Vault::snapshot) else { return }; // locked
        let read = match since.read_changed() {
            Ok(Some(read)) => read,
            Ok(None) => return,
            Err(error) => return report(error),
        };
        match adopt(&session, &since, read) {
            Ok(Some(changed)) => show_changes(&app, changed),
            Ok(None) => {}
            Err(error) => report(error),
        }
    });
}

/// Takes a file read since `since` and writes what this device kept from
/// before ([Vault::adopt], [Vault::save_pending]); the ids of the entries that
/// differ, or `None` when the file was read or written since.
fn adopt(session: &Session, since: &dbfile::Snapshot, read: dbfile::Read) -> Result<Option<Vec<String>>, OpenError> {
    session.with_mut(|v| {
        let Some(changed) = v.adopt(since, read) else { return Ok(Ok(None)) };
        Ok(v.save_pending().map(|()| Some(changed)))
    })?
}

/// Tells the window the database changed under it (another device's change
/// arrived): `database-changed` with the new listing.
pub(crate) fn show_changes(app: &AppHandle, changed: Vec<String>) {
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

/// Async like the rest: a save in progress holds the session, and waiting
/// for it must not freeze the window.
#[tauri::command(async)]
pub fn lock(app: AppHandle) {
    lock_now(&app);
}

/// Drops the database and every decrypted value, clears our clipboard copy,
/// and tells the window to show the unlock screen.
pub(crate) fn lock_now(app: &AppHandle) {
    let session = app.state::<Session>();
    if session.is_unlocked() {
        session.set(None);
        browser_host::tell(false);
        *app.state::<FileWatch>().0.lock().unwrap() = None;
        window::set_title(app, None);
        app.state::<gauth::Session>().cancel();
        browser_host::forget_questions(app);
        let _ = app.emit("locked", ());
        // What changed here goes up; a merge waits for the next unlock.
        sync::request(app);
    }
    clipboard::clear_if_ours();
    opened::clean(&opened::folder());
    // A window chosen before the lock (or while locked) is not typed into after it.
    app.state::<AutoTypeTarget>().0.lock().unwrap().take();
}

/// The settings kept in the open database's file.
#[tauri::command(async)]
pub fn database_settings(session: State<Session>) -> Result<vault::DatabaseSettings, String> {
    session.read(Vault::settings)
}

/// Changes a setting kept in the open database's file; the change is saved
/// and synced like an edit.
#[tauri::command(async)]
pub fn set_database_setting(app: AppHandle, session: State<Session>, setting: edit::Setting, value: String) -> Result<vault::DatabaseSettings, String> {
    let settings = session.with_mut(|v| {
        v.set_setting(setting, &value)?;
        Ok(v.settings())
    })?;
    show_database(&app, &settings);
    sync::upload_soon(&app);
    Ok(settings)
}

/// True when some database in the list is synced with `cloud`.
pub(crate) fn cloud_in_use(store: &Store, cloud: remote::Cloud) -> bool {
    store.read(|s| s.databases.iter().any(|d| d.remote.as_ref().and_then(|r| r.location.cloud()) == Some(cloud)))
}

/// After a database left the list or stopped syncing: the cloud it used is
/// signed out if no other database uses it, and the sync status starts afresh.
pub(crate) fn forget_if_unused(app: &AppHandle, cloud: Option<remote::Cloud>) {
    if let Some(cloud) = cloud.filter(|c| !cloud_in_use(&app.state(), *c)) {
        cloud.provider().sign_out();
    }
    sync::reset(app);
}

/// The cloud a database in the list is synced with, if any.
pub(crate) fn cloud_of(store: &Store, file: &Path) -> Option<remote::Cloud> {
    store.read(|s| s.databases.iter().find(|d| d.file == file).and_then(|d| d.remote.as_ref()).and_then(|r| r.location.cloud()))
}
