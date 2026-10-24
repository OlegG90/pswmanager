//! Auto-Type's chooser (#307): a small window of its own, over the window the
//! hotkey was pressed in, where the user unlocks and picks the entry. The main
//! window is left as it is, so it never covers the window typed into.

use crate::settings::Settings;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub const LABEL: &str = "chooser";
const WIDTH: f64 = 480.0;
const HEIGHT: f64 = 460.0;

fn chooser(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(LABEL)
}

/// Brings the chooser up over `over` (a window's left, top, right and bottom),
/// to ask what waits for the hotkey's window. It is made the first time and
/// kept, hidden, after; a page loading asks by itself, one loaded is told.
pub fn ask(app: &AppHandle, over: Option<(i32, i32, i32, i32)>) {
    let (window, made) = match chooser(app) {
        Some(window) => (window, false),
        None => {
            let built = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("chooser.html".into()))
                .theme(crate::window::window_theme(&Settings::of(&app.state())))
                .title("PswManager — Auto-Type")
                .inner_size(WIDTH, HEIGHT)
                .min_inner_size(360.0, 320.0)
                // Above the window it is over, which may take the focus back; it goes when answered.
                .always_on_top(true)
                .skip_taskbar(true)
                .visible(false)
                .build();
            match built {
                Ok(window) => (window, true),
                Err(e) => return eprintln!("Cannot open Auto-Type's chooser: {e}"),
            }
        }
    };
    if let Some(bounds) = over {
        place(&window, bounds);
    }
    show(app);
    if !made {
        let _ = app.emit_to(LABEL, "auto-type-waiting", ());
    }
}

/// Centres the chooser over `(left, top, right, bottom)`, kept on that window's
/// screen. Its size is the one it takes there: a screen of another scale
/// resizes it as it arrives.
fn place(window: &WebviewWindow, (left, top, right, bottom): (i32, i32, i32, i32)) {
    let (Ok(size), Ok(scale)) = (window.outer_size(), window.scale_factor()) else { return };
    let (cx, cy) = ((left + right) / 2, (top + bottom) / 2);
    let monitor = window.monitor_from_point(cx.into(), cy.into()).ok().flatten();
    let rescale = monitor.as_ref().map_or(1.0, |m| m.scale_factor() / scale);
    let (width, height) = ((size.width as f64 * rescale) as i32, (size.height as f64 * rescale) as i32);
    let (mut x, mut y) = (cx - width / 2, cy - height / 2);
    if let Some(monitor) = monitor {
        let area = monitor.work_area();
        x = x.clamp(area.position.x, (area.position.x + area.size.width as i32 - width).max(area.position.x));
        y = y.clamp(area.position.y, (area.position.y + area.size.height as i32 - height).max(area.position.y));
    }
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

/// Shows the chooser as it is: after typing failed, to say why.
pub fn show(app: &AppHandle) {
    let Some(window) = chooser(app) else { return };
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
}

/// Runs `prompt`, which opens Windows Hello's prompt, with the chooser no
/// longer on top: the prompt is a window of its own, which the chooser would cover.
pub fn below<T>(app: &AppHandle, prompt: impl FnOnce() -> T) -> T {
    let window = chooser(app);
    if let Some(window) = &window {
        let _ = window.set_always_on_top(false);
    }
    let answer = prompt();
    if let Some(window) = &window {
        let _ = window.set_always_on_top(true);
    }
    answer
}

pub fn hide(app: &AppHandle) {
    if let Some(window) = chooser(app) {
        let _ = window.hide();
    }
}

/// Applies the theme setting, as the main window does.
pub fn apply_theme(app: &AppHandle) {
    if let Some(window) = chooser(app) {
        let _ = window.set_theme(crate::window::window_theme(&Settings::of(&app.state())));
    }
}
