//! Locking after inactivity: the window reports use, a background check locks
//! once the database has been left alone for the configured time.

use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const DEFAULT_MINUTES: u64 = 5;
const MINUTES_RANGE: (u64, u64) = (1, 60);
/// How often the inactivity check runs.
pub const CHECK_EVERY: Duration = Duration::from_secs(10);

/// When the user last did something in the app.
pub struct Activity(Mutex<Instant>);

impl Default for Activity {
    fn default() -> Self {
        Activity(Mutex::new(Instant::now()))
    }
}

impl Activity {
    pub fn touch(&self) {
        *self.0.lock().unwrap() = Instant::now();
    }

    pub fn idle_for(&self) -> Duration {
        self.0.lock().unwrap().elapsed()
    }
}

/// The lock timeout for the `lockAfterMinutes` setting: 0 means never, other
/// values are kept within 1–60 minutes.
pub fn timeout(minutes: Option<u64>) -> Option<Duration> {
    let (min, max) = MINUTES_RANGE;
    match minutes.unwrap_or(DEFAULT_MINUTES) {
        0 => None,
        m => Some(Duration::from_secs(m.clamp(min, max) * 60)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_follows_the_setting_within_limits() {
        assert_eq!(timeout(None), Some(Duration::from_secs(5 * 60)));
        assert_eq!(timeout(Some(0)), None);
        assert_eq!(timeout(Some(15)), Some(Duration::from_secs(15 * 60)));
        assert_eq!(timeout(Some(1000)), Some(Duration::from_secs(60 * 60)));
    }

    #[test]
    fn touching_resets_the_idle_time() {
        let activity = Activity(Mutex::new(Instant::now() - Duration::from_secs(600)));
        assert!(activity.idle_for() >= Duration::from_secs(600));
        activity.touch();
        assert!(activity.idle_for() < Duration::from_secs(1));
    }
}
