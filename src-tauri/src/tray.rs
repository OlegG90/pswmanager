//! The notification-area icon: left click shows or hides the window; the menu
//! has Show, Lock, Sync now, Start with Windows, Settings and Quit.

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};
use tauri_plugin_autostart::ManagerExt;

use crate::window;

/// The menu's "Start with Windows" check mark, kept in step when the
/// settings screen changes it.
struct AutostartItem(CheckMenuItem<Wry>);

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let enabled = app.autolaunch().is_enabled().unwrap_or(false);
    let autostart = CheckMenuItem::with_id(app, "autostart", "Start with Windows", true, enabled, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "show", "Show", true, None::<&str>)?,
            &MenuItem::with_id(app, "lock", "Lock", true, None::<&str>)?,
            &MenuItem::with_id(app, "sync", "Sync now", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &autostart,
            &MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?,
        ],
    )?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("PswManager")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "show" => window::show(app),
            "lock" => crate::lock_now(app),
            "sync" => crate::sync::request(app),
            "autostart" => {
                let _ = set_autostart(app, !autostart_enabled(app));
                // An open settings screen shows the new state.
                let _ = app.emit("settings-changed", ());
            }
            "settings" => {
                window::show(app);
                let _ = app.emit("open-settings", ());
            }
            "quit" => window::quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                window::toggle(tray.app_handle(), false);
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    app.manage(AutostartItem(autostart));
    Ok(())
}

pub fn autostart_enabled(app: &AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

/// Turns "Start with Windows" on or off, then shows what the registry now
/// says, so a failed change does not leave a wrong check mark.
pub fn set_autostart(app: &AppHandle, on: bool) -> Result<(), String> {
    let manager = app.autolaunch();
    let changed = if on { manager.enable() } else { manager.disable() };
    if let Some(item) = app.try_state::<AutostartItem>() {
        let _ = item.0.set_checked(autostart_enabled(app));
    }
    changed.map_err(|e| format!("Could not change Start with Windows: {e}"))
}
