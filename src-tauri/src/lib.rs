mod clipboard;
mod data_dir;
mod icons;
mod store;
mod vault;

use serde::Serialize;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use store::{Store, WindowGeometry};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder, Window, WindowEvent};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use vault::{EntryDetail, Listing, Vault};
use zeroize::Zeroizing;

const DEFAULT_WIDTH: f64 = 900.0;
const DEFAULT_HEIGHT: f64 = 600.0;
const DEFAULT_CLEAR_SECONDS: u64 = 20;

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
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    database: Option<String>,
    key_file: Option<String>,
    unlocked: bool,
}

#[tauri::command]
fn status(store: State<Store>, session: State<Session>) -> Status {
    let shown = |p: &Option<PathBuf>| p.as_ref().map(|p| p.display().to_string());
    let (database, key_file) = store.read(|s| (shown(&s.database), shown(&s.key_file)));
    Status { database, key_file, unlocked: session.0.lock().unwrap().is_some() }
}

fn pick(window: &Window, name: &str, extensions: &[&str]) -> Result<Option<PathBuf>, String> {
    let mut dialog = window.dialog().file().set_parent(window);
    if !extensions.is_empty() {
        dialog = dialog.add_filter(name, extensions);
    }
    dialog.blocking_pick_file().map(|p| p.into_path().map_err(|e| e.to_string())).transpose()
}

#[tauri::command(async)]
fn pick_database(window: Window, store: State<Store>, session: State<Session>) -> Result<Status, String> {
    if let Some(path) = pick(&window, "KeePass database", &["kdbx"])? {
        store.update(|s| s.database = Some(path)).map_err(|e| e.to_string())?;
    }
    Ok(status(store, session))
}

#[tauri::command(async)]
fn pick_key_file(window: Window, store: State<Store>, session: State<Session>) -> Result<Status, String> {
    if let Some(path) = pick(&window, "Key file", &[])? {
        store.update(|s| s.key_file = Some(path)).map_err(|e| e.to_string())?;
    }
    Ok(status(store, session))
}

#[tauri::command(async)]
fn clear_key_file(store: State<Store>, session: State<Session>) -> Result<Status, String> {
    store.update(|s| s.key_file = None).map_err(|e| e.to_string())?;
    Ok(status(store, session))
}

#[tauri::command(async)]
fn unlock(app: AppHandle, store: State<Store>, session: State<Session>, password: String) -> Result<Listing, String> {
    let password = Zeroizing::new(password);
    let (database, key_file) = store.read(|s| (s.database.clone(), s.key_file.clone()));
    let database = database.ok_or("Choose a database first")?;
    let password = (!password.is_empty()).then_some(password.as_str());
    let vault = Vault::open(&database, password, key_file.as_deref())?;
    let listing = vault.listing();
    if setting_bool(&store, "downloadIcons", true) {
        fetch_icons(app, vault.hosts(), icons::cache_dir(store.dir()));
    }
    *session.0.lock().unwrap() = Some(vault);
    Ok(listing)
}

/// Fetches missing site icons in the background and tells the window about
/// each one that arrives.
fn fetch_icons(app: AppHandle, hosts: Vec<String>, dir: PathBuf) {
    std::thread::spawn(move || {
        icons::Cache::new(dir).fetch_missing(hosts, |host| {
            let _ = app.emit("icon-ready", host);
        });
    });
}

#[tauri::command]
fn lock(session: State<Session>) {
    *session.0.lock().unwrap() = None;
    clipboard::clear_if_ours();
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
    let seconds = store.read(|s| s.settings.get("clearClipboard").and_then(Value::as_u64)).unwrap_or(DEFAULT_CLEAR_SECONDS);
    clipboard::copy(value, Duration::from_secs(seconds))?;
    Ok(seconds)
}

/// Opens the entry's URL in the default browser (web addresses only).
#[tauri::command(async)]
fn open_url(app: AppHandle, session: State<Session>, id: String) -> Result<(), String> {
    let url = session.with(|v| v.field(&id, "URL"))?;
    icons::host_of(&url).ok_or("The entry has no web address")?;
    let url = if url.contains("://") { url.to_string() } else { format!("https://{}", url.trim()) };
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

/// A cached site icon as a `data:` URL, or nothing yet.
#[tauri::command(async)]
fn icon(store: State<Store>, host: String) -> Option<String> {
    icons::Cache::new(icons::cache_dir(store.dir())).get(&host).and_then(|bytes| icons::data_url(&bytes))
}

#[tauri::command]
fn get_settings(store: State<Store>) -> Map<String, Value> {
    store.read(|s| s.settings.clone())
}

#[tauri::command(async)]
fn update_settings(store: State<Store>, changes: Map<String, Value>) -> Result<(), String> {
    store.update(|s| s.settings.extend(changes)).map_err(|e| e.to_string())
}

fn setting_bool(store: &Store, name: &str, default: bool) -> bool {
    store.read(|s| s.settings.get(name).and_then(Value::as_bool)).unwrap_or(default)
}

fn open_window(app: &AppHandle) -> tauri::Result<()> {
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
    window.show()
}

/// A saved position can point at a monitor that is no longer connected.
fn is_on_screen(window: &tauri::WebviewWindow) -> tauri::Result<bool> {
    let pos = window.outer_position()?;
    Ok(window.monitor_from_point(pos.x.into(), pos.y.into())?.is_some())
}

fn remember_geometry(window: &Window) -> tauri::Result<()> {
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
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(Session::default())
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
            get_settings,
            update_settings,
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { .. } = event {
                let _ = remember_geometry(window);
                clipboard::clear_if_ours();
            }
        })
        .setup(|app| {
            app.manage(Store::load(data_dir::resolve_state_file(None::<&Path>)));
            open_window(app.handle())?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running PswManager");
}
