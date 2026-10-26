//! Auto-Type's chooser (#307): a small window of its own, over the window the
//! hotkey was pressed in, where the user unlocks and picks the entry. The main
//! window is left as it is, so it never covers the window typed into.

use crate::settings::Settings;
use pswm_core::vault::EntryChoice;
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

/// What the chooser shows for the hotkey's window: the decision behind the
/// wait, as pure data (windows and locks stay out), so tests read the same
/// table the window follows.
#[derive(Debug, PartialEq)]
pub enum Plan {
    /// One entry alone fits and may be typed at once: its id.
    Type { id: String },
    /// The choice to show.
    Ask,
    /// Why the window was not typed into, said once.
    Notice,
    /// Locked: the chooser unlocks first (nothing waits meanwhile).
    Unlock,
    /// No window waits.
    Nothing,
}

/// What waits for the hotkey's window: a notice still standing is said first,
/// as `auto_type_waiting` says it; then nothing without a window, the unlock
/// without it, one fitting entry typed at once unless asked, else the choice.
pub fn plan(has_target: bool, notice: Option<&str>, unlocked: bool, may_type: bool, always_ask: bool, matches: &[EntryChoice]) -> Plan {
    if notice.is_some() {
        return Plan::Notice;
    }
    if !has_target {
        return Plan::Nothing;
    }
    if !unlocked {
        return Plan::Unlock;
    }
    if let [only] = matches {
        if may_type && !always_ask {
            return Plan::Type { id: only.id.clone() };
        }
    }
    Plan::Ask
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(id: &str) -> EntryChoice {
        EntryChoice { id: id.into(), title: "Mail".into(), username: "user".into() }
    }

    fn planned(has_target: bool, notice: Option<&str>, unlocked: bool, may_type: bool, always_ask: bool, ids: &[&str]) -> Plan {
        let matches: Vec<EntryChoice> = ids.iter().map(|id| choice(id)).collect();
        plan(has_target, notice, unlocked, may_type, always_ask, &matches)
    }

    #[test]
    fn a_standing_notice_is_said_first() {
        assert_eq!(planned(true, Some("gone"), true, true, false, &["a"]), Plan::Notice);
    }

    #[test]
    fn with_no_window_nothing_waits() {
        assert_eq!(planned(false, None, true, true, false, &[]), Plan::Nothing);
    }

    #[test]
    fn locked_unlocks_first() {
        assert_eq!(planned(true, None, false, true, false, &["a"]), Plan::Unlock);
    }

    #[test]
    fn one_fitting_entry_types_at_once() {
        assert_eq!(planned(true, None, true, true, false, &["a"]), Plan::Type { id: "a".into() });
    }

    #[test]
    fn asked_when_set_or_several_fit() {
        assert_eq!(planned(true, None, true, true, true, &["a"]), Plan::Ask, "always ask");
        assert_eq!(planned(true, None, true, false, false, &["a"]), Plan::Ask, "only looking");
        assert_eq!(planned(true, None, true, true, false, &["a", "b"]), Plan::Ask, "several fit");
        assert_eq!(planned(true, None, true, true, false, &[]), Plan::Ask, "none fits: search");
    }
}
