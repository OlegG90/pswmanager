//! Locking after inactivity: the window reports use, a background check locks
//! once the database has been left alone for the configured time.

use std::sync::Mutex;
use std::time::{Duration, Instant};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touching_resets_the_idle_time() {
        let activity = Activity(Mutex::new(Instant::now() - Duration::from_secs(600)));
        assert!(activity.idle_for() >= Duration::from_secs(600));
        activity.touch();
        assert!(activity.idle_for() < Duration::from_secs(1));
    }
}
