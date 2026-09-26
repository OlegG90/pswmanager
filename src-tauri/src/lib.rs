mod activity;
mod clipboard;
mod data_dir;
mod icons;
mod session_watch;
mod store;
mod tray;
mod vault;

use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use store::{Store, WindowGeometry};
use activity::Activity;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder, Window, WindowEvent};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_opener::OpenerExt;
use vault::{EntryDetail, Listing, Vault};
use zeroize::Zeroizing;

const DEFAULT_WIDTH: f64 = 900.0;
const DEFAULT_HEIGHT: f64 = 600.0;
const DEFAULT_HOTKEY: &str = "Ctrl+Alt+P";
/// Passed by the "Start with Windows" entry: start in the tray, locked.
const AUTOSTART_ARG: &str = "--autostart";
/// Seconds before a copied value is cleared.
const CLEAR_SECONDS_DEFAULT: u64 = 20;
const CLEAR_SECONDS_RANGE: (u64, u64) = (5, 120);

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
struct Notice(Mutex<Option<String>>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    database: Option<String>,
    key_file: Option<String>,
    unlocked: bool,
    notice: Option<String>,
}

#[tauri::command]
fn status(app: AppHandle, store: State<Store>, session: State<Session>) -> Status {
    let shown = |p: &Option<PathBuf>| p.as_ref().map(|p| p.display().to_string());
    let (database, key_file) = store.read(|s| (shown(&s.database), shown(&s.key_file)));
    let notice = app.state::<Notice>().0.lock().unwrap().clone();
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
    Ok(status(app.clone(), app.state(), app.state()))
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
    if setting(&store, "downloadIcons", Value::as_bool).unwrap_or(true) {
        let mut hosts: Vec<String> = listing.entries.iter().filter_map(|e| e.host.clone()).collect();
        hosts.sort();
        hosts.dedup();
        fetch_icons(app.clone(), hosts, store.dir().to_path_buf());
    }
    session.set(Some(vault));
    app.state::<Activity>().touch();
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
    hide(&app);
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
    let (min, max) = CLEAR_SECONDS_RANGE;
    let seconds = setting(&store, "clearClipboard", Value::as_u64).unwrap_or(CLEAR_SECONDS_DEFAULT).clamp(min, max);
    clipboard::copy(value, Duration::from_secs(seconds))?;
    Ok(seconds)
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

/// A setting from the state file, if it is set and of the expected type.
fn setting<T>(store: &Store, name: &str, read: impl FnOnce(&Value) -> Option<T>) -> Option<T> {
    store.read(|s| s.settings.get(name).and_then(read))
}

fn show_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return };
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    let _ = app.emit("window-shown", ());
}

/// Hides the window to the tray, remembering where it was.
fn hide(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return };
    if window.is_visible().unwrap_or(false) {
        let _ = remember_geometry(&window);
    }
    let _ = window.hide();
    if setting(&app.state::<Store>(), "lockWhenHidden", Value::as_bool).unwrap_or(false) {
        lock_now(app);
    }
}

/// Tray click and global hotkey: show the window, or hide it when it is
/// already in front.
fn toggle_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return };
    let in_front = window.is_visible().unwrap_or(false)
        && !window.is_minimized().unwrap_or(false)
        && window.is_focused().unwrap_or(false);
    if in_front {
        hide(app);
    } else {
        show_window(app);
    }
}

fn quit(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if window.is_visible().unwrap_or(false) {
            let _ = remember_geometry(&window);
        }
    }
    lock_now(app);
    app.exit(0);
}

/// Locks once the database has been left alone for `lockAfterMinutes`.
fn watch_inactivity(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(activity::CHECK_EVERY);
        let minutes = setting(&app.state::<Store>(), "lockAfterMinutes", Value::as_u64);
        let idle = app.state::<Activity>().idle_for();
        if activity::timeout(minutes).is_some_and(|limit| idle >= limit) {
            lock_now(&app);
        }
    });
}

/// Registers the show / hide hotkey from the `hotkey` setting. A key taken by
/// another app is reported on the unlock screen instead of failing the start.
fn register_hotkey(app: &AppHandle) {
    let hotkey = setting(&app.state::<Store>(), "hotkey", |v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| DEFAULT_HOTKEY.to_string());
    let registered = app.global_shortcut().on_shortcut(hotkey.as_str(), |app, _, event| {
        if event.state == ShortcutState::Pressed {
            toggle_window(app);
        }
    });
    if let Err(e) = registered {
        *app.state::<Notice>().0.lock().unwrap() = Some(format!("The hotkey {hotkey} is not available: {e}"));
    }
}

fn open_window(app: &AppHandle, visible: bool) -> tauri::Result<()> {
    let geometry = app.state::<Store>().read(|s| s.window.clone());
    let mut builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
        .title("PswManager")
        .min_inner_size(520.0, 360.0)
        .visible(false);
    builder = match &geometry {
        Some(g) => builder.inner_size(g.width, g.height).position(g.x, g.y).maximized(g.maximized),
        None => builder.inner_size(DEFAULT_WIDTH, DEFAULT_HEIGHT).center(),
    };
    let window = builder.build()?;
    if geometry.is_some() && !is_on_screen(&window)? {
        window.center()?;
    }
    if visible {
        window.show()?;
    }
    Ok(())
}

/// A saved position can point at a monitor that is no longer connected.
fn is_on_screen(window: &tauri::WebviewWindow) -> tauri::Result<bool> {
    let pos = window.outer_position()?;
    Ok(window.monitor_from_point(pos.x.into(), pos.y.into())?.is_some())
}

fn remember_geometry(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let maximized = window.is_maximized()?;
    let scale = window.scale_factor()?;
    let pos = window.outer_position()?.to_logical::<f64>(scale);
    let size = window.inner_size()?.to_logical::<f64>(scale);
    window.state::<Store>().update(|s| {
        // A maximized window keeps the size it will restore to.
        s.window = match (&s.window, maximized) {
            (Some(prev), true) => Some(WindowGeometry { maximized, ..prev.clone() }),
            _ => Some(WindowGeometry { x: pos.x, y: pos.y, width: size.width, height: size.height, maximized }),
        };
    })?;
    Ok(())
}

pub fn run() {
    let autostarted = std::env::args().skip(1).any(|arg| arg == AUTOSTART_ARG);
    tauri::Builder::default()
        // Must come first: a second launch hands over to this process and exits.
        .plugin(tauri_plugin_single_instance::init(|app, _, _| show_window(app)))
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
                hide(window.app_handle());
            }
        })
        .setup(move |app| {
            app.manage(Store::load(data_dir::resolve_state_file()));
            let handle = app.handle();
            open_window(handle, !autostarted)?;
            tray::create(handle)?;
            register_hotkey(handle);
            watch_inactivity(handle.clone());
            let on_leave = handle.clone();
            session_watch::watch(move || {
                if setting(&on_leave.state::<Store>(), "lockOnSessionLock", Value::as_bool).unwrap_or(true) {
                    lock_now(&on_leave);
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running PswManager");
}
