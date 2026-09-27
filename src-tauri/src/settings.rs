//! The user's settings, kept by the frontend in the state file: every name,
//! default and allowed range in one place.

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

    pub fn clear_clipboard_after(&self) -> Duration {
        clear_after(self.get("clearClipboard", Value::as_u64))
    }

    pub fn sync_every(&self) -> Option<Duration> {
        minutes(self.get("syncEveryMinutes", Value::as_u64), SYNC_EVERY_MINUTES)
    }

    pub fn download_icons(&self) -> bool {
        self.get("downloadIcons", Value::as_bool).unwrap_or(true)
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
            hotkey: self.hotkey(),
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
/// registry instead, and the hotkey is only shown.
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
    pub hotkey: String,
}

fn check(name: &str, value: &Value) -> Result<(), String> {
    let within = |(_, min, max): (u64, u64, u64), never: bool| {
        value.as_u64().is_some_and(|v| (never && v == 0) || (min..=max).contains(&v))
    };
    let ok = match name {
        "lockAfterMinutes" => within(LOCK_AFTER_MINUTES, true),
        "syncEveryMinutes" => within(SYNC_EVERY_MINUTES, true),
        "clearClipboard" => within(CLEAR_SECONDS, false),
        "lockOnSessionLock" | "lockWhenHidden" | "downloadIcons" => value.is_boolean(),
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
        assert!(settings.set("hotkey", "Ctrl+K".into()).is_err());
        assert!(settings.set("database", "x".into()).is_err());
        assert!(settings.set("clearClipboard", 0.into()).is_err());
        assert!(settings.set("clearClipboard", 121.into()).is_err());
        assert!(settings.set("lockAfterMinutes", 61.into()).is_err());
        assert!(settings.set("lockAfterMinutes", "5".into()).is_err());
        assert!(settings.set("lockWhenHidden", 1.into()).is_err());
        assert!(store.read(|s| s.settings.is_empty()));
        settings.set("syncEveryMinutes", 0.into()).unwrap();
        assert_eq!(settings.sync_every(), None);
    }
}
