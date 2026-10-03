//! The user's settings, kept by the frontend in the state file: every name,
//! default and allowed range in one place, for both apps (each offers the
//! ones it has: the hotkey is the PC's, locking in the background the phone's).

use crate::store::Store;
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;

const HOTKEY: &str = "Ctrl+Alt+P";
/// Minutes without use before locking; 0 means never.
const LOCK_AFTER_MINUTES: (u64, u64, u64) = (5, 1, 60);
/// Minutes between checks of the remote file while unlocked; 0 means never.
const SYNC_EVERY_MINUTES: (u64, u64, u64) = (5, 1, 60);
/// Seconds before a copied value is cleared.
const CLEAR_SECONDS: (u64, u64, u64) = (20, 5, 120);
/// Light or dark, or as the system is set (`system`, the default).
const THEMES: [&str; 3] = ["system", "light", "dark"];
/// On the phone, seconds in the background before locking: at once, 30 s
/// (the default), 1 min, 5 min; `null` for never.
const LOCK_IN_BACKGROUND: [u64; 4] = [0, 30, 60, 300];
const LOCK_IN_BACKGROUND_DEFAULT: u64 = 30;
/// On the phone, days between asking for the master password when biometric
/// unlock is on.
const PASSWORD_EVERY_DAYS: (u64, u64, u64) = (14, 1, 90);

pub struct Settings<'a>(&'a Store);

impl<'a> Settings<'a> {
    pub fn of(store: &'a Store) -> Self {
        Settings(store)
    }

    fn get<T>(&self, name: &str, read: impl FnOnce(&Value) -> Option<T>) -> Option<T> {
        self.0.read(|s| s.settings.get(name).and_then(read))
    }

    pub fn hotkey(&self) -> String {
        self.get("hotkey", |v| v.as_str().map(str::to_string)).unwrap_or_else(|| HOTKEY.into())
    }

    pub fn lock_after(&self) -> Option<Duration> {
        lock_after(self.get("lockAfterMinutes", Value::as_u64))
    }

    pub fn lock_on_session_lock(&self) -> bool {
        self.get("lockOnSessionLock", Value::as_bool).unwrap_or(true)
    }

    pub fn lock_when_hidden(&self) -> bool {
        self.get("lockWhenHidden", Value::as_bool).unwrap_or(false)
    }

    /// On the phone: how long in the background before locking; `None` for never.
    pub fn lock_in_background(&self) -> Option<Duration> {
        match self.0.read(|s| s.settings.get("lockInBackground").cloned()) {
            Some(Value::Null) => None,
            Some(v) => Some(v.as_u64().filter(|s| LOCK_IN_BACKGROUND.contains(s)).unwrap_or(LOCK_IN_BACKGROUND_DEFAULT)),
            None => Some(LOCK_IN_BACKGROUND_DEFAULT),
        }
        .map(Duration::from_secs)
    }

    /// On the phone: lock when the screen turns off.
    pub fn lock_on_screen_off(&self) -> bool {
        self.get("lockOnScreenOff", Value::as_bool).unwrap_or(true)
    }

    /// On the phone: unlock with a fingerprint or face (offered on the unlock screen).
    pub fn biometric_unlock(&self) -> bool {
        self.get("biometricUnlock", Value::as_bool).unwrap_or(true)
    }

    /// On the phone: how often the master password is asked for anyway.
    pub fn password_every(&self) -> Duration {
        let (default, min, max) = PASSWORD_EVERY_DAYS;
        Duration::from_secs(self.get("passwordEveryDays", Value::as_u64).unwrap_or(default).clamp(min, max) * 24 * 60 * 60)
    }

    pub fn clear_clipboard_after(&self) -> Duration {
        clear_after(self.get("clearClipboard", Value::as_u64))
    }

    pub fn sync_every(&self) -> Option<Duration> {
        minutes(self.get("syncEveryMinutes", Value::as_u64), SYNC_EVERY_MINUTES)
    }

    pub fn download_icons(&self) -> bool {
        self.get("downloadIcons", Value::as_bool).unwrap_or(true)
    }

    /// `system`, `light` or `dark`.
    pub fn theme(&self) -> &'static str {
        let chosen = self.get("theme", |v| v.as_str().map(str::to_string));
        THEMES.into_iter().find(|t| chosen.as_deref() == Some(*t)).unwrap_or(THEMES[0])
    }

    /// Every setting as it is in effect, for the settings screen.
    pub fn view(&self) -> View {
        let in_minutes = |d: Option<Duration>| d.map_or(0, |d| d.as_secs() / 60);
        View {
            lock_after_minutes: in_minutes(self.lock_after()),
            lock_on_session_lock: self.lock_on_session_lock(),
            lock_when_hidden: self.lock_when_hidden(),
            clear_clipboard: self.clear_clipboard_after().as_secs(),
            sync_every_minutes: in_minutes(self.sync_every()),
            download_icons: self.download_icons(),
            theme: self.theme(),
            hotkey: self.hotkey(),
            lock_in_background: self.lock_in_background().map(|d| d.as_secs()),
            lock_on_screen_off: self.lock_on_screen_off(),
            biometric_unlock: self.biometric_unlock(),
            password_every_days: self.password_every().as_secs() / (24 * 60 * 60),
        }
    }

    /// Changes one setting from the settings screen. Only the settings the
    /// screen offers, with a value of the right kind within its limits.
    pub fn set(&self, name: &str, value: Value) -> Result<(), String> {
        check(name, &value)?;
        self.0
            .update(|s| {
                s.settings.insert(name.to_string(), value);
            })
            .map_err(|e| format!("Could not save the setting: {e}"))
    }
}

/// The settings screen's values; "Start with Windows" comes from the
/// registry instead. Each app shows the ones it has.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    /// 0 means never.
    pub lock_after_minutes: u64,
    pub lock_on_session_lock: bool,
    pub lock_when_hidden: bool,
    pub clear_clipboard: u64,
    /// 0 means never.
    pub sync_every_minutes: u64,
    pub download_icons: bool,
    pub theme: &'static str,
    pub hotkey: String,
    /// Seconds; `None` for never.
    pub lock_in_background: Option<u64>,
    pub lock_on_screen_off: bool,
    pub biometric_unlock: bool,
    pub password_every_days: u64,
}

fn check(name: &str, value: &Value) -> Result<(), String> {
    let within = |(_, min, max): (u64, u64, u64), never: bool| {
        value.as_u64().is_some_and(|v| (never && v == 0) || (min..=max).contains(&v))
    };
    let ok = match name {
        "lockAfterMinutes" => within(LOCK_AFTER_MINUTES, true),
        "syncEveryMinutes" => within(SYNC_EVERY_MINUTES, true),
        "clearClipboard" => within(CLEAR_SECONDS, false),
        "lockOnSessionLock" | "lockWhenHidden" | "downloadIcons" | "lockOnScreenOff" | "biometricUnlock" => value.is_boolean(),
        "passwordEveryDays" => within(PASSWORD_EVERY_DAYS, false),
        "lockInBackground" => value.is_null() || value.as_u64().is_some_and(|s| LOCK_IN_BACKGROUND.contains(&s)),
        "theme" => value.as_str().is_some_and(|t| THEMES.contains(&t)),
        // Checked and registered by the caller before it is kept.
        "hotkey" => value.as_str().is_some_and(|k| !k.trim().is_empty()),
        _ => return Err(format!("There is no setting {name}")),
    };
    if ok {
        Ok(())
    } else {
        Err(format!("{value} is not a valid value for {name}"))
    }
}

fn lock_after(value: Option<u64>) -> Option<Duration> {
    minutes(value, LOCK_AFTER_MINUTES)
}

/// A setting in minutes within its limits; 0 means never.
fn minutes(value: Option<u64>, (default, min, max): (u64, u64, u64)) -> Option<Duration> {
    match value.unwrap_or(default) {
        0 => None,
        m => Some(Duration::from_secs(m.clamp(min, max) * 60)),
    }
}

fn clear_after(seconds: Option<u64>) -> Duration {
    let (default, min, max) = CLEAR_SECONDS;
    Duration::from_secs(seconds.unwrap_or(default).clamp(min, max))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_timeout_follows_the_setting_within_limits() {
        assert_eq!(lock_after(None), Some(Duration::from_secs(5 * 60)));
        assert_eq!(lock_after(Some(0)), None);
        assert_eq!(lock_after(Some(15)), Some(Duration::from_secs(15 * 60)));
        assert_eq!(lock_after(Some(1000)), Some(Duration::from_secs(60 * 60)));
    }

    #[test]
    fn the_master_password_is_asked_for_every_14_days_or_as_set() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::load(dir.path().join("pswm.json"));
        let days = |s: &Store| Settings::of(s).password_every().as_secs() / 86400;
        assert_eq!(days(&store), 14);
        Settings::of(&store).set("passwordEveryDays", 30.into()).unwrap();
        assert_eq!(days(&store), 30);
        assert!(Settings::of(&store).set("passwordEveryDays", 0.into()).is_err());
        assert!(Settings::of(&store).set("passwordEveryDays", 91.into()).is_err());
        assert!(Settings::of(&store).biometric_unlock());
    }

    #[test]
    fn clipboard_timeout_stays_within_limits() {
        assert_eq!(clear_after(None), Duration::from_secs(20));
        assert_eq!(clear_after(Some(0)), Duration::from_secs(5));
        assert_eq!(clear_after(Some(u64::MAX)), Duration::from_secs(120));
    }

    #[test]
    fn missing_or_mistyped_settings_fall_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::load(dir.path().join("pswm.json"));
        store
            .update(|s| {
                s.settings.insert("lockWhenHidden".into(), "yes".into());
                s.settings.insert("hotkey".into(), "Ctrl+Shift+K".into());
            })
            .unwrap();
        let settings = Settings::of(&store);
        assert!(!settings.lock_when_hidden());
        assert!(settings.lock_on_session_lock());
        assert_eq!(settings.hotkey(), "Ctrl+Shift+K");
    }

    #[test]
    fn view_shows_defaults_until_changed() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::load(dir.path().join("pswm.json"));
        let settings = Settings::of(&store);
        let view = settings.view();
        assert_eq!((view.lock_after_minutes, view.clear_clipboard, view.sync_every_minutes), (5, 20, 5));
        assert!(view.lock_on_session_lock && !view.lock_when_hidden && view.download_icons);
        assert_eq!(view.hotkey, "Ctrl+Alt+P");

        settings.set("lockAfterMinutes", 0.into()).unwrap();
        settings.set("clearClipboard", 60.into()).unwrap();
        settings.set("downloadIcons", false.into()).unwrap();
        let view = Settings::of(&Store::load(dir.path().join("pswm.json"))).view();
        assert_eq!((view.lock_after_minutes, view.clear_clipboard), (0, 60));
        assert!(!view.download_icons);
    }

    #[test]
    fn set_refuses_unknown_names_and_bad_values() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::load(dir.path().join("pswm.json"));
        let settings = Settings::of(&store);
        assert!(settings.set("hotkey", "".into()).is_err());
        assert!(settings.set("database", "x".into()).is_err());
        assert!(settings.set("clearClipboard", 0.into()).is_err());
        assert!(settings.set("clearClipboard", 121.into()).is_err());
        assert!(settings.set("lockAfterMinutes", 61.into()).is_err());
        assert!(settings.set("lockAfterMinutes", "5".into()).is_err());
        assert!(settings.set("lockWhenHidden", 1.into()).is_err());
        assert!(store.read(|s| s.settings.is_empty()));
        settings.set("syncEveryMinutes", 0.into()).unwrap();
        assert_eq!(settings.sync_every(), None);
        assert!(settings.set("theme", "blue".into()).is_err());
        assert_eq!(settings.theme(), "system");
        settings.set("theme", "dark".into()).unwrap();
        assert_eq!(settings.theme(), "dark");
    }

    #[test]
    fn the_phone_locks_in_the_background_after_a_choice_or_never() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::load(dir.path().join("pswm.json"));
        let settings = Settings::of(&store);
        assert_eq!(settings.lock_in_background(), Some(Duration::from_secs(30)));
        assert!(settings.lock_on_screen_off());
        settings.set("lockInBackground", 0.into()).unwrap();
        assert_eq!(settings.lock_in_background(), Some(Duration::ZERO));
        settings.set("lockInBackground", Value::Null).unwrap();
        assert_eq!(settings.lock_in_background(), None);
        assert!(settings.set("lockInBackground", 45.into()).is_err());
        settings.set("lockOnScreenOff", false.into()).unwrap();
        assert!(!settings.view().lock_on_screen_off);
    }
}
