//! The user's settings, kept by the frontend in the state file: every name,
//! default and allowed range in one place.

use crate::store::Store;
use serde_json::Value;
use std::time::Duration;

const HOTKEY: &str = "Ctrl+Alt+P";
/// Minutes without use before locking; 0 means never.
const LOCK_AFTER_MINUTES: (u64, u64, u64) = (5, 1, 60);
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

    pub fn download_icons(&self) -> bool {
        self.get("downloadIcons", Value::as_bool).unwrap_or(true)
    }
}

fn lock_after(minutes: Option<u64>) -> Option<Duration> {
    let (default, min, max) = LOCK_AFTER_MINUTES;
    match minutes.unwrap_or(default) {
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
}
