// The database, merge, sync and stores live in the shared core.
use pswm_core::{qr, gauth, icons, opened, settings, store, updates};
use pswm_core::session::{KeyNeeded, Session};

mod activity;
mod app_settings;
mod auto_type_flow;
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
use app_settings::register_hotkeys;
use auto_type_flow::cancel_auto_type;
use backups::start_backup_clock;
use databases::{lock_now, open_from_command_line};
use exchange::{ImportedFile, WaitingFile};
use settings::Settings;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use store::Store;
use tauri::{AppHandle, Emitter, Manager, State, Window, WindowEvent};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use zeroize::Zeroizing;

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

/// A QR code's text for `from_clipboard` / `from_file` to take: from the
/// picture on the clipboard (when `new` says it is new), or else a picture
/// the user picks. Nothing when they cancel.
fn read_qr(
    window: &Window,
    from_clipboard: qr::FromPixels,
    from_file: qr::FromFile,
    new: impl Fn(&str) -> bool,
) -> Result<Option<Zeroizing<String>>, String> {
    if let Some(text) = clipboard::qr(from_clipboard).filter(|text| new(text)) {
        return Ok(Some(text));
    }
    let Some(path) = pick(window, "Pictures", &["png", "jpg", "jpeg"])? else { return Ok(None) };
    let content = read_picked(&path, qr::check_size)?;
    from_file(&content).map(Some)
}

/// A file the user picked, read whole; `check_size` refuses one too big
/// before it is read.
fn read_picked(path: &std::path::Path, check_size: fn(u64) -> Result<(), String>) -> Result<Zeroizing<Vec<u8>>, String> {
    let unreadable = |e: std::io::Error| format!("Cannot read the file: {e}");
    check_size(std::fs::metadata(path).map_err(unreadable)?.len())?;
    std::fs::read(path).map(Zeroizing::new).map_err(unreadable)
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
            app_settings::settings,
            app_settings::set_setting,
            entries::password_health,
            entries::similar_entries,
            entries::merge_entries,
            entries::merge_preview,
            auto_type_flow::auto_type_waiting,
            auto_type_flow::auto_type,
            auto_type_flow::auto_type_entry,
            auto_type_flow::cancel_auto_type,
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
