mod activity;
mod clipboard;
mod credentials;
mod data_dir;
mod dbfile;
mod dropbox;
mod edit;
mod file_watch;
mod generator;
mod icons;
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
use serde::Serialize;
use settings::Settings;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use remote::Remote as _;
use store::Store;
use tauri::{AppHandle, Emitter, Manager, State, Window, WindowEvent};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_opener::OpenerExt;
use edit::EntryData;
use vault::{EntryDetail, Listing, Vault};
use zeroize::Zeroizing;

/// Passed by the "Start with Windows" entry: start in the tray, locked.
const AUTOSTART_ARG: &str = "--autostart";
/// How often the inactivity check runs.
const CHECK_EVERY: Duration = Duration::from_secs(10);

fn is_autostart<S: AsRef<str>>(mut args: impl Iterator<Item = S>) -> bool {
    args.any(|arg| arg.as_ref() == AUTOSTART_ARG)
}

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
    database: Option<String>,
    /// Where the database is synced to, if anywhere.
    synced_with: Option<String>,
    key_file: Option<String>,
    unlocked: bool,
    notice: Option<String>,
}

#[tauri::command(async)]
fn status(store: State<Store>, session: State<Session>, notice: State<Notice>) -> Status {
    let shown = |p: &Option<PathBuf>| p.as_ref().map(|p| p.display().to_string());
    let (database, synced_with, key_file) =
        store.read(|s| (shown(&s.database), s.remote.as_ref().map(|r| r.location.describe()), shown(&s.key_file)));
    let notice = notice.0.get().cloned();
    Status { database, synced_with, key_file, unlocked: session.is_unlocked(), notice }
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

/// Another database can be chosen only while locked, and not while this
/// device has changes the remote file lacks: they would stay behind.
fn can_switch(app: &AppHandle) -> Result<(), String> {
    if app.state::<Session>().is_unlocked() {
        return Err("Lock the database first".into());
    }
    if sync::is_running(app) {
        return Err("A sync is running; try again in a moment".into());
    }
    if sync::has_pending(&app.state::<Store>()) {
        return Err("Changes made here are not in the remote file yet: unlock, and they are synced first".into());
    }
    Ok(())
}

fn synced_with_dropbox(store: &Store) -> bool {
    store.read(|s| matches!(s.remote.as_ref().map(|r| &r.location), Some(remote::Location::Dropbox { .. })))
}

/// Leaving a Dropbox database signs out: the refresh token is removed.
/// `was_dropbox` is asked before the switch, so a failed switch keeps it.
fn leave_cloud(app: &AppHandle, was_dropbox: bool) {
    if was_dropbox {
        dropbox::sign_out();
    }
    sync::reset(app);
}

/// Opens a local file: the app does not sync it.
#[tauri::command(async)]
fn pick_database(app: AppHandle, window: Window) -> Result<Status, String> {
    can_switch(&app)?;
    let Some(picked) = pick(&window, "KeePass database", &["kdbx"])? else { return choose(app, |_| {}) };
    leave_cloud(&app, synced_with_dropbox(&app.state()));
    choose(app, |s| {
        s.database = Some(picked);
        s.remote = None;
    })
}

/// The database becomes a file in a folder (a LAN share, a NAS), synced
/// through a working copy.
#[tauri::command(async)]
fn sync_with_folder(app: AppHandle, window: Window) -> Result<Status, String> {
    can_switch(&app)?;
    let Some(path) = pick(&window, "KeePass database", &["kdbx"])? else { return choose(app, |_| {}) };
    let location = remote::Location::Folder { path };
    let was_dropbox = synced_with_dropbox(&app.state());
    sync::start(&app.state::<Store>(), location)?;
    leave_cloud(&app, was_dropbox);
    choose(app, |_| {})
}

/// The local database a new cloud file can start from, if one is open.
fn local_database(store: &Store) -> Option<PathBuf> {
    store.read(|s| if s.remote.is_none() { s.database.clone() } else { None })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DropboxFiles {
    /// The databases in the app folder, as paths there.
    files: Vec<String>,
    /// The local database's name, when it can be uploaded instead.
    upload: Option<String>,
}

/// Signs in to Dropbox in the browser, then lists the databases in the app
/// folder and the local one that could be uploaded instead.
#[tauri::command(async)]
fn sign_in_to_dropbox(app: AppHandle) -> Result<DropboxFiles, String> {
    can_switch(&app)?;
    dropbox::sign_in(|url| app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string()))?;
    let files = dropbox::list_databases().map_err(|e| e.message())?;
    let upload = local_database(&app.state::<Store>()).and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
    Ok(DropboxFiles { files, upload })
}

/// The user gave up on Dropbox: stops waiting for the browser, and signs out
/// unless the database is already synced with Dropbox.
#[tauri::command(async)]
fn cancel_dropbox(app: AppHandle) {
    dropbox::cancel_sign_in();
    if !synced_with_dropbox(&app.state()) {
        dropbox::sign_out();
    }
}

/// Syncs with `path` in the Dropbox app folder; without one, the local
/// database is uploaded there first (never over a file already there).
#[tauri::command(async)]
fn sync_with_dropbox(app: AppHandle, path: Option<String>) -> Result<Status, String> {
    can_switch(&app)?;
    let store = app.state::<Store>();
    let path = match path {
        Some(path) => path,
        None => {
            let local = local_database(&store).ok_or("Open a local file first")?;
            let bytes = std::fs::read(&local).map_err(|e| format!("Cannot read {}: {e}", local.display()))?;
            let name = local.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "database.kdbx".into());
            let path = format!("/{name}");
            match (dropbox::Dropbox { path: path.clone() }).upload(&bytes, None) {
                Ok(_) => path,
                Err(remote::RemoteError::Changed) => return Err(format!("{name} is already in Dropbox: choose it instead")),
                Err(e) => return Err(e.message()),
            }
        }
    };
    sync::start(&store, remote::Location::Dropbox { path })?;
    sync::reset(&app);
    choose(app, |_| {})
}

/// Stops syncing. A folder's file becomes the database, used as a local
/// file; a cloud database's working copy moves out of `sync/` into the data
/// folder and becomes it, and the account is signed out. Refused while this
/// device has changes the remote file lacks.
#[tauri::command(async)]
fn stop_sync(app: AppHandle) -> Result<Status, String> {
    can_switch(&app)?;
    let store = app.state::<Store>();
    let Some((working, remote)) = store.read(|s| Some((s.database.clone()?, s.remote.clone()?))) else { return choose(app, |_| {}) };
    let database = match remote.location.local_path() {
        Some(path) => path,
        // Out of `sync/`, where starting to sync again would overwrite it.
        None => sync::keep_as_local(&store, &working)?,
    };
    leave_cloud(&app, synced_with_dropbox(&store));
    choose(app, |s| {
        s.remote = None;
        s.database = Some(database);
    })
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
    choose(app, |s| s.key_file = picked.or(s.key_file.take()))
}

#[tauri::command(async)]
fn clear_key_file(app: AppHandle) -> Result<Status, String> {
    choose(app, |s| s.key_file = None)
}

#[tauri::command(async)]
fn unlock(app: AppHandle, store: State<Store>, session: State<Session>, password: String) -> Result<Listing, String> {
    let password = Zeroizing::new(password);
    sync::ensure_working_copy(&store)?;
    let (database, key_file, synced) = store.read(|s| (s.database.clone(), s.key_file.clone(), s.remote.is_some()));
    let database = database.ok_or("Open a local file or sync with a folder first")?;
    let password = (!password.is_empty()).then_some(password.as_str());
    let vault = Vault::open(&database, password, key_file.as_deref())?;
    let listing = vault.listing();
    fetch_icons(&app, &listing);
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
    let _ = app.emit("database-changed", DiskChange { listing, changed });
}

/// Fetches the listed sites' missing icons in the background, if the user
/// allows it, and tells the window about each one that arrives.
fn fetch_icons(app: &AppHandle, listing: &Listing) {
    let store = app.state::<Store>();
    if !Settings::of(&store).download_icons() {
        return;
    }
    let mut hosts: Vec<String> = listing.entries.iter().filter_map(|e| e.host.clone()).collect();
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
        let _ = app.emit("locked", ());
        // What changed here goes up; a merge waits for the next unlock.
        sync::request(app);
    }
    clipboard::clear_if_ours();
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

#[tauri::command(async)]
fn entry(session: State<Session>, id: String) -> Result<EntryDetail, String> {
    session.with(|v| v.detail(&id))
}

/// One field's value, for showing it. Asked for only on "reveal".
#[tauri::command(async)]
fn reveal(session: State<Session>, id: String, field: String) -> Result<String, String> {
    session.with(|v| v.field(&id, &field)).map(|value| value.to_string())
}

/// Copies a field without the value passing through the frontend. Returns
/// the seconds until the clipboard is cleared.
#[tauri::command(async)]
fn copy_field(store: State<Store>, session: State<Session>, id: String, field: String) -> Result<u64, String> {
    let value = session.with(|v| v.field(&id, &field))?;
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
) -> Result<Saved, String> {
    let saved = session.with_mut(|v| {
        let (id, conflicts) = v.save_entry(id.as_deref(), base.as_ref(), &data)?;
        Ok(Saved { id, listing: v.listing(), conflicts })
    })?;
    fetch_icons(&app, &saved.listing);
    // Tags alone can wait for the next sync (hiding, locking, quitting).
    if !base.as_ref().is_some_and(|base| edit::only_tags_changed(base, &data)) {
        sync::upload_soon(&app);
    }
    Ok(saved)
}

/// Moves an entry to the recycle bin, saves the file and returns the new listing.
#[tauri::command(async)]
fn delete_entry(app: AppHandle, session: State<Session>, id: String) -> Result<Listing, String> {
    let listing = session.with_mut(|v| {
        v.delete_entry(&id)?;
        Ok(v.listing())
    })?;
    sync::upload_soon(&app);
    Ok(listing)
}

#[tauri::command(async)]
fn group_paths(session: State<Session>) -> Result<Vec<Vec<String>>, String> {
    session.read(Vault::group_paths)
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
    let registered = app.global_shortcut().on_shortcut(hotkey.as_str(), |app, _, event| {
        if event.state == ShortcutState::Pressed {
            window::toggle(app, true);
        }
    });
    if let Err(e) = registered {
        let _ = app.state::<Notice>().0.set(format!("The hotkey {hotkey} is not available: {e}"));
    }
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
    let autostarted = is_autostart(std::env::args().skip(1));
    tauri::Builder::default()
        // Must come first: a second launch hands over to this process and exits.
        // A second "Start with Windows" launch leaves the running app as it is.
        .plugin(tauri_plugin_single_instance::init(|app, args, _| {
            if !is_autostart(args.iter()) {
                window::show(app);
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::Builder::new().arg(AUTOSTART_ARG).build())
        .manage(Session::default())
        .manage(Activity::default())
        .manage(Notice::default())
        .manage(FileWatch::default())
        .manage(sync::Syncer::default())
        .invoke_handler(tauri::generate_handler![
            status,
            pick_database,
            sync_with_folder,
            sign_in_to_dropbox,
            sync_with_dropbox,
            cancel_dropbox,
            stop_sync,
            sync_now,
            sync_status,
            pick_key_file,
            clear_key_file,
            unlock,
            lock,
            listing,
            entry,
            reveal,
            copy_field,
            open_url,
            icon,
            touch,
            hide_window,
            edit_entry,
            save_entry,
            delete_entry,
            group_paths,
            totp,
            copy_totp,
            generate_password,
            password_strength,
        ])
        .on_window_event(|window, event| {
            // Closing the window only hides it; Quit is in the tray menu.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                window::hide(window.app_handle());
            }
        })
        .setup(move |app| {
            app.manage(Store::load(data_dir::resolve_state_file()));
            let handle = app.handle();
            window::open(handle, !autostarted)?;
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
