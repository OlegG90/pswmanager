// The database, merge, sync and stores live in the shared core.
use pswm_core::{gauth, icons, opened, settings, store, updates, vault};
use pswm_core::autotype::Typed;
use pswm_core::session::{KeyNeeded, Session};

mod activity;
mod autotype;
mod backups;
mod browser;
mod browser_host;
mod chooser;
mod cli;
mod clipboard;
mod credentials;
mod data_dir;
mod databases;
mod entries;
mod exchange;
mod file_watch;
mod hello;
mod native_messaging;
mod questions;
mod session_watch;
mod sync;
mod syncing;
mod tray;
mod window;

use activity::Activity;
use backups::start_backup_clock;
use databases::{lock_now, open_from_command_line};
use exchange::{ImportedFile, WaitingFile};
use serde::Serialize;
use settings::Settings;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use store::Store;
use tauri::{AppHandle, Emitter, Manager, State, Window, WindowEvent};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use tauri_plugin_opener::OpenerExt;

/// How often the inactivity check runs.
const CHECK_EVERY: Duration = Duration::from_secs(10);

/// Changes which copies need a key, while the database is unlocked, and tells
/// the window (`key-needed`) when that changed or a copy needs one (so the
/// window may ask again what it could not ask before).
fn need_key(app: &AppHandle, change: impl FnOnce(&mut KeyNeeded)) {
    let Some((before, now)) = app.state::<Session>().change_key_needed(change) else { return };
    if now != before || now != KeyNeeded::default() {
        let _ = app.emit("key-needed", now);
    }
}

/// Watches the open database's file; dropping the watcher stops it.
#[derive(Default)]
struct FileWatch(Mutex<Option<notify::RecommendedWatcher>>);

/// The window Auto-Type types into while the user unlocks or chooses the
/// entry (#272).
#[derive(Default)]
struct AutoTypeTarget(Mutex<Option<autotype::Target>>);

/// Why the hotkey's window was not typed into, for the chooser to say once it
/// is up (#307).
#[derive(Default)]
struct AutoTypeNotice(Mutex<Option<String>>);

/// Something the user should know that happened before the window could say
/// it (the global hotkey could not be registered).
#[derive(Default)]
struct Notice(OnceLock<String>);

fn pick(window: &Window, name: &str, extensions: &[&str]) -> Result<Option<PathBuf>, String> {
    let mut dialog = window.dialog().file().set_parent(window);
    if !extensions.is_empty() {
        dialog = dialog.add_filter(name, extensions);
    }
    dialog.blocking_pick_file().map(|p| p.into_path().map_err(|e| e.to_string())).transpose()
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

/// Where PswManager's source lives (the workspace's `repository`): opened from
/// Settings → About. Fixed at build time, so the window never asks to open an
/// address of its own.
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

/// Opens the app's repository in the default browser.
#[tauri::command(async)]
fn open_repository(app: AppHandle) -> Result<(), String> {
    app.opener().open_url(REPOSITORY, None::<&str>).map_err(|e| e.to_string())
}

/// Asks GitHub for the latest release and compares it with this copy.
#[tauri::command(async)]
fn check_for_update() -> Result<updates::UpdateCheck, String> {
    updates::check(REPOSITORY, env!("CARGO_PKG_VERSION"))
}

/// Opens the latest release's page, where its files are, in the default browser.
#[tauri::command(async)]
fn open_latest_release(app: AppHandle) -> Result<(), String> {
    app.opener().open_url(format!("{REPOSITORY}/releases/latest"), None::<&str>).map_err(|e| e.to_string())
}

/// The settings screen's values.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsView {
    #[serde(flatten)]
    settings: settings::View,
    start_with_windows: bool,
    /// Windows Hello is set up on this PC: unlocking with it is offered.
    windows_hello: bool,
    /// Another program (KeePassXC) is registered for the browser extension.
    other_browser_host: bool,
    /// The integration is on, but another program holds the extension's pipe.
    browser_unserved: bool,
}

fn settings_view(app: &AppHandle) -> SettingsView {
    SettingsView {
        settings: Settings::of(&app.state()).view(),
        start_with_windows: tray::autostart_enabled(app),
        windows_hello: hello::available(),
        other_browser_host: browser_host::other_registered(app),
        browser_unserved: browser_host::unserved(app),
    }
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
    } else if let Some(hotkey) = Hotkey::named(&name) {
        change_hotkey(&app, hotkey, value.as_str().ok_or("A hotkey is a key combination")?)?;
    } else {
        let off = value == false;
        Settings::of(&app.state()).set(&name, value)?;
        if name == "browserIntegration" {
            if let Err(message) = browser_host::apply(&app) {
                Settings::of(&app.state()).set(&name, false.into())?;
                return Err(message);
            }
        } else if name == "theme" {
            window::apply_theme(&app);
        } else if name == "biometricUnlock" && off {
            hello::forget_all();
        }
    }
    Ok(settings_view(&app))
}

/// Auto-Type's choice, when the window asks which entry to type.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AutoTypeAsk {
    /// The title of the window typed into.
    window: String,
    /// A browser's site, when it could be read.
    site: Option<String>,
    browser: bool,
    /// The entries that fit it.
    matches: Vec<vault::EntryChoice>,
}

/// Auto-Type's hotkey (#272): the window in front waits to be typed into. One
/// entry alone fits it: typed at once (unless the user wants to be asked).
/// Otherwise, or while locked, the chooser comes up over it to unlock or
/// choose (#307); the main window stays as it is.
fn auto_type_pressed(app: &AppHandle) {
    // A press while one is typing (the hotkey held, pressed twice) types nothing more.
    let Some(_typing) = Typing::start() else { return };
    let Some(target) = autotype::foreground() else {
        // Pressed in the main window: said so, rather than nothing happening
        // (in the chooser, which asks already, nothing needs saying).
        if window::is_focused(app) {
            let _ = app.emit("notice", "Auto-Type types into another window: press the hotkey there, or use the entry's Auto-Type (Ctrl+Shift+V)");
        }
        return; // the desktop, the taskbar
    };
    let over = target.bounds();
    *app.state::<AutoTypeTarget>().0.lock().unwrap() = Some(target);
    *app.state::<AutoTypeNotice>().0.lock().unwrap() = None;
    if app.state::<Session>().is_unlocked() {
        match waiting_window(app, true) {
            // Typed: a chooser still up from an earlier press has nothing to ask now.
            Ok(None) => return chooser::hide(app),
            Ok(Some(_)) => {}
            Err(message) => *app.state::<AutoTypeNotice>().0.lock().unwrap() = Some(message),
        }
    }
    // Once up (and unlocked), the chooser asks for what waits (auto_type_waiting).
    chooser::ask(app, over);
}

/// What the chooser shows once it is up and unlocked: why the hotkey's window
/// was not typed into, or the choice for it; nothing when none waits. Asked
/// after every unlock: the user may have unlocked for something else.
#[tauri::command(async)]
fn auto_type_waiting(app: AppHandle) -> Result<Option<AutoTypeAsk>, String> {
    if let Some(message) = app.state::<AutoTypeNotice>().0.lock().unwrap().take() {
        return Err(message);
    }
    waiting_window(&app, false)
}

/// What the window waiting to be typed into needs: nothing when there is none,
/// or when `may_type` and one entry alone fits it (typed now, unless the user
/// wants to be asked); else the choice to show, and the window waits on. Taken
/// while deciding, so two calls never both type.
fn waiting_window(app: &AppHandle, may_type: bool) -> Result<Option<AutoTypeAsk>, String> {
    let Some(target) = take_waiting(app) else { return Ok(None) };
    let matches = app.state::<Session>().read(|v| v.auto_type_matches(&target.window))?;
    let always_ask = Settings::of(&app.state()).auto_type_always_ask();
    if let chooser::Plan::Type { id } = chooser::plan(may_type, always_ask, &matches) {
        return type_entry(app, &target, &id, Typed::Sequence).map(|()| None);
    }
    let site = target.window.page.as_ref().map(pswm_core::browser::Page::place);
    let ask = AutoTypeAsk { window: target.window.title.clone(), site, browser: target.window.browser, matches };
    // A newer press meanwhile is the one that waits. Locked meanwhile, none
    // does: checked under the lock lock_now takes the window with.
    let waiting_target = app.state::<AutoTypeTarget>();
    let mut waiting = waiting_target.0.lock().unwrap();
    if !app.state::<Session>().is_unlocked() {
        return Ok(None);
    }
    waiting.get_or_insert(target);
    Ok(Some(ask))
}

/// Auto-Type typing now: one at a time, so keys of two are never mixed.
struct Typing;

static TYPING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

impl Typing {
    /// The turn to type; `None` while another typing runs.
    fn start() -> Option<Typing> {
        (!TYPING.swap(true, std::sync::atomic::Ordering::SeqCst)).then_some(Typing)
    }
}

impl Drop for Typing {
    fn drop(&mut self) {
        TYPING.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Types the entry the user chose in the chooser into the window waiting for
/// it. The chooser steps aside first and comes back when typing fails, to say why.
#[tauri::command(async)]
fn auto_type(app: AppHandle, id: String, typed: Typed) -> Result<(), String> {
    let _typing = Typing::start().ok_or("Auto-Type is typing already")?;
    let target = take_waiting(&app).ok_or("The window to type into is gone: press the Auto-Type hotkey there again")?;
    chooser::hide(&app);
    let done = type_entry(&app, &target, &id, typed);
    match done {
        Ok(()) => chooser_gone(&app),
        Err(_) => chooser::show(&app),
    }
    done
}

/// The user chose no entry: nothing waits to be typed into, and the chooser goes.
#[tauri::command(async)]
fn cancel_auto_type(app: AppHandle) {
    take_waiting(&app);
    app.state::<AutoTypeNotice>().0.lock().unwrap().take();
    chooser::hide(&app);
    chooser_gone(&app);
}

/// The chooser has gone. With the main window hidden, PswManager is put away:
/// it locks or syncs as hiding the main window does.
fn chooser_gone(app: &AppHandle) {
    if !window::is_shown(app) {
        window::settle(app);
    }
}

/// The window waiting to be typed into, if it still is, taken: no other call gets it.
fn take_waiting(app: &AppHandle) -> Option<autotype::Target> {
    app.state::<AutoTypeTarget>().0.lock().unwrap().take().filter(autotype::Target::is_current)
}

/// Entry-level Auto-Type: the window steps aside, and the entry is typed into
/// the window that comes forward then (the one in front before PswManager).
#[tauri::command(async)]
fn auto_type_entry(app: AppHandle, id: String, typed: Typed) -> Result<(), String> {
    let _typing = Typing::start().ok_or("Auto-Type is typing already")?;
    // An entry that cannot be typed (Auto-Type off, no TOTP secret) says so
    // before the window steps aside.
    app.state::<Session>().read(|v| v.auto_type_steps(&id, "", typed))??;
    if !window::step_aside(&app) {
        return Err("Auto-Type types from the window".into());
    }
    let done = autotype::behind().ok_or_else(|| "There is no window behind PswManager to type into".to_string()).and_then(|target| type_entry(&app, &target, &id, typed));
    window::come_back(&app, done.is_ok());
    done
}

/// Types `typed` of entry `id` into `target`.
fn type_entry(app: &AppHandle, target: &autotype::Target, id: &str, typed: Typed) -> Result<(), String> {
    let steps = app.state::<Session>().read(|v| v.auto_type_steps(id, &target.window.title, typed))??;
    // What the window shows now may refuse the typing: an unreadable
    // browser's address, or a page gone on since.
    if let Some(refusal) = pswm_core::autotype::refusal(&target.window, autotype::page_now(target).as_ref()) {
        return Err(refusal);
    }
    if autotype::runs_higher(target) {
        return Err(format!("\"{}\" runs as administrator: Auto-Type cannot type into it unless PswManager does too", target.window.title));
    }
    autotype::type_into(target, &steps, || app.state::<Session>().is_unlocked())
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

/// The app's global hotkeys.
#[derive(Clone, Copy, PartialEq)]
enum Hotkey {
    /// Shows or hides the window.
    Window,
    /// Auto-Type into the window in front (#272).
    AutoType,
}

impl Hotkey {
    const ALL: [Hotkey; 2] = [Hotkey::Window, Hotkey::AutoType];

    /// The setting it is kept in.
    fn setting(self) -> &'static str {
        match self {
            Hotkey::Window => "hotkey",
            Hotkey::AutoType => "autoTypeHotkey",
        }
    }

    fn named(setting: &str) -> Option<Hotkey> {
        Hotkey::ALL.into_iter().find(|h| h.setting() == setting)
    }

    /// The combination set; empty when it is off.
    fn combination(self, app: &AppHandle) -> String {
        let store = app.state::<Store>();
        let settings = Settings::of(&store);
        match self {
            Hotkey::Window => settings.hotkey(),
            Hotkey::AutoType => settings.auto_type_hotkey(),
        }
    }
}

/// Registers the hotkeys from the settings. A key taken by another app is
/// reported on the unlock screen instead of failing the start.
fn register_hotkeys(app: &AppHandle) {
    let mut taken = Vec::new();
    for hotkey in Hotkey::ALL {
        let combination = hotkey.combination(app);
        if combination.is_empty() {
            continue;
        }
        if let Err(e) = listen_to(app, hotkey, &combination) {
            taken.push(format!("The hotkey {combination} is not available: {e}"));
        }
    }
    if !taken.is_empty() {
        let _ = app.state::<Notice>().0.set(taken.join("\n"));
    }
}

/// Does what `hotkey` does on `combination`.
fn listen_to(app: &AppHandle, hotkey: Hotkey, combination: &str) -> Result<(), String> {
    let shortcut: Shortcut = combination.parse().map_err(|e| format!("{combination} is not a key combination: {e}"))?;
    app.global_shortcut()
        .on_shortcut(shortcut, move |app, _, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            match hotkey {
                Hotkey::Window => window::toggle(app, true),
                // Off the event loop: reading the window and typing take a while.
                Hotkey::AutoType => {
                    let app = app.clone();
                    std::thread::spawn(move || auto_type_pressed(&app));
                }
            }
        })
        .map_err(|e| e.to_string())
}

/// Moves `hotkey` to `combination` (empty turns Auto-Type's off), kept only
/// once it is registered: a combination another app or the other hotkey holds
/// is refused and the old one stays.
fn change_hotkey(app: &AppHandle, hotkey: Hotkey, combination: &str) -> Result<(), String> {
    let old = hotkey.combination(app);
    let same = |a: &str, b: &str| a.parse::<Shortcut>().ok().zip(b.parse::<Shortcut>().ok()).is_some_and(|(a, b)| a == b);
    if combination.is_empty() {
        if hotkey == Hotkey::Window {
            return Err("The window's hotkey cannot be turned off".into());
        }
        let _ = app.global_shortcut().unregister(old.as_str());
    } else if !same(&old, combination) {
        let shortcut = combination.parse::<Shortcut>().map_err(|_| format!("\"{combination}\" is not a key combination"))?;
        if !shortcut.mods.intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER) {
            return Err("A hotkey needs Ctrl, Alt or Win, so typing does not trigger it".into());
        }
        if Hotkey::ALL.into_iter().any(|other| other != hotkey && same(&other.combination(app), combination)) {
            return Err(format!("{combination} is the other hotkey already"));
        }
        listen_to(app, hotkey, combination).map_err(|e| format!("{combination} is not available — another app may use it ({e})"))?;
        if !old.is_empty() {
            let _ = app.global_shortcut().unregister(old.as_str());
        }
    }
    Settings::of(&app.state()).set(hotkey.setting(), combination.into())
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
    // Started by a browser for the KeePassXC-Browser extension: a proxy to the running app.
    if native_messaging::started_by_browser(&std::env::args().skip(1).collect::<Vec<_>>()) {
        return native_messaging::run_proxy();
    }
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
    pswm_core::secrets::install(Box::new(credentials::CredentialManager));
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
        .manage(ImportedFile::default())
        .manage(WaitingFile::default())
        .manage(gauth::Session::default())
        .manage(AutoTypeTarget::default())
        .manage(AutoTypeNotice::default())
        .manage(questions::Questions::default())
        .manage(sync::Syncer::default())
        .invoke_handler(tauri::generate_handler![
            databases::status,
            databases::pick_database,
            syncing::sync_with_folder,
            syncing::link_database,
            syncing::upload_to_folder,
            syncing::pick_remote_file,
            databases::pick_new_file,
            databases::pick_key_file_path,
            databases::create_database,
            databases::select_database,
            databases::remove_database,
            syncing::sign_in_to_cloud,
            syncing::sync_with_cloud,
            syncing::cancel_cloud,
            syncing::stop_sync,
            syncing::sync_now,
            syncing::sync_status,
            databases::key_needed,
            backups::backup,
            backups::set_backup_every,
            backups::pick_backup_folder,
            backups::backup_now,
            backups::show_backup_folder,
            databases::pick_key_file,
            databases::clear_key_file,
            databases::unlock,
            databases::hello_ready,
            databases::unlock_with_hello,
            exchange::import_file,
            exchange::import_file_with_password,
            exchange::cancel_import,
            exchange::gauth_start,
            exchange::gauth_read,
            exchange::gauth_import,
            exchange::gauth_cancel,
            exchange::delete_imported_file,
            exchange::confirm_export,
            exchange::export_file,
            databases::lock,
            entries::listing,
            databases::database_settings,
            databases::set_database_setting,
            databases::rename_database_file,
            databases::history_limits_preview,
            databases::set_history_limits,
            databases::create_key_file,
            databases::change_master_key,
            databases::enter_other_key,
            databases::encryption_unlock_time,
            databases::set_encryption,
            entries::entry,
            entries::reveal,
            entries::entry_history,
            entries::entry_version,
            entries::restore_version,
            entries::copy_field,
            entries::open_url,
            open_repository,
            check_for_update,
            open_latest_release,
            entries::icon,
            touch,
            hide_window,
            entries::edit_entry,
            entries::save_entry,
            entries::delete_entries,
            entries::duplicate_entry,
            entries::restore_entries,
            entries::delete_for_good,
            entries::empty_trash,
            entries::set_tag,
            entries::rename_tag,
            entries::remove_tag,
            entries::save_attachment,
            entries::open_attachment,
            entries::pick_file_to_attach,
            entries::scan_totp_qr,
            entries::release_files,
            entries::pick_icon_image,
            entries::totp,
            entries::copy_totp,
            entries::generate_password,
            entries::password_strength,
            settings,
            set_setting,
            entries::password_health,
            entries::similar_entries,
            entries::merge_entries,
            entries::merge_preview,
            auto_type_waiting,
            auto_type,
            auto_type_entry,
            cancel_auto_type,
            browser_host::browser_answer,
            browser_host::browser_associations,
            browser_host::forget_browser_association,
        ])
        .on_window_event(|window, event| {
            // Closing the window only hides it; Quit is in the tray menu. Closing
            // the chooser is its Cancel.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if window.label() == chooser::LABEL {
                    // Off the event loop: it may lock, which waits for a save.
                    let app = window.app_handle().clone();
                    std::thread::spawn(move || cancel_auto_type(app));
                } else {
                    window::hide(window.app_handle());
                }
            }
        })
        .setup(move |app| {
            let store = Store::load(state_file);
            app.manage(icons::Cache::in_data_dir(store.dir()));
            app.manage(store);
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
            register_hotkeys(handle);
            if let Err(message) = browser_host::apply(handle) {
                let _ = handle.state::<Notice>().0.set(message);
            }
            watch_inactivity(handle.clone());
            lock_on_session_lock(handle.clone());
            sync::start_clock(handle.clone());
            start_backup_clock(handle.clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running PswManager");
}
