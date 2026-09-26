mod activity;
mod clipboard;
mod data_dir;
mod dbfile;
mod edit;
mod generator;
mod icons;
mod otp;
mod session_watch;
mod settings;
mod store;
mod tray;
mod vault;
mod window;

use activity::Activity;
use serde::Serialize;
use settings::Settings;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use store::Store;
use tauri::{AppHandle, Emitter, Manager, State, Window, WindowEvent};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_opener::OpenerExt;
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
        f(vault).ok_or_else(|| "That entry is no longer in the database".into())
    }

    fn set(&self, vault: Option<Vault>) {
        *self.0.lock().unwrap() = vault;
    }

    fn is_unlocked(&self) -> bool {
        self.0.lock().unwrap().is_some()
    }
}

/// Something the user should know that happened before the window could say
/// it (the global hotkey could not be registered).
#[derive(Default)]
struct Notice(OnceLock<String>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    database: Option<String>,
    key_file: Option<String>,
    unlocked: bool,
    notice: Option<String>,
}

#[tauri::command]
fn status(store: State<Store>, session: State<Session>, notice: State<Notice>) -> Status {
    let shown = |p: &Option<PathBuf>| p.as_ref().map(|p| p.display().to_string());
    let (database, key_file) = store.read(|s| (shown(&s.database), shown(&s.key_file)));
    let notice = notice.0.get().cloned();
    Status { database, key_file, unlocked: session.is_unlocked(), notice }
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

#[tauri::command(async)]
fn pick_database(app: AppHandle, window: Window) -> Result<Status, String> {
    let picked = pick(&window, "KeePass database", &["kdbx"])?;
    choose(app, |s| s.database = picked.or(s.database.take()))
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
    let (database, key_file) = store.read(|s| (s.database.clone(), s.key_file.clone()));
    let database = database.ok_or("Choose a database first")?;
    let password = (!password.is_empty()).then_some(password.as_str());
    let vault = Vault::open(&database, password, key_file.as_deref())?;
    let listing = vault.listing();
    if Settings::of(&store).download_icons() {
        let mut hosts: Vec<String> = listing.entries.iter().filter_map(|e| e.host.clone()).collect();
        hosts.sort();
        hosts.dedup();
        fetch_icons(app.clone(), hosts, store.dir().to_path_buf());
    }
    // Touched first, so the inactivity check never sees a fresh vault as idle.
    app.state::<Activity>().touch();
    session.set(Some(vault));
    Ok(listing)
}

/// Fetches missing site icons in the background and tells the window about
/// each one that arrives.
fn fetch_icons(app: AppHandle, hosts: Vec<String>, data_dir: PathBuf) {
    std::thread::spawn(move || {
        icons::Cache::in_data_dir(&data_dir).fetch_missing(hosts, |host| {
            let _ = app.emit("icon-ready", host);
        });
    });
}

#[tauri::command]
fn lock(app: AppHandle) {
    lock_now(&app);
}

/// Drops the database and every decrypted value, clears our clipboard copy,
/// and tells the window to show the unlock screen.
fn lock_now(app: &AppHandle) {
    let session = app.state::<Session>();
    if session.is_unlocked() {
        session.set(None);
        let _ = app.emit("locked", ());
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
    session.with(|v| Some(v.listing()))
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
        .invoke_handler(tauri::generate_handler![
            status,
            pick_database,
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
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running PswManager");
}
