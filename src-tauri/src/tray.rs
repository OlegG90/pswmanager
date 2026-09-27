//! The notification-area icon: left click shows or hides the window; the menu
//! has Show, Lock, Sync now, Start with Windows and Quit.

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::AppHandle;
use tauri_plugin_autostart::ManagerExt;

use crate::window;

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
            "autostart" => set_autostart(app, &autostart),
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
    Ok(())
}

/// Flips "Start with Windows", then shows what the registry now says, so a
/// failed change does not leave a wrong check mark.
fn set_autostart(app: &AppHandle, item: &CheckMenuItem<tauri::Wry>) {
    let manager = app.autolaunch();
    let enabled = manager.is_enabled().unwrap_or(false);
    let _ = if enabled { manager.disable() } else { manager.enable() };
    let _ = item.set_checked(manager.is_enabled().unwrap_or(enabled));
}
