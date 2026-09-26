//! The one app window: created hidden or shown, hidden to the tray instead of
//! closed, and brought back by the tray, the hotkey or a second launch.

use crate::settings::Settings;
use crate::store::{Store, WindowGeometry};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

const LABEL: &str = "main";
const DEFAULT_WIDTH: f64 = 900.0;
const DEFAULT_HEIGHT: f64 = 600.0;

/// A window that starts hidden is maximized when first shown: maximizing a
/// hidden window can show it.
static MAXIMIZE_ON_SHOW: AtomicBool = AtomicBool::new(false);

pub fn open(app: &AppHandle, visible: bool) -> tauri::Result<()> {
    let geometry = app.state::<Store>().read(|s| s.window.clone());
    let mut builder = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::default())
        .title("PswManager")
        .min_inner_size(520.0, 360.0)
        .visible(false);
    builder = match &geometry {
        Some(g) => builder.inner_size(g.width, g.height).position(g.x, g.y).maximized(g.maximized && visible),
        None => builder.inner_size(DEFAULT_WIDTH, DEFAULT_HEIGHT).center(),
    };
    let window = builder.build()?;
    if geometry.is_some() && !is_on_screen(&window)? {
        window.center()?;
    }
    if visible {
        window.show()?;
    } else {
        MAXIMIZE_ON_SHOW.store(geometry.is_some_and(|g| g.maximized), Ordering::Relaxed);
    }
    Ok(())
}

fn main_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(LABEL)
}

pub fn show(app: &AppHandle) {
    let Some(window) = main_window(app) else { return };
    let _ = window.show();
    if MAXIMIZE_ON_SHOW.swap(false, Ordering::Relaxed) {
        let _ = window.maximize();
    }
    let _ = window.unminimize();
    let _ = window.set_focus();
    let _ = app.emit("window-shown", ());
}

/// Hides the window to the tray, remembering where it was.
pub fn hide(app: &AppHandle) {
    let Some(window) = main_window(app) else { return };
    remember_geometry(&window);
    let _ = window.hide();
    if Settings::of(&app.state()).lock_when_hidden() {
        crate::lock_now(app);
    }
}

/// The hotkey: show the window, or hide it when it is already in front.
pub fn toggle(app: &AppHandle) {
    let Some(window) = main_window(app) else { return };
    if is_shown(&window) && window.is_focused().unwrap_or(false) {
        hide(app);
    } else {
        show(app);
    }
}

/// A tray click: clicking the icon takes the focus from the window, so a shown
/// window is hidden whether it had the focus or not.
pub fn toggle_from_tray(app: &AppHandle) {
    let Some(window) = main_window(app) else { return };
    if is_shown(&window) {
        hide(app);
    } else {
        show(app);
    }
}

fn is_shown(window: &WebviewWindow) -> bool {
    window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(false)
}

pub fn quit(app: &AppHandle) {
    if let Some(window) = main_window(app) {
        remember_geometry(&window);
    }
    crate::lock_now(app);
    app.exit(0);
}

/// A saved position can point at a monitor that is no longer connected.
fn is_on_screen(window: &WebviewWindow) -> tauri::Result<bool> {
    let pos = window.outer_position()?;
    Ok(window.monitor_from_point(pos.x.into(), pos.y.into())?.is_some())
}

/// Saves where a shown window is; a hidden one has nothing new to say.
fn remember_geometry(window: &WebviewWindow) {
    if !window.is_visible().unwrap_or(false) {
        return;
    }
    let read = || -> tauri::Result<_> {
        let scale = window.scale_factor()?;
        let pos = window.outer_position()?.to_logical::<f64>(scale);
        let size = window.inner_size()?.to_logical::<f64>(scale);
        Ok((window.is_maximized()?, pos, size))
    };
    let Ok((maximized, pos, size)) = read() else { return };
    let _ = window.state::<Store>().update(|s| {
        // A maximized window keeps the size it will restore to.
        s.window = match (&s.window, maximized) {
            (Some(prev), true) => Some(WindowGeometry { maximized, ..prev.clone() }),
            _ => Some(WindowGeometry { x: pos.x, y: pos.y, width: size.width, height: size.height, maximized }),
        };
    });
}
