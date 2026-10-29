//! The app's settings as the window shows and changes them, and the global
//! hotkeys they set.
use crate::auto_type_flow::auto_type_pressed;
use crate::{browser_host, hello, tray, window, Notice};
use pswm_core::settings::{self, Settings};
use pswm_core::store::Store;
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

/// The settings screen's values.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SettingsView {
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
pub fn settings(app: AppHandle) -> SettingsView {
    settings_view(&app)
}

/// Changes one setting; it applies at once, since every use reads it anew.
#[tauri::command(async)]
pub fn set_setting(app: AppHandle, name: String, value: serde_json::Value) -> Result<SettingsView, String> {
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
pub(crate) fn register_hotkeys(app: &AppHandle) {
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
